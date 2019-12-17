use std::fs;
use std::io::{self, Error, ErrorKind, Write};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use crate::{NIX_VERSION_MAGIC, PAD_LEN};

pub fn to_vec<P: AsRef<Path>>(path: P) -> io::Result<Vec<u8>> {
    let mut buffer = Vec::new();
    to_writer(&mut buffer, path)?;
    Ok(buffer)
}

pub fn to_writer<W, P>(writer: &mut W, path: P) -> io::Result<()>
where
    W: Write,
    P: AsRef<Path>,
{
    let target = path.as_ref();
    if !target.exists() {
        return Err(Error::new(ErrorKind::NotFound, "Path not found"));
    }

    write_padded(writer, NIX_VERSION_MAGIC)?;
    encode_entry(writer, target)
}

fn encode_entry<W: Write>(writer: &mut W, path: &Path) -> io::Result<()> {
    write_padded(writer, b"(")?;
    write_padded(writer, b"type")?;

    let metadata = fs::symlink_metadata(&path)?;
    if metadata.file_type().is_dir() {
        write_padded(writer, b"directory")?;

        let mut entries: Vec<_> = fs::read_dir(path)?.collect::<Result<_, _>>()?;
        entries.sort_by(|x, y| x.path().cmp(&y.path()));

        for entry in entries {
            write_padded(writer, b"entry")?;
            write_padded(writer, b"(")?;
            write_padded(writer, b"name")?;
            write_padded(writer, entry.file_name().to_string_lossy().as_bytes())?;
            write_padded(writer, b"node")?;
            encode_entry(writer, &entry.path())?;
            write_padded(writer, b")")?;
        }
    } else if metadata.file_type().is_file() {
        write_padded(writer, b"regular")?;

        if metadata.mode() & 0o111 != 0 {
            write_padded(writer, b"executable")?;
            write_padded(writer, b"")?;
        }

        write_padded(writer, b"contents")?;
        let file = fs::read(path)?;
        write_padded(writer, &file)?;
    } else if metadata.file_type().is_symlink() {
        write_padded(writer, b"symlink")?;
        write_padded(writer, b"target")?;
        let target = fs::read_link(path)?;
        write_padded(writer, target.to_string_lossy().as_bytes())?;
    } else {
        return Err(Error::new(ErrorKind::InvalidData, "Unrecognized file type"));
    }

    write_padded(writer, b")")?;

    Ok(())
}

fn write_padded<W: Write>(writer: &mut W, bytes: &[u8]) -> io::Result<()> {
    let len = bytes.len() as u64;
    writer.write_all(&len.to_le_bytes())?;
    writer.write_all(bytes)?;

    let remainder = bytes.len() % PAD_LEN;
    if remainder > 0 {
        let buf = [0u8; PAD_LEN];
        let padding = PAD_LEN - remainder;
        writer.write_all(&buf[..padding])?;
    }

    Ok(())
}
