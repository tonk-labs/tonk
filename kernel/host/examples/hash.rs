//! Print the BLAKE3 hash of each file named, as a pin names a kernel.
fn main() {
    for path in std::env::args().skip(1) {
        let bytes = std::fs::read(&path).expect("the file reads");
        println!("{} {path}", blake3::hash(&bytes).to_hex());
    }
}
