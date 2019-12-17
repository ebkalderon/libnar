use std::env;
use std::fs::File;

use libnar::Archive;

fn main() {
    let path = env::args().nth(1).expect("Expected path to *.nar archive");
    let file = File::open(path).unwrap();

    let mut nar = Archive::new(file);
    let entries = nar.entries().unwrap();

    for entry in entries {
        let entry = entry.unwrap();
        println!("{:?}", entry);
    }
}
