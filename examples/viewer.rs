fn main() {
    let path = std::env::args().nth(1).expect("Expected path to *.nar archive");
    let file = std::fs::File::open(path).unwrap();

    let entries = libnar::de::Parameters::new().entries(file).unwrap();

    for entry in entries {
        let entry = entry.unwrap();
        println!("{:?}", entry);
    }
}
