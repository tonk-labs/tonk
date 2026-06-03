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
//! Native-only (there is no filesystem on wasm, and it needs no
//! running system). The document is embedded with `include_str!`
//! rather than read with `std::fs` at runtime: CI runs the suite from
//! a `cargo nextest archive`, which bundles the compiled test binaries
//! but not arbitrary runtime data files, so a runtime read of a
//! sibling crate's asset fails in the sandbox. Embedding makes the
//! library a build input of this *native* test binary only (it travels
//! inside the archive) — the `#[cfg(not(wasm32))]` gate keeps it out
//! of the wasm bundle, so editing the library still never forces a
//! wasm rebuild.

#![cfg(not(target_arch = "wasm32"))]

/// The served standard library, embedded at compile time. Path is
/// relative to this source file.
const STANDARD_LIBRARY: &str = include_str!("../../tonk-core/assets/library/core.yaml");

#[test]
fn it_lowers_the_standard_library() {
    let parsed = tonk_notation::parse(STANDARD_LIBRARY);
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
