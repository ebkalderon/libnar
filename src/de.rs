use std::cell::{Cell, RefCell};
use std::fmt::{self, Debug, Formatter};
use std::fs::{self, OpenOptions};
use std::future::Future;
use std::io::{self, Cursor, Error, ErrorKind, Read};
use std::marker::PhantomData;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;

use filetime::FileTime;
use genawaiter::sync::Gen;

use crate::{NIX_VERSION_MAGIC, PAD_LEN};

type Co<'a> = genawaiter::sync::Co<io::Result<Entry<'a>>>;

#[derive(Debug)]
struct ArchiveInner<R: ?Sized> {
    canonicalize_mtime: bool,
    remove_xattrs: bool,
    position: Cell<u64>,
    reader: RefCell<R>,
}

impl<'a, R: ?Sized + Read> Read for &'a ArchiveInner<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let bytes_read = self.reader.borrow_mut().read(buf)?;
        self.position.set(self.position.get() + bytes_read as u64);
        Ok(bytes_read)
    }
}

pub struct Archive<R: ?Sized + Read> {
    inner: ArchiveInner<R>,
}

impl<R: Read> Archive<R> {
    pub fn new(reader: R) -> Self {
        Archive {
            inner: ArchiveInner {
                canonicalize_mtime: true,
                remove_xattrs: true,
                position: Cell::new(0),
                reader: RefCell::new(reader),
            },
        }
    }

    pub fn into_inner(self) -> R {
        self.inner.reader.into_inner()
    }

    pub fn entries(&mut self) -> io::Result<Entries<R>> {
        let archive: &mut Archive<dyn Read> = self;
        archive.entries_inner().map(|iter| Entries {
            iter,
            _marker: PhantomData
        })
    }

    pub fn set_canonicalize_mtime(&mut self, canonicalize: bool) {
        self.inner.canonicalize_mtime = canonicalize;
    }

    pub fn set_remove_xattrs(&mut self, remove: bool) {
        self.inner.remove_xattrs = remove;
    }

    pub fn unpack<P: AsRef<Path>>(&mut self, dst: P) -> io::Result<()> {
        let archive: &mut Archive<dyn Read> = self;
        archive.unpack_inner(dst.as_ref())
    }
}

impl<'a> Archive<dyn Read + 'a> {
    fn entries_inner(&mut self) -> io::Result<Box<dyn Iterator<Item = io::Result<Entry>> + '_>> {
        if self.inner.position.get() != 0 {
            let message = "Cannot call `entries` unless reader is in position 0";
            return Err(Error::new(ErrorKind::Other, message));
        }

        if self.read_bytes_padded()? != NIX_VERSION_MAGIC {
            return Err(Error::new(ErrorKind::Other, "Not a valid NAR archive"));
        }

        let gen = Gen::new(move |co| parse(co, self));
        Ok(Box::new(gen.into_iter()))
    }

    fn unpack_inner(&mut self, dst: &Path) -> io::Result<()> {
        for entry in self.entries_inner()? {
            let mut file = entry?;
            file.unpack_in(dst)?;
        }
        Ok(())
    }

    fn read_utf8_padded(&self) -> io::Result<String> {
        let bytes = self.read_bytes_padded()?;
        String::from_utf8(bytes).map_err(|e| Error::new(ErrorKind::InvalidData, e))
    }

    fn read_bytes_padded(&self) -> io::Result<Vec<u8>> {
        let mut len_buffer = [0u8; PAD_LEN];
        (&self.inner).read_exact(&mut len_buffer[..])?;
        let len = u64::from_le_bytes(len_buffer);

        let mut data_buffer = vec![0u8; len as usize];
        (&self.inner).read_exact(&mut data_buffer)?;

        let remainder = data_buffer.len() % PAD_LEN;
        if remainder > 0 {
            let mut buffer = [0u8; PAD_LEN];
            let padding = &mut buffer[0..PAD_LEN - remainder];
            (&self.inner).read_exact(padding)?;
            if !buffer.iter().all(|b| *b == 0) {
                return Err(Error::new(ErrorKind::Other, "Bad archive padding"));
            }
        }

        Ok(data_buffer)
    }
}

impl<'a, R: Read> Debug for Archive<R> {
    fn fmt(&self, fmt: &mut Formatter) -> fmt::Result {
        fmt.debug_struct(stringify!(Archive))
            .field("canonicalize_mtime", &self.inner.canonicalize_mtime)
            .field("remove_xattrs", &self.inner.remove_xattrs)
            .field("position", &self.inner.position)
            .finish()
    }
}

async fn parse(mut co: Co<'_>, archive: &Archive<dyn Read + '_>) {
    if let Err(err) = try_parse(&mut co, archive, PathBuf::new()).await {
        co.yield_(Err(err)).await;
    }
}

async fn try_parse(
    co: &mut Co<'_>,
    archive: &Archive<dyn Read + '_>,
    path: PathBuf,
) -> io::Result<()> {
    if archive.read_utf8_padded()? != "(" {
        return Err(Error::new(ErrorKind::Other, "Missing open tag"));
    }

    if archive.read_utf8_padded()? != "type" {
        return Err(Error::new(ErrorKind::Other, "Missing type tag"));
    }

    match archive.read_utf8_padded()?.as_str() {
        "regular" => {
            let mut executable = false;
            let mut tag = archive.read_utf8_padded()?;

            if tag == "executable" {
                executable = true;
                if archive.read_utf8_padded()? != "" {
                    return Err(Error::new(ErrorKind::Other, "Incorrect executable tag"));
                }
                tag = archive.read_utf8_padded()?;
            }

            let data = if tag == "contents" {
                archive.read_bytes_padded()?
            } else {
                return Err(Error::new(ErrorKind::Other, "Missing contents tag"));
            };

            if archive.read_utf8_padded()? != ")" {
                return Err(Error::new(ErrorKind::Other, "Missing regular close tag"));
            }

            co.yield_(Ok(Entry::new(
                path,
                EntryKind::Regular { executable, data },
                archive,
            )))
            .await;
        }
        "symlink" => {
            let target = if archive.read_utf8_padded()? == "target" {
                archive.read_utf8_padded().map(PathBuf::from)?
            } else {
                return Err(Error::new(ErrorKind::Other, "Missing target tag"));
            };

            if archive.read_utf8_padded()? != ")" {
                return Err(Error::new(ErrorKind::Other, "Missing symlink close tag"));
            }

            co.yield_(Ok(Entry::new(path, EntryKind::Symlink { target }, archive)))
                .await;
        }
        "directory" => {
            co.yield_(Ok(Entry::new(path.clone(), EntryKind::Directory, archive)))
                .await;

            loop {
                match archive.read_utf8_padded()?.as_str() {
                    "entry" => {
                        if archive.read_utf8_padded()? != "(" {
                            return Err(Error::new(ErrorKind::Other, "Missing open tag"));
                        }

                        let entry_path = if archive.read_utf8_padded()? == "name" {
                            archive.read_utf8_padded().map(PathBuf::from)?
                        } else {
                            return Err(Error::new(ErrorKind::Other, "Missing name field"));
                        };

                        if archive.read_utf8_padded()? != "node" {
                            return Err(Error::new(ErrorKind::Other, "Missing node field"));
                        }

                        let recurse: Pin<Box<dyn Future<Output = _>>> =
                            Box::pin(try_parse(co, archive, path.join(entry_path)));
                        recurse.await?;

                        if archive.read_utf8_padded()? != ")" {
                            return Err(Error::new(ErrorKind::Other, "Missing nested close tag"));
                        }
                    }
                    ")" => break,
                    _ => return Err(Error::new(ErrorKind::Other, "Incorrect directory field")),
                }
            }
        }
        _ => return Err(Error::new(ErrorKind::Other, "Unrecognized file type")),
    }

    Ok(())
}

pub struct Entries<'a, R: 'a + Read> {
    iter: Box<dyn Iterator<Item = io::Result<Entry<'a>>> + 'a>,
    _marker: PhantomData<&'a Archive<R>>,
}

impl<'a, R: Read> Iterator for Entries<'a, R> {
    type Item = io::Result<Entry<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next()
    }
}

impl<'a, R: Read> Debug for Entries<'a, R> {
    fn fmt(&self, fmt: &mut Formatter) -> fmt::Result {
        write!(fmt, stringify!(Entries))
    }
}

pub struct Entry<'a> {
    name: PathBuf,
    kind: EntryKind,
    canonicalize_mtime: bool,
    remove_xattrs: bool,
    _marker: PhantomData<&'a ()>,
}

impl<'a> Entry<'a> {
    fn new(name: PathBuf, kind: EntryKind, archive: &Archive<dyn Read + '_>) -> Self {
        Entry {
            name,
            kind,
            canonicalize_mtime: archive.inner.canonicalize_mtime,
            remove_xattrs: archive.inner.remove_xattrs,
            _marker: PhantomData,
        }
    }

    #[inline]
    pub fn name(&self) -> &Path {
        &self.name
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

    pub fn set_canonicalize_mtime(&mut self, canonicalize: bool) {
        self.canonicalize_mtime = canonicalize;
    }

    pub fn set_remove_xattrs(&mut self, remove: bool) {
        self.remove_xattrs = remove;
    }

    pub fn unpack_in<P: AsRef<Path>>(&mut self, dst: P) -> io::Result<()> {
        let path = if self.name.as_os_str().len() == 0 {
            dst.as_ref().to_owned()
        } else {
            dst.as_ref().join(&self.name)
        };

        for component in path.components() {
            if let Component::Prefix(_) | Component::RootDir | Component::ParentDir = component {
                let message = format!("Invalid path component in {:?}", path);
                return Err(Error::new(ErrorKind::Other, message));
            }
        }

        // If the timestamp of our parent has been canonicalized, we want to keep it that way after
        // we unpack, whether we choose to canonicalize as well or not.
        let recanonicalize_parent = path
            .parent()
            .filter(|_| !self.name.as_os_str().is_empty())
            .and_then(|p| fs::symlink_metadata(p).ok())
            .filter(|m| {
                FileTime::from_creation_time(&m)
                    .filter(|time| *time == FileTime::zero())
                    .is_some()
            });

        match &mut self.kind {
            EntryKind::Directory => Self::unpack_dir(&path)?,
            EntryKind::Regular { executable, data } => Self::unpack_file(&path, *executable, data)?,
            EntryKind::Symlink { target } => Self::unpack_symlink(&path, target)?,
        }

        if self.remove_xattrs {
            #[cfg(all(unix, feature = "xattr"))]
            for attr in xattr::list(&path)? {
                xattr::remove(&path, attr)?;
            }
        }

        if self.canonicalize_mtime {
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
            Err(Error::new(
                err.kind(),
                format!("{} when creating dir {}", err, dst.display()),
            ))
        })
    }

    fn unpack_file(dst: &Path, executable: bool, data: &mut Vec<u8>) -> io::Result<()> {
        let mut opt = OpenOptions::new();
        opt.create(true).write(true);

        if executable {
            opt.mode(0o555);
        } else {
            opt.mode(0o444);
        }

        let mut file = opt.open(&dst)?;
        io::copy(&mut Cursor::new(data), &mut file)?;
        Ok(())
    }

    fn unpack_symlink(dst: &Path, target: &Path) -> io::Result<()> {
        if fs::symlink_metadata(&dst).is_ok() {
            fs::remove_file(&dst)?;
        }

        std::os::unix::fs::symlink(target, dst)
    }
}

impl<'a> Debug for Entry<'a> {
    fn fmt(&self, fmt: &mut Formatter) -> fmt::Result {
        fmt.debug_struct(stringify!(Entry))
            .field("name", &self.name)
            .field("kind", &self.kind)
            .finish()
    }
}

enum EntryKind {
    Directory,
    Regular { executable: bool, data: Vec<u8> },
    Symlink { target: PathBuf },
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
