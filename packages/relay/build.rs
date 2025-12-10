use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();

    println!("cargo:rerun-if-changed=scripts/extract-wasm.js");
    println!("cargo:rerun-if-changed=package.json");

    let output = Command::new("node")
        .arg("scripts/extract-wasm.js")
        .current_dir(&manifest_dir)
        .output()
        .expect("Failed to run node scripts/extract-wasm.js - make sure Node.js is installed and bun install has been run");

    if !output.status.success() {
        panic!(
            "extract-wasm.js failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let out_path = Path::new(&out_dir).join("tonk_core_bg.wasm");
    fs::write(&out_path, &output.stdout).expect("Failed to write WASM to OUT_DIR");

    println!(
        "cargo:warning=Embedded WASM from @tonk/core ({} bytes)",
        output.stdout.len()
    );
}
