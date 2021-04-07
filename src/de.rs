use std::fmt::{self, Debug, Formatter};
use std::fs::{self, OpenOptions};
use std::future::Future;
use std::io::{self, ErrorKind, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;

use filetime::FileTime;
use genawaiter::sync::Gen;

use crate::{NIX_VERSION_MAGIC, PAD_LEN};

#[derive(Debug)]
struct ArchiveInner<R: ?Sized> {
    position: u64,
    reader: R,
}

impl<R: ?Sized + Read> Read for ArchiveInner<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let bytes_read = self.reader.read(buf)?;
        self.position += bytes_read as u64;
        Ok(bytes_read)
    }
}

impl<R: ?Sized + Read> ArchiveInner<R> {
    fn read_utf8_padded(&mut self) -> Result<String> {
        let bytes = self.read_bytes_padded()?;
        Ok(String::from_utf8(bytes)?)
    }

    fn read_bytes_padded(&mut self) -> Result<Vec<u8>> {
        let mut len_buffer = [0u8; PAD_LEN];
        self.read_exact(&mut len_buffer[..])?;
        let len = u64::from_le_bytes(len_buffer);

        let mut data_buffer = vec![0u8; len as usize];
        self.read_exact(&mut data_buffer)?;

        let remainder = data_buffer.len() % PAD_LEN;
        if remainder > 0 {
            let mut buffer = [0u8; PAD_LEN];
            let padding = &mut buffer[0..PAD_LEN - remainder];
            self.read_exact(padding)?;
            if !buffer.iter().all(|b| *b == 0) {
                return Err(Error::BadPadding);
            }
        }

        Ok(data_buffer)
    }

    fn expect_tag(&mut self, tag: Tag) -> Result<()> {
        if self.read_utf8_padded()? == tag.into_str() {
            Ok(())
        } else {
            Err(Error::MissingTag(tag))
        }
    }
}

#[derive(Debug)]
pub struct UnknownTag;

macro_rules! define_tags {
    ($($sym:ident => $str:expr),* $(,)?) => {
        #[derive(Copy, Clone, Debug)]
        #[non_exhaustive]
        pub enum Tag {
            $($sym),*
        }

        impl Tag {
            pub fn into_str(self) -> &'static str {
                match self {
                    $(Tag::$sym => $str,)*
                }
            }
        }

        impl std::str::FromStr for Tag {
            type Err = UnknownTag;

            fn from_str(s: &str) -> std::result::Result<Tag, UnknownTag> {
                Ok(match s {
                    $($str => Tag::$sym,)*
                    _ => return Err(UnknownTag),
                })
            }
        }
    }
}

define_tags! {
    Empty => "",
    Open => "(",
    Close => ")",
    Type => "type",

    Regular => "regular",
    Symlink => "symlink",
    Directory => "directory",
    Entry => "entry",

    Contents => "contents",
    Executable => "executable",
    Target => "target",
    Name => "name",
    Node => "node",
}

impl fmt::Display for Tag {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.into_str())
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    InvalidMagic,
    BadPadding,
    MissingTag(Tag),
    InvalidTag(Tag),
    InvalidDirEntryName(&'static str),
    InvalidDirEntryChar(char),
    InvalidDirEntry,
    UnknownFileType(String),

    InvalidPathComponent { path: PathBuf },
    Io(io::Error),
    IoAt { inner: io::Error, path: PathBuf },
    Utf8(std::string::FromUtf8Error),
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::IoAt { inner, .. } => Some(inner),
            Error::Utf8(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    #[inline]
    fn from(x: io::Error) -> Error {
        Error::Io(x)
    }
}

impl From<std::string::FromUtf8Error> for Error {
    #[inline]
    fn from(x: std::string::FromUtf8Error) -> Error {
        Error::Utf8(x)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        use Error as E;
        match self {
            E::InvalidMagic => write!(f, "Not a valid NAR archive"),
            E::BadPadding => write!(f, "Bad archive padding"),
            E::MissingTag(t) => write!(f, "Missing `{}` tag", t),
            E::InvalidTag(t) => write!(f, "Invalid `{}` tag", t),

            E::InvalidDirEntryName("") => write!(f, "Entry name is empty"),
            E::InvalidDirEntryName(n) => write!(f, "Invalid name `{}`", n),
            E::InvalidDirEntryChar(c) => write!(f, "Invalid character in entry name: `{}`", c),
            E::InvalidDirEntry => write!(f, "Invalid directory entry"),

            E::UnknownFileType(ft) => write!(f, "Unrecognized file type `{}`", ft),

            E::InvalidPathComponent { path } => {
                write!(f, "Invalid path component in {}", path.display())
            }
            E::Io(e) => write!(f, "I/O error: {}", e),
            E::IoAt { inner, path } => write!(
                f,
                "I/O error: {}; while handling: {}",
                inner,
                path.display()
            ),
            E::Utf8(e) => write!(f, "Utf8 error: {}", e),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
type Co = genawaiter::sync::Co<Result<Entry>>;

#[derive(Copy, Clone, Debug)]
pub struct Parameters {
    pub canonicalize_mtime: bool,
    pub remove_xattrs: bool,
}

impl Default for Parameters {
    #[inline]
    fn default() -> Self {
        Self {
            canonicalize_mtime: true,
            remove_xattrs: true,
        }
    }
}

impl Parameters {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    async fn parse_all<'a, R: Read + 'a>(self, mut co: Co, mut reader: ArchiveInner<R>) {
        if let Err(err) = try_parse(&mut co, self, &mut reader, PathBuf::new()).await {
            co.yield_(Err(err)).await;
        }
    }

    pub fn entries<'a, R: Read + 'a>(
        self,
        reader: R,
    ) -> Result<impl Iterator<Item = Result<Entry>> + 'a> {
        let mut reader = ArchiveInner {
            position: 0,
            reader,
        };
        if ((&mut reader) as &mut ArchiveInner<dyn Read + 'a>).read_bytes_padded()?
            != NIX_VERSION_MAGIC
        {
            Err(Error::InvalidMagic)
        } else {
            Ok(Gen::new(move |co| self.parse_all(co, reader)).into_iter())
        }
    }

    pub fn unpack<R: Read, P: AsRef<Path>>(self, reader: R, dst: P) -> Result<()> {
        let dst = dst.as_ref();
        for entry in self.entries(reader)? {
            let mut file = entry?;
            file.unpack_in(dst)?;
        }
        Ok(())
    }
}

#[derive(Default)]
struct LookAhead(Option<String>);

impl LookAhead {
    pub fn fetch_from(&mut self, archive: &mut ArchiveInner<dyn Read + '_>) -> Result<()> {
        assert_eq!(self.0, None);
        self.0 = Some(archive.read_utf8_padded()?);
        Ok(())
    }

    pub fn expect_tag(&mut self, tag: Tag) -> Result<()> {
        if self.eat_tag(tag) {
            Ok(())
        } else {
            Err(Error::MissingTag(tag))
        }
    }

    pub fn eat_tag(&mut self, tag: Tag) -> bool {
        if let Some(x) = self.0.take() {
            if x == tag.into_str() {
                return true;
            }
            self.0 = Some(x);
        }
        false
    }
}

async fn try_parse(
    co: &mut Co,
    params: Parameters,
    mut archive: &mut ArchiveInner<dyn Read + '_>,
    path: PathBuf,
) -> Result<()> {
    archive.expect_tag(Tag::Open)?;
    archive.expect_tag(Tag::Type)?;

    let ft = archive.read_utf8_padded()?;
    match ft.as_str() {
        "regular" => {
            let mut executable = false;
            let mut la: LookAhead = Default::default();
            la.fetch_from(&mut archive)?;

            if la.eat_tag(Tag::Executable) {
                executable = true;
                if archive.expect_tag(Tag::Empty).is_err() {
                    return Err(Error::InvalidTag(Tag::Executable));
                }
                la.fetch_from(&mut archive)?;
            }

            la.expect_tag(Tag::Contents)?;
            let data = archive.read_bytes_padded()?;

            archive.expect_tag(Tag::Close)?;

            co.yield_(Ok(Entry {
                path,
                kind: EntryKind::Regular { executable, data },
                params,
            }))
            .await;
        }
        "symlink" => {
            archive.expect_tag(Tag::Target)?;
            let target: PathBuf = archive.read_utf8_padded()?.into();
            archive.expect_tag(Tag::Close)?;

            co.yield_(Ok(Entry {
                path,
                kind: EntryKind::Symlink { target },
                params,
            }))
            .await;
        }
        "directory" => {
            co.yield_(Ok(Entry {
                path: path.clone(),
                kind: EntryKind::Directory,
                params,
            }))
            .await;

            loop {
                match archive.read_utf8_padded()?.as_str() {
                    "entry" => {
                        archive.expect_tag(Tag::Open)?;
                        archive.expect_tag(Tag::Name)?;

                        let entry_name = archive.read_utf8_padded()?;
                        match entry_name.as_str() {
                            "" => return Err(Error::InvalidDirEntryName("")),
                            "~" => return Err(Error::InvalidDirEntryName("~")),
                            "." => return Err(Error::InvalidDirEntryName(".")),
                            ".." => return Err(Error::InvalidDirEntryName("..")),
                            _ if entry_name.contains('/') => {
                                return Err(Error::InvalidDirEntryChar('/'))
                            }
                            _ => {}
                        };

                        archive.expect_tag(Tag::Node)?;

                        let child_entry: Pin<Box<dyn Future<Output = _>>> =
                            Box::pin(try_parse(co, params, archive, path.join(entry_name)));
                        child_entry.await?;

                        archive.expect_tag(Tag::Close)?;
                    }
                    ")" => break,
                    _ => return Err(Error::InvalidDirEntry),
                }
            }
        }
        _ => return Err(Error::UnknownFileType(ft)),
    }

    Ok(())
}

#[derive(Debug)]
pub struct Entry {
    path: PathBuf,
    kind: EntryKind,
    pub params: Parameters,
}

impl Entry {
    #[inline]
    pub fn name(&self) -> &Path {
        &self.path
    }

    #[inline]
    pub fn is_dir(&self) -> bool {
        match &self.kind {
            EntryKind::Directory => true,
            _ => false,
        }
    }

    #[inline]
    pub fn is_executable(&self) -> bool {
        match &self.kind {
            EntryKind::Regular { executable, .. } => *executable,
            _ => false,
        }
    }

    #[inline]
    pub fn is_file(&self) -> bool {
        match &self.kind {
            EntryKind::Regular { executable, .. } => !executable,
            _ => false,
        }
    }

    #[inline]
    pub fn is_symlink(&self) -> bool {
        match &self.kind {
            EntryKind::Symlink { .. } => true,
            _ => false,
        }
    }

    pub fn unpack_in<P: AsRef<Path>>(&mut self, dst: P) -> Result<()> {
        let dst = dst.as_ref();
        let path = if self.path.as_os_str().is_empty() {
            dst.to_owned()
        } else {
            dst.join(&self.path)
        };

        for component in path.components() {
            if matches!(
                component,
                Component::Prefix(_) | Component::RootDir | Component::ParentDir
            ) {
                return Err(Error::InvalidPathComponent { path });
            }
        }

        // If the timestamp of our parent has been canonicalized, we want to keep it that way after
        // we unpack, whether we choose to canonicalize as well or not.
        let recanonicalize_parent = path
            .parent()
            .filter(|_| !self.path.as_os_str().is_empty())
            .and_then(|p| fs::symlink_metadata(p).ok())
            .filter(|m| {
                FileTime::from_creation_time(&m)
                    .filter(|time| *time == FileTime::zero())
                    .is_some()
            });

        match &mut self.kind {
            EntryKind::Directory => Self::unpack_dir(&path),
            EntryKind::Regular { executable, data } => Self::unpack_file(&path, *executable, data),
            EntryKind::Symlink { target } => Self::unpack_symlink(&path, target),
        }
        .map_err(|inner| Error::IoAt {
            inner,
            path: path.clone(),
        })?;

        if self.params.remove_xattrs {
            #[cfg(all(unix, feature = "xattr"))]
            for attr in xattr::list(&path)? {
                xattr::remove(&path, attr)?;
            }
        }

        if self.params.canonicalize_mtime {
            let metadata = fs::symlink_metadata(&path)?;
            let atime = FileTime::from_last_access_time(&metadata);
            filetime::set_symlink_file_times(&path, atime, FileTime::zero())?;
        }

        if let Some(metadata) = recanonicalize_parent {
            if let Some(parent) = path.parent() {
                let atime = FileTime::from_last_access_time(&metadata);
                filetime::set_symlink_file_times(&parent, atime, FileTime::zero())?;
            }
        }

        Ok(())
    }

    fn unpack_dir(dst: &Path) -> io::Result<()> {
        fs::create_dir(&dst).or_else(|err| {
            if err.kind() == ErrorKind::AlreadyExists {
                let prev = fs::metadata(&dst);
                if prev.map(|m| m.is_dir()).unwrap_or(false) {
                    return Ok(());
                }
            }
            Err(err)
        })
    }

    fn unpack_file(dst: &Path, executable: bool, data: &[u8]) -> io::Result<()> {
        if dst.exists() {
            fs::remove_file(&dst)?;
        }

        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(if executable { 0o555 } else { 0o444 })
            .open(&dst)?;

        file.write_all(data)?;
        Ok(())
    }

    fn unpack_symlink(dst: &Path, target: &Path) -> io::Result<()> {
        if fs::symlink_metadata(&dst).is_ok() {
            fs::remove_file(&dst)?;
        }

        std::os::unix::fs::symlink(target, dst)
    }
}

enum EntryKind {
    Directory,
    Regular { executable: bool, data: Vec<u8> },
    Symlink { target: PathBuf },
}

impl From<EntryKind> for Tag {
    fn from(ek: EntryKind) -> Tag {
        match ek {
            EntryKind::Directory => Tag::Directory,
            EntryKind::Regular { executable, .. } => {
                if executable {
                    Tag::Executable
                } else {
                    Tag::Regular
                }
            }
            EntryKind::Symlink { .. } => Tag::Symlink,
        }
    }
}

impl Debug for EntryKind {
    fn fmt(&self, fmt: &mut Formatter) -> fmt::Result {
        use EntryKind::*;
        match self {
            Directory => fmt.debug_struct(stringify!(Directory)).finish(),
            Regular { executable, .. } => fmt
                .debug_struct(stringify!(Regular))
                .field("executable", executable)
                .finish(),
            Symlink { target } => fmt
                .debug_struct(stringify!(Symlink))
                .field("target", target)
                .finish(),
        }
    }
}
