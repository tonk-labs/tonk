//! Write what a deployment serves under `/_host/` for the kernels: the Tonk
//! kernel and persona, each under its content identifier, and `kernel.json`
//! naming them, into the directory given.

fn main() {
    let directory = std::path::PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("the directory to write the components into"),
    );
    let kernels = directory.join("kernel");
    std::fs::create_dir_all(&kernels).expect("the directory is writable");
    let kernel = tonk_components::TONK_KERNEL_ID;
    let persona = tonk_components::TONK_PERSONA_ID;
    for (id, bytes) in [
        (kernel, tonk_components::TONK_KERNEL_BYTES),
        (persona, tonk_components::TONK_PERSONA_BYTES),
    ] {
        std::fs::write(kernels.join(format!("{id}.wasm")), bytes).expect("the kernel writes");
    }
    std::fs::write(
        directory.join("kernel.json"),
        format!("{{\"kernel\":\"{kernel}\",\"persona\":\"{persona}\"}}\n"),
    )
    .expect("kernel.json writes");
    println!("kernel {kernel} persona {persona}");
}
