//! Harness page assets, baked into the binary.
//!
//! `build.rs` stages the slide-preview Trunk `dist/` into
//! `$OUT_DIR/preview-dist` — from `SLIDE_PREVIEW_DIST`, an existing
//! checked-out `dist/`, or a `trunk build` — and it is embedded here.
//! This is what lets `slide preview serve` run with no `--assets`
//! flag, including from a binary installed outside the repo. When the
//! stage step found nothing the embedded directory is empty and the
//! daemon falls back to requiring `--assets`.

use include_dir::{Dir, include_dir};

static HARNESS: Dir<'_> = include_dir!("$OUT_DIR/preview-dist");

/// Embedded bytes for a harness file (`index.html`, the trunk JS/wasm,
/// `snippets/...`), or `None` if absent.
pub fn get(relative: &str) -> Option<&'static [u8]> {
    HARNESS.get_file(relative).map(|file| file.contents())
}

/// Whether any harness assets were embedded at build time.
pub fn is_embedded() -> bool {
    !HARNESS.entries().is_empty()
}
