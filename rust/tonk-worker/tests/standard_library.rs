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

/// The lean profile library — seeded onto the profile meta branch,
/// backs the Hub (the `space` directory view + the `space/create`
/// command and its form).
const PROFILE_LIBRARY: &str = include_str!("../../tonk-core/assets/library/profile.yaml");

/// The served showcase demo, embedded at compile time.
const DEMO_LIBRARY: &str = include_str!("../../tonk-core/assets/library/demo.yaml");

/// Lower a library document the same way the seed does, asserting it
/// parses, analyzes with no running system, and lowers to claims.
fn assert_library_lowers(label: &str, document: &str) {
    let parsed = tonk_notation::parse(document);
    let syntax = parsed
        .syntax
        .unwrap_or_else(|| panic!("{label} must parse with no diagnostics"));
    let tree = tonk_analyzer::analyzer::analyze_local(&syntax)
        .unwrap_or_else(|e| panic!("{label} must analyze with no running system: {e:#?}"));

    // Both halves of the seed must lower without error: the concept
    // claims and the `rule!:` installs. A failure in either is a
    // document that would break the seed.
    let request = tree
        .analysis
        .lower_to_claims()
        .unwrap_or_else(|e| panic!("{label} must lower to claims: {e:#?}"));
    let _rules = tree.analysis.rule_installs();

    assert!(
        !request.claims.is_empty(),
        "{label} should lower to at least one claim",
    );
}

#[test]
fn it_lowers_the_standard_library() {
    assert_library_lowers("standard library (core.yaml)", STANDARD_LIBRARY);
}

#[test]
fn it_lowers_the_profile_library() {
    assert_library_lowers("profile library (profile.yaml)", PROFILE_LIBRARY);
}

#[test]
fn it_parses_the_showcase_demo() {
    // The demo is seeded *after* the scaffold and resolves its bare
    // concept references (`workspace`, `board`, …) against the
    // committed scaffold, so it can't be `analyze_local`'d here with
    // no running system. A parse check still catches the syntax-level
    // faults that would break the seed; the full cross-document lower
    // is exercised by the wasm `it_seeds_showcase_on_top_of_scaffold`
    // test in `router::repository`.
    let parsed = tonk_notation::parse(DEMO_LIBRARY);
    assert!(
        parsed.diagnostics.is_empty(),
        "showcase demo must parse with no diagnostics; got {:?}",
        parsed.diagnostics,
    );
    assert!(
        parsed.syntax.is_some(),
        "showcase demo must produce a syntax tree",
    );
}
