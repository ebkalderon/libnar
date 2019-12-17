use std::fs::File;
use std::path::Path;

use libnar::Archive;

const TARGET_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src");

fn main() {
    let nar = Path::new(TARGET_PATH).with_extension("nar");

    let mut file = File::create(&nar).unwrap();
    libnar::to_writer(&mut file, TARGET_PATH).unwrap();

    let file = File::open(&nar).unwrap();
    let mut nar = Archive::new(file);
    nar.unpack("src2").unwrap();
}
