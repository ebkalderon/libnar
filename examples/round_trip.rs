use std::fs::File;

const TARGET_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src");

fn main() {
    let nar = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("libnar.nar");

    let mut file = File::create(&nar).unwrap();
    libnar::to_writer(&mut file, TARGET_PATH).unwrap();

    let file = File::open(&nar).unwrap();
    libnar::de::Parameters::new()
        .unpack(file, "libnar")
        .unwrap();
}
