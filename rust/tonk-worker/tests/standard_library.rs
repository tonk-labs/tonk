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

/// The sheets template — seeded on top of core when chosen. Must lower
/// self-contained (it re-declares the core concepts it references).
const SHEETS_LIBRARY: &str = include_str!("../../tonk-core/assets/library/sheets.yaml");

/// The wiki template — seeded on top of core when chosen, like sheets.
const WIKI_LIBRARY: &str = include_str!("../../tonk-core/assets/library/wiki.yaml");

/// The board template — seeded on top of core when chosen, like sheets.
const BOARD_LIBRARY: &str = include_str!("../../tonk-core/assets/library/board.yaml");

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
fn it_lowers_core_concatenated_with_the_sheets_template() {
    // The worker never seeds sheets.yaml alone: for the `sheets`
    // template it concatenates core.yaml ahead of it into ONE document
    // and evaluates the whole thing in a single commit. The template
    // therefore relies on the concepts core declares (tonk:view,
    // tonk:view/directory, tonk:replica) and must not redeclare them —
    // duplicate anchors are rejected within a document. Analyze the
    // same concatenation the seed builds so that collision is caught
    // here rather than at first launch.
    let seeded = format!("{STANDARD_LIBRARY}\n{SHEETS_LIBRARY}");
    assert_library_lowers("core.yaml + sheets.yaml (sheets template)", &seeded);
}

#[test]
fn it_lowers_core_concatenated_with_the_wiki_template() {
    // Same single-document seed as sheets: core.yaml is concatenated
    // ahead of wiki.yaml, so the template must reuse core's concepts
    // (tonk:view, tonk:view/directory, tonk:replica, `component`)
    // without redeclaring their anchors.
    let seeded = format!("{STANDARD_LIBRARY}\n{WIKI_LIBRARY}");
    assert_library_lowers("core.yaml + wiki.yaml (wiki template)", &seeded);
}

#[test]
fn it_lowers_core_concatenated_with_the_board_template() {
    // Same single-document seed as sheets and wiki: core.yaml is
    // concatenated ahead of board.yaml, so the template must reuse
    // core's concepts (tonk:view, tonk:view/directory, tonk:replica,
    // `component`) without redeclaring their anchors.
    let seeded = format!("{STANDARD_LIBRARY}\n{BOARD_LIBRARY}");
    assert_library_lowers("core.yaml + board.yaml (board template)", &seeded);
}

#[test]
fn it_overrides_the_space_alias_to_the_wiki_in_wiki() {
    assert!(
        WIKI_LIBRARY.contains("entity: tonk:wiki"),
        "wiki.yaml must override tonk/space -> tonk:wiki",
    );
}

#[test]
fn it_overrides_the_space_alias_to_the_board_in_board() {
    assert!(
        BOARD_LIBRARY.contains("entity: tonk:board/canvas"),
        "board.yaml must override tonk/space -> tonk:board/canvas",
    );
}

#[test]
fn it_defaults_the_space_alias_to_blank_in_core() {
    assert!(
        STANDARD_LIBRARY.contains("entity: tonk:blank"),
        "core.yaml must seed the default tonk/space -> tonk:blank alias",
    );
    assert!(
        !STANDARD_LIBRARY.contains("model: tonk:sheet"),
        "core.yaml must not carry the sheets workspace after the split",
    );
}

#[test]
fn it_overrides_the_space_alias_to_binder_in_sheets() {
    assert!(
        SHEETS_LIBRARY.contains("entity: tonk:binder"),
        "sheets.yaml must override tonk/space -> tonk:binder",
    );
}
