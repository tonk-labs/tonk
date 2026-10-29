//! Builds the components of the Tonk build for `wasm32-wasip3`, and writes
//! their bytes to `components.rs` in `OUT_DIR`. See `src/lib.rs`.
//!
//! It runs `cargo build` for each component crate of the workspace, as
//! Sapling builds its guest kernels and Wasmtime its test programs. The nested
//! build has a target directory of its own, in `OUT_DIR`, since the outer build
//! holds the lock of its own target directory. It takes none of the flags of
//! the outer build, which are for the host.

use std::env;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::Command;

/// The target each component builds for: the world's WASI.
const TARGET: &str = "wasm32-wasip3";

/// The components: the crate each is built from, the features it is built
/// with, and the constant that names it. The persona is the same crate as the
/// kernel, built for the world `sapling:kernel/persona`.
const COMPONENTS: &[(&str, &[&str], &str)] = &[
    ("tonk-kernel", &[], "TONK_KERNEL"),
    ("tonk-kernel", &["persona"], "TONK_PERSONA"),
];

/// The crates a component is built from, and the workspace's manifest and
/// lock: a change to any of them builds the components again.
const SOURCES: &[&str] = &[
    "dialog-reactor",
    "tonk-account",
    "tonk-analytics",
    "tonk-analyzer",
    "tonk-common",
    "tonk-core",
    "tonk-evaluator",
    "tonk-host",
    "tonk-identity",
    "tonk-intent",
    "tonk-invite",
    "tonk-kernel",
    "tonk-kernel-world",
    "tonk-language-server",
    "tonk-library",
    "tonk-lingo",
    "tonk-macros",
    "tonk-notation",
    "tonk-router",
    "tonk-schema",
    "tonk-template",
    "tonk-worker",
    "tonk-worker-api",
    "../Cargo.toml",
    "../Cargo.lock",
];

fn main() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets it"));
    let rust_dir = manifest_dir.parent().expect("the crate is in `rust/`");
    for source in SOURCES {
        println!(
            "cargo::rerun-if-changed={}",
            rust_dir.join(source).display()
        );
    }

    let target_dir = out_dir.join("target");
    let mut generated = String::new();
    // One build after another, in the same target directory: the components
    // share every dependency, so each build after the first rebuilds only its
    // own crate. Each is copied out before the next build writes over it.
    for (package, features, constant) in COMPONENTS {
        let cargo = env::var_os("CARGO").expect("cargo sets it");
        let mut command = Command::new(cargo);
        command
            .arg("build")
            .arg("--release")
            .arg(format!("--target={TARGET}"))
            .arg("--manifest-path")
            .arg(rust_dir.join("../Cargo.toml"))
            .arg("--package")
            .arg(package)
            .env("CARGO_TARGET_DIR", &target_dir)
            // The flags of the outer build are for the host.
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .env_remove("RUSTFLAGS")
            .env_remove("CARGO_BUILD_TARGET")
            .env_remove("CARGO_BUILD_RUSTFLAGS")
            // A component is built, not linted, whatever the outer command:
            // under `cargo clippy` the outer build runs rustc through clippy.
            .env_remove("RUSTC_WORKSPACE_WRAPPER")
            .env_remove("CLIPPY_ARGS")
            .env("CARGO_PROFILE_RELEASE_OPT_LEVEL", "s")
            .env("CARGO_PROFILE_RELEASE_LTO", "true")
            .env("CARGO_PROFILE_RELEASE_CODEGEN_UNITS", "1")
            // Debug info only: the polyfill reads the section of features a
            // module was built with, so that it can skip a scan of every body.
            .env("CARGO_PROFILE_RELEASE_STRIP", "debuginfo");
        let mut features = features.to_vec();
        if env::var_os("CARGO_FEATURE_CONNECTION_INVITES").is_some() {
            features.push("connection-invites");
        }
        if !features.is_empty() {
            command.arg("--features").arg(features.join(","));
        }
        let status = command.status().expect("cargo runs");
        assert!(
            status.success(),
            "cargo could not build {constant}: {status}"
        );
        let built = target_dir
            .join(TARGET)
            .join("release")
            .join(format!("{}.wasm", package.replace('-', "_")));
        assert!(
            built.is_file(),
            "cargo built no component at {}",
            built.display()
        );
        let bytes = std::fs::read(&built).expect("the component reads");
        let path = out_dir.join(format!("{}.wasm", constant.to_lowercase()));
        std::fs::write(&path, &bytes).expect("OUT_DIR is writable");
        writeln!(
            generated,
            "/// The bytes of the component `{constant}`, built from the crate `{package}`.\npub const {constant}_BYTES: &[u8] = include_bytes!({:?});\n/// Its content identifier: CIDv1, raw, BLAKE3, base32, as a space's boot\n/// cell pins it.\npub const {constant}_ID: &str = {:?};",
            path.display().to_string(),
            content_id(&bytes),
        )
        .expect("a write to a string does not fail");
    }
    std::fs::write(out_dir.join("components.rs"), generated).expect("OUT_DIR is writable");
}

/// The content identifier of `bytes`: CIDv1, the raw codec, BLAKE3, in the
/// multibase `b` (RFC 4648 base32, lowercase, unpadded). `tonk-kernel-host`
/// names a kernel the same way, and tests that the two agree.
fn content_id(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut cid = vec![0x01, 0x55, 0x1e, 0x20];
    cid.extend_from_slice(blake3::hash(bytes).as_bytes());
    let mut out = String::from("b");
    let (mut buffer, mut bits) = (0u32, 0);
    for byte in cid {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buffer >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    out
}
