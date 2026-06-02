//! The standard library lowers cleanly.
//!
//! The service worker seeds `tonk-core/assets/library/core.yaml` into
//! every new repository by fetching the served asset and running it
//! through the evaluate pipeline (`parse → analyze → commit`). This
//! test runs the same `parse → analyze_local → lower` front half
//! against the source document, so a document that would fail the
//! seed at runtime — a parse error, an unresolved `&anchor`, a bad
//! concept declaration, a rule that won't lift — fails here instead.
//!
//! Native-only: it reads the document from disk (there is no
//! filesystem on wasm) and needs no running system. The file is read
//! with `std::fs` at runtime rather than `include_str!` on purpose —
//! embedding it would make `core.yaml` a build input of this crate,
//! and editing the library would then force a wasm rebuild, defeating
//! the point of serving it as a static asset.

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;

#[test]
fn it_lowers_the_standard_library() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tonk-core/assets/library/core.yaml");
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));

    let parsed = tonk_notation::parse(&text);
    let syntax = parsed
        .syntax
        .expect("standard library must parse with no diagnostics");
    let tree = tonk_analyzer::analyzer::analyze_local(&syntax)
        .expect("standard library must analyze with no running system");

    // Both halves of the seed must lower without error: the concept
    // claims and the `rule!:` installs. A failure in either is a
    // document that would break the repository-creation seed.
    let request = tree
        .analysis
        .lower_to_claims()
        .expect("standard library must lower to claims");
    let _rules = tree.analysis.rule_installs();

    assert!(
        !request.claims.is_empty(),
        "standard library should lower to at least one claim",
    );
}
