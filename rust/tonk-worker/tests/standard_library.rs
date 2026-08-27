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

/// The notebook template — a prose document whose `dialog` fences are
/// live query cells.
const NOTEBOOK_LIBRARY: &str = include_str!("../../tonk-core/assets/library/notebook.yaml");

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

/// Form controls expose their submitted value at `.value` (a
/// `RadioNodeList` included). Nothing else on an `<input>` is a value
/// slot, so a read path ending anywhere else resolves to `undefined`.
const FORM_CONTROL_PROPERTIES: &[&str] = &["value"];

/// The read-path prefix that addresses a named control inside the
/// submitting form.
const FORM_CONTROL_PREFIX: &str = "dom.event.current-target.elements.";

/// Every `elements.<name>/<leaf>` read path in `document` must end at a
/// property a form control actually has.
///
/// The event extractor walks the path against the live form and aborts
/// the WHOLE command when a leaf resolves to `undefined`
/// (`ExtractError::UnresolvedField`) — no claim posted, no
/// `preventDefault`, a dead button with only a console warning. A leaf
/// typo is therefore silent at seed time and fatal at click time, which
/// is what this catches. The trap is naming the field and its leaf
/// after the same thing (`revocation/revocation-url`): the leaf is a JS
/// property, not a label.
fn assert_form_reads_resolve(label: &str, document: &str) {
    for (index, _) in document.match_indices(FORM_CONTROL_PREFIX) {
        let rest = &document[index + FORM_CONTROL_PREFIX.len()..];
        let identifier = rest
            .split(|c: char| c.is_whitespace() || c == '"' || c == '\'')
            .next()
            .unwrap_or_default();
        let Some((control, leaf)) = identifier.split_once('/') else {
            panic!("{label}: `{FORM_CONTROL_PREFIX}{identifier}` names no property to read");
        };
        assert!(
            FORM_CONTROL_PROPERTIES.contains(&leaf),
            "{label}: `{FORM_CONTROL_PREFIX}{control}/{leaf}` reads \
             `form.elements.{control}.{}` — not a form-control property, so \
             the command aborts unresolved on submit",
            kebab_to_camel(leaf),
        );
    }
}

/// The event layer camel-cases every path segment at read time; mirror it
/// so the failure message names the property the browser would look for.
fn kebab_to_camel(segment: &str) -> String {
    let mut camel = String::with_capacity(segment.len());
    let mut upper = false;
    for c in segment.chars() {
        if c == '-' {
            upper = true;
        } else if upper {
            camel.extend(c.to_uppercase());
            upper = false;
        } else {
            camel.push(c);
        }
    }
    camel
}

#[test]
fn it_reads_form_controls_at_properties_they_have() {
    assert_form_reads_resolve("standard library (core.yaml)", STANDARD_LIBRARY);
    assert_form_reads_resolve("profile library (profile.yaml)", PROFILE_LIBRARY);
    assert_form_reads_resolve("sheets template (sheets.yaml)", SHEETS_LIBRARY);
    assert_form_reads_resolve("wiki template (wiki.yaml)", WIKI_LIBRARY);
    assert_form_reads_resolve("board template (board.yaml)", BOARD_LIBRARY);
}

#[test]
fn it_leaves_network_bearing_space_bindings_unquoted() {
    assert!(
        PROFILE_LIBRARY.contains("space={id}"),
        "the FAB space binding must be resolved by the renderer"
    );
    assert!(
        !PROFILE_LIBRARY.contains("space=\"{id}\""),
        "a quoted binding can reach membership fetches unresolved"
    );
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
fn it_lowers_core_concatenated_with_the_notebook_template() {
    // Same single-document seed as the other templates: core.yaml is
    // concatenated ahead of notebook.yaml, so the template must reuse
    // core's concepts without redeclaring their anchors.
    let seeded = format!("{STANDARD_LIBRARY}\n{NOTEBOOK_LIBRARY}");
    assert_library_lowers("core.yaml + notebook.yaml (notebook template)", &seeded);
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
