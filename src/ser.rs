use std::fs::{self, File};
use std::io::{self, Error, ErrorKind, Read, Write};
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
    if !fs::symlink_metadata(target).is_ok() {
        return Err(Error::new(ErrorKind::NotFound, "Path not found"));
    }

    write_padded(writer, NIX_VERSION_MAGIC)?;
    encode_entry(writer, target)
}

fn encode_entry<W: Write>(writer: &mut W, path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;

    write_padded(writer, b"(")?;
    write_padded(writer, b"type")?;

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
        let mut file = File::open(path)?;
        write_padded_from_reader(writer, &mut file, metadata.len())?;
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

fn write_padded_from_reader<W, R>(writer: &mut W, reader: &mut R, len: u64) -> io::Result<()>
where
    W: Write,
    R: Read,
{
    writer.write_all(&len.to_le_bytes())?;
    io::copy(reader, writer)?;

    let remainder = (len % PAD_LEN as u64) as usize;
    if remainder > 0 {
        let buf = [0u8; PAD_LEN];
        let padding = PAD_LEN - remainder;
        writer.write_all(&buf[..padding])?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;

    use super::*;

    #[test]
    fn writes_multiple_of_eight_exactly() {
        let mut buffer = Vec::new();
        let length = 16u64;
        let data = vec![1u8; length as usize];
        write_padded(&mut buffer, &data[..]).unwrap();

        let written_data_len = size_of::<u64>() as u64 + length;
        assert_eq!(buffer.len() as u64, written_data_len);

        let header_bytes = length.to_le_bytes();
        assert_eq!(&buffer[..size_of::<u64>()], header_bytes);

        let data_bytes = [1u8; 16];
        assert_eq!(&buffer[size_of::<u64>()..], data_bytes);
    }

    #[test]
    fn pads_non_multiple_of_eight() {
        let mut buffer = Vec::new();
        let length = 5u64;
        let data = vec![1u8; length as usize];
        write_padded(&mut buffer, &data[..]).unwrap();

        let written_data_len = size_of::<u64>() as u64 + length + 3;
        assert_eq!(buffer.len() as u64, written_data_len);

        let header_bytes = length.to_le_bytes();
        assert_eq!(&buffer[..size_of::<u64>()], header_bytes);

        let data_bytes = [1u8; 5];
        assert_eq!(&buffer[size_of::<u64>()..size_of::<u64>() + 5], data_bytes);

        let padding_bytes = [0u8; 3];
        assert_eq!(&buffer[size_of::<u64>() + 5..], padding_bytes);
    }
}
