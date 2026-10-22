//! Write what a deployment serves under `/_host/` for the kernel: the Tonk
//! kernel under its content identifier, and `kernel.json` naming it, into the
//! directory given.

fn main() {
    let directory = std::path::PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("the directory to write the components into"),
    );
    let kernels = directory.join("kernel");
    std::fs::create_dir_all(&kernels).expect("the directory is writable");
    let id = tonk_components::TONK_KERNEL_ID;
    std::fs::write(
        kernels.join(format!("{id}.wasm")),
        tonk_components::TONK_KERNEL_BYTES,
    )
    .expect("the kernel writes");
    std::fs::write(
        directory.join("kernel.json"),
        format!("{{\"kernel\":\"{id}\"}}\n"),
    )
    .expect("kernel.json writes");
    println!("kernel {id}");
}
