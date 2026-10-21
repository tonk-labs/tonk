//! Print the content identifier of the kernel wasm named, as a space's boot
//! cell pins it.
fn main() {
    let path = std::env::args().nth(1).expect("a kernel to name");
    let bytes = std::fs::read(&path).expect("the kernel reads");
    println!("{}", kernel_host::boot::kernel_id(&bytes));
}
