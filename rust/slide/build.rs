//! Stage the slide-preview harness page into `$OUT_DIR/preview-dist`
//! so it can be embedded in the binary (see `preview::assets`). The
//! daemon then serves the harness with no `--assets` flag.
//!
//! Resolution order, first hit wins:
//! 1. `SLIDE_PREVIEW_DIST` — an explicit built `dist/`. The nix build
//!    passes the slide-preview Trunk derivation here so it never runs
//!    `trunk` inside slide's own sandbox.
//! 2. A checked-out `rust/slide-preview/dist/` (a prior `trunk build`).
//! 3. A best-effort `trunk build` of the harness crate (dev convenience
//!    when the dist is missing).
//!
//! If none resolve, the staged directory is left empty and the daemon
//! falls back to requiring `--assets`. The build never fails on a
//! missing harness — embedding is a convenience, not a hard dependency.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR set by cargo"));
    let stage = out_dir.join("preview-dist");
    let _ = fs::remove_dir_all(&stage);
    fs::create_dir_all(&stage).expect("create preview-dist stage dir");

    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set"));
    let harness = manifest.join("../slide-preview");
    let default_dist = harness.join("dist");

    println!("cargo:rerun-if-env-changed=SLIDE_PREVIEW_DIST");
    println!("cargo:rerun-if-changed={}", harness.join("src").display());
    println!(
        "cargo:rerun-if-changed={}",
        harness.join("index.html").display()
    );
    println!("cargo:rerun-if-changed={}", default_dist.display());

    let resolved = env::var_os("SLIDE_PREVIEW_DIST")
        .map(PathBuf::from)
        .filter(|dir| dir.is_dir())
        .or_else(|| default_dist.is_dir().then(|| default_dist.clone()))
        .or_else(|| (trunk_build(&harness) && default_dist.is_dir()).then(|| default_dist.clone()));

    match resolved {
        Some(dir) => copy_tree(&dir, &stage),
        None => println!(
            "cargo:warning=slide preview harness not embedded (no SLIDE_PREVIEW_DIST, no \
             rust/slide-preview/dist, and `trunk build` unavailable); `slide preview serve` will \
             require --assets"
        ),
    }
}

/// Best-effort `trunk build` of the harness crate. Returns whether it
/// succeeded; a missing `trunk` or a non-zero exit is swallowed (the
/// caller falls through to leaving the embed empty).
fn trunk_build(harness: &Path) -> bool {
    Command::new("trunk")
        .arg("build")
        .current_dir(harness)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Recursively copy `src` into `dst` (which already exists).
fn copy_tree(src: &Path, dst: &Path) {
    for entry in fs::read_dir(src).expect("read harness dist") {
        let entry = entry.expect("dir entry");
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            fs::create_dir_all(&to).expect("create nested dir");
            copy_tree(&from, &to);
        } else {
            fs::copy(&from, &to).expect("copy harness file");
        }
    }
}
