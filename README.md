# libnar

Library for reading and writing from NAR (Nix Archive) files written in Rust.

The NAR format, developed exclusively for the [Nix package manager], is a fully
deterministic and reproducible alternative to the [tar] archive format. It is
used to serialize and deserialize filesystem objects, such as files and
directories and symlinks, in and out of the Nix store. Unlike tar, `.nar`
archives have the following properties:

[Nix package manager]: https://nixos.org/nix/
[tar]: https://en.wikipedia.org/wiki/Tar_(computing)

1. Deterministic ordering when unpacking files
2. Fully specified, no undefined or implementation-specific behavior
3. Strips out non-reproducible file metadata (creation time, last access time,
   owner and group IDs, all file mode permissions except for executable) before
   packing and normalizes them at unpacking time
4. Strips out the `setuid` and sticky bits along with all filesystem-specific
   extended attributes before packing

`libnar` is a fast and lightweight implementation of the Nix Archive format in
Rust and provides a convenient interface for opening, creating, packing, and
unpacking `.nar` files. It is intentionally kept as minimal as possible with few
dependencies to keep the codebase portable.

## Examples

### Opening an archive

```rust
use std::fs::File;

use libnar::Archive;

fn main() {
    let file = File::open("/path/to/archive.nar").unwrap();
    let mut nar = Archive::new(file).unwrap();

    let entries = nar.entries().unwrap();
    for entry in entries {
        let entry = entry.unwrap();
        println!("{:?}", entry);
    }
}
```

### Extracting an archive

```rust
use std::fs::File;

use libnar::de::Parameters;

fn main() {
    let file = File::open("/path/to/archive.nar").unwrap();
    Parameters::new().unpack(file, "./archive").unwrap();
}
```

### Creating an archive

```rust
use std::fs::File;

fn main() {
    let mut file = File::create("/path/to/archive.nar").unwrap();
    libnar::to_writer(&mut file, "/path/to/archive").unwrap();
}
```

## License

`libnar` is free and open source software distributed under the terms of both
the [MIT](LICENSE-MIT) and the [Apache 2.0](LICENSE-APACHE) licenses.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
