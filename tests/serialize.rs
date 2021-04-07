use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;

#[test]
fn serializes_regular_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut file = File::create(dir.path().join("file.txt")).unwrap();
    writeln!(file, "lorem ipsum dolor sic amet").unwrap();

    let expected: Vec<u8> = std::iter::empty()
        .chain(
            13u64
                .to_le_bytes()
                .iter()
                .chain(b"nix-archive-1")
                .chain(&[0u8; 3]),
        )
        .chain(1u64.to_le_bytes().iter().chain(b"(").chain(&[0u8; 7]))
        .chain(4u64.to_le_bytes().iter().chain(b"type").chain(&[0u8; 4]))
        .chain(7u64.to_le_bytes().iter().chain(b"regular").chain(&[0u8; 1]))
        .chain(8u64.to_le_bytes().iter().chain(b"contents"))
        .chain(
            27u8.to_le_bytes()
                .iter()
                .chain(&[0u8; 7])
                .chain("lorem ipsum dolor sic amet\n".as_bytes())
                .chain(&[0u8; 5]),
        )
        .chain(1u64.to_le_bytes().iter().chain(b")").chain(&[0u8; 7]))
        .copied()
        .collect();

    let output = libnar::to_vec(dir.path().join("file.txt")).unwrap();
    assert_eq!(output, expected);
}

#[test]
fn serializes_executable_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .mode(0o777)
        .open(dir.path().join("script.sh"))
        .unwrap();

    write!(file, "#!/bin/sh\nset -euo pipefail\nexit 0\n").unwrap();

    let expected: Vec<u8> = std::iter::empty()
        .chain(
            13u64
                .to_le_bytes()
                .iter()
                .chain(b"nix-archive-1")
                .chain(&[0u8; 3]),
        )
        .chain(1u64.to_le_bytes().iter().chain(b"(").chain(&[0u8; 7]))
        .chain(4u64.to_le_bytes().iter().chain(b"type").chain(&[0u8; 4]))
        .chain(7u64.to_le_bytes().iter().chain(b"regular").chain(&[0u8; 1]))
        .chain(
            10u8.to_le_bytes()
                .iter()
                .chain(&[0u8; 7])
                .chain(b"executable")
                .chain(&[0u8; 5]),
        )
        .chain(0u8.to_le_bytes().iter().chain(b"").chain(&[0u8; 8]))
        .chain(8u64.to_le_bytes().iter().chain(b"contents"))
        .chain(
            35u8.to_le_bytes()
                .iter()
                .chain(&[0u8; 7])
                .chain("#!/bin/sh\nset -euo pipefail\nexit 0\n".as_bytes())
                .chain(&[0u8; 5]),
        )
        .chain(1u64.to_le_bytes().iter().chain(b")").chain(&[0u8; 7]))
        .copied()
        .collect();

    let output = libnar::to_vec(dir.path().join("script.sh")).unwrap();
    assert_eq!(output, expected);
}

#[test]
fn serializes_symlink() {
    let dir = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink("./foo", dir.path().join("foo")).unwrap();

    let expected: Vec<u8> = std::iter::empty()
        .chain(
            13u64
                .to_le_bytes()
                .iter()
                .chain(b"nix-archive-1")
                .chain(&[0u8; 3]),
        )
        .chain(1u64.to_le_bytes().iter().chain(b"(").chain(&[0u8; 7]))
        .chain(4u64.to_le_bytes().iter().chain(b"type").chain(&[0u8; 4]))
        .chain(7u64.to_le_bytes().iter().chain(b"symlink").chain(&[0u8; 1]))
        .chain(6u64.to_le_bytes().iter().chain(b"target").chain(&[0u8; 2]))
        .chain(5u64.to_le_bytes().iter().chain(b"./foo").chain(&[0u8; 3]))
        .chain(1u64.to_le_bytes().iter().chain(b")").chain(&[0u8; 7]))
        .copied()
        .collect();

    let output = libnar::to_vec(dir.path().join("foo")).unwrap();
    assert_eq!(output, expected);
}

#[test]
fn serializes_directory() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("subdir")).unwrap();
    fs::write(dir.path().join("subdir").join("file"), "hello world").unwrap();

    let expected: Vec<u8> = std::iter::empty()
        .chain(
            13u64
                .to_le_bytes()
                .iter()
                .chain(b"nix-archive-1")
                .chain(&[0u8; 3]),
        )
        .chain(1u64.to_le_bytes().iter().chain(b"(").chain(&[0u8; 7]))
        .chain(4u64.to_le_bytes().iter().chain(b"type").chain(&[0u8; 4]))
        .chain(
            9u64.to_le_bytes()
                .iter()
                .chain(b"directory")
                .chain(&[0u8; 7]),
        )
        .chain(5u64.to_le_bytes().iter().chain(b"entry").chain(&[0u8; 3]))
        .chain(1u64.to_le_bytes().iter().chain(b"(").chain(&[0u8; 7]))
        .chain(4u64.to_le_bytes().iter().chain(b"name").chain(&[0u8; 4]))
        .chain(6u64.to_le_bytes().iter().chain(b"subdir").chain(&[0u8; 2]))
        .chain(4u64.to_le_bytes().iter().chain(b"node").chain(&[0u8; 4]))
        .chain(1u64.to_le_bytes().iter().chain(b"(").chain(&[0u8; 7]))
        .chain(4u64.to_le_bytes().iter().chain(b"type").chain(&[0u8; 4]))
        .chain(
            9u64.to_le_bytes()
                .iter()
                .chain(b"directory")
                .chain(&[0u8; 7]),
        )
        .chain(5u64.to_le_bytes().iter().chain(b"entry").chain(&[0u8; 3]))
        .chain(1u64.to_le_bytes().iter().chain(b"(").chain(&[0u8; 7]))
        .chain(4u64.to_le_bytes().iter().chain(b"name").chain(&[0u8; 4]))
        .chain(4u64.to_le_bytes().iter().chain(b"file").chain(&[0u8; 4]))
        .chain(4u64.to_le_bytes().iter().chain(b"node").chain(&[0u8; 4]))
        .chain(1u64.to_le_bytes().iter().chain(b"(").chain(&[0u8; 7]))
        .chain(4u64.to_le_bytes().iter().chain(b"type").chain(&[0u8; 4]))
        .chain(7u64.to_le_bytes().iter().chain(b"regular").chain(&[0u8; 1]))
        .chain(8u64.to_le_bytes().iter().chain(b"contents"))
        .chain(
            11u64
                .to_le_bytes()
                .iter()
                .chain("hello world".as_bytes())
                .chain(&[0u8; 5]),
        )
        .chain(1u64.to_le_bytes().iter().chain(b")").chain(&[0u8; 7]))
        .chain(1u64.to_le_bytes().iter().chain(b")").chain(&[0u8; 7]))
        .chain(1u64.to_le_bytes().iter().chain(b")").chain(&[0u8; 7]))
        .chain(1u64.to_le_bytes().iter().chain(b")").chain(&[0u8; 7]))
        .chain(1u64.to_le_bytes().iter().chain(b")").chain(&[0u8; 7]))
        .copied()
        .collect();

    let output = libnar::to_vec(dir.path()).unwrap();
    assert_eq!(output, expected);
}
