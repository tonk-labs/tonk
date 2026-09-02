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

/// Light-DOM markup mounted by the Hub account custom element. The profile
/// library supplies its geometry, so their visual contract is checked here
/// together.
const HUB_ACCOUNT_MARKUP: &str = include_str!("../../tonk-workspace/src/ui_hub_account.html");
/// The shared stylesheet: the theme tokens and the hub chrome's CSS,
/// which moved out of the directory view so the /settings route (its
/// own view, same chrome) is styled by the same block.
const HUB_STYLES: &str = include_str!("../../tonk-ui/styles.css");
const SETTINGS_PANEL_MARKUP: &str =
    include_str!("../../tonk-workspace/src/ui_account_settings.html");

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

fn css_rule<'a>(document: &'a str, selector: &str) -> &'a str {
    document
        .split(selector)
        .nth(1)
        .and_then(|css| css.split('}').next())
        .unwrap_or_else(|| panic!("profile library must contain the `{selector}` rule"))
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
fn it_defaults_the_space_alias_to_blank_in_core() {
    assert!(
        STANDARD_LIBRARY.contains("entity: tonk:blank"),
        "core.yaml must seed the default tonk/space -> tonk:blank alias",
    );
}

#[test]
fn it_describes_space_removal_as_device_local() {
    let rendered_words = PROFILE_LIBRARY
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        rendered_words.contains("Remove {name} from this device?"),
        "the Hub confirmation must name the device-local removal boundary",
    );
    assert!(
        rendered_words.contains("Removing it does not delete other members' copies."),
        "the Hub confirmation must preserve independent replicas",
    );
    assert!(
        !rendered_words.contains("from this account, on every"),
        "the Hub must not imply that local removal erases account or peer copies",
    );
}

#[test]
fn it_uses_the_shared_native_dialog_for_hub_space_removal() {
    for contract in [
        "<ui-space-remove>",
        "data-space-remove-open",
        "<tonk-dialog data-space-remove-dialog",
        "data-dialog=\"close\"",
        "type=\"submit\" html:form=\"remove-{subject}\"",
    ] {
        assert!(
            PROFILE_LIBRARY.contains(contract),
            "Hub removal must preserve the shared dialog contract `{contract}`"
        );
    }
    for rejected in [
        ".rm-radio",
        "class=\"rm-radio\"",
        "class=\"mscrim\"",
        "role=\"alertdialog\"",
        "for=\"rm-",
    ] {
        assert!(
            !PROFILE_LIBRARY.contains(rejected),
            "Hub removal must not retain `{rejected}`"
        );
    }
}

#[test]
fn it_keeps_keyboard_focus_visible_on_inverted_hub_controls() {
    assert!(
        HUB_STYLES
            .contains("box-shadow:inset 0 0 0 2px var(--on-ink), inset 0 0 0 4px var(--ink);"),
        "Hub focus rings need both palette poles so selected and ordinary controls stay visible",
    );
}

#[test]
fn it_styles_the_absent_space_as_tonk_edge_chrome() {
    let absent = PROFILE_LIBRARY
        .split("/* The absent-space state")
        .nth(1)
        .and_then(|source| source.split("/* Keyed off the display's own state").next())
        .expect("profile library must contain the absent-space style block");
    for contract in [
        "--edge-page:#e8e6e4",
        "--edge-ink:#38182a",
        "--edge-page:#161313",
        "--edge-ink:#e2dfdd",
        "font-family:'IBM Plex Sans Condensed'",
        "box-shadow:0 0 0 1px var(--edge-ring)",
        "min-height:44px",
        "transition-property:scale",
    ] {
        assert!(
            absent.contains(contract),
            "the absent-space state must preserve the Tonk edge contract `{contract}`"
        );
    }
    assert!(
        !absent.contains("var(--wa-color-brand-fill-loud)"),
        "the absent-space action must not retain the outdated Web Awesome pill skin"
    );
    assert!(
        css_rule(absent, ".space-unknown-statement {").contains("color:var(--edge-ink)"),
        "the absent-space statement must override the global heading skin with local mode ink"
    );
    for contract in [
        "class=\"space-unknown-mast\"",
        "class=\"space-unknown-wall\"",
        "you don't have this space",
        "join a space",
    ] {
        assert!(
            PROFILE_LIBRARY.contains(contract),
            "the absent-space markup must preserve `{contract}`"
        );
    }
    assert!(
        !PROFILE_LIBRARY.contains("you don't have this spot")
            && !PROFILE_LIBRARY.contains("join a spot"),
        "the absent-space panel must use current user-facing terminology"
    );
    assert!(
        STANDARD_LIBRARY.contains("aria-label=\"Space name\""),
        "the repository-name control must expose the current noun"
    );
}

#[test]
fn it_keeps_the_hub_on_the_shared_theme_tokens() {
    // Colors live in ONE place — the theme block at the top of
    // `tonk-ui/styles.css`, injected into every sealed guest. The hub must
    // CONSUME the shared tokens without restating a palette of its own; a
    // local literal here is the drift this contract exists to prevent.
    for consumed in [
        "background:var(--page)",
        "color:var(--ink)",
        "background:var(--cur)",
        "var(--frost-solid)",
        "var(--wash-p)",
    ] {
        assert!(
            HUB_STYLES.contains(consumed),
            "the Hub must consume the shared theme token `{consumed}`",
        );
    }
    for restated in [
        "--page:#",
        "--ink:#",
        "--cur:#",
        "--panel:#",
        "--frost:rgba(",
    ] {
        assert!(
            !PROFILE_LIBRARY.contains(restated),
            "the Hub must not restate the palette locally (`{restated}`)",
        );
    }
}

#[test]
fn it_builds_one_centered_hub_launcher_with_a_settings_route() {
    for contract in [".hubcol", "width:432px", ".hc-view"] {
        assert!(
            HUB_STYLES.contains(contract),
            "the centered Hub launcher must contain `{contract}`",
        );
    }
    assert!(
        PROFILE_LIBRARY.contains("create new space"),
        "the centered Hub launcher must contain `create new space`",
    );
    let hubbar = HUB_STYLES
        .split(".hubbar {")
        .nth(1)
        .and_then(|css| css.split('}').next())
        .expect("the Hub bar rule");
    for rejected in ["position:fixed", "right:", "border-radius"] {
        assert!(
            !hubbar.contains(rejected),
            "the centered Hub bar must reject `{rejected}`",
        );
    }
    for (selector, width) in [(".hc-acct {", "width:144px"), (".hc-view {", "width:288px")] {
        assert!(
            css_rule(HUB_STYLES, selector).contains(width),
            "the proportional desktop Hub cell `{selector}` must contain `{width}`",
        );
    }
    let rejected = "class=\"shead";
    assert!(
        !PROFILE_LIBRARY.contains(rejected),
        "the centered Hub launcher must reject `{rejected}`",
    );
    assert_eq!(
        PROFILE_LIBRARY.matches("no spaces yet").count(),
        1,
        "the empty Hub must state the neutral roster fact exactly once",
    );
    for rejected in ["signed out", "no spaces available"] {
        assert!(
            !PROFILE_LIBRARY.to_lowercase().contains(rejected),
            "a provider-free local profile must not claim `{rejected}`",
        );
    }
    for contract in [
        "<ui-hub-account>",
        "href=\"/space/{subject}\"",
        "class=\"snew-form\"",
    ] {
        assert!(
            PROFILE_LIBRARY.contains(contract),
            "provider-free Hub access must preserve `{contract}`",
        );
    }
}

#[test]
fn it_separates_the_account_roster_into_independent_blocks() {
    let menu = css_rule(HUB_STYLES, ".account-menu {");
    for contract in ["display:flex", "flex-direction:column", "gap:7px"] {
        assert!(
            menu.contains(contract),
            "the account roster must contain `{contract}`",
        );
    }
    let profiles = css_rule(HUB_STYLES, ".account-menu__profiles {");
    assert!(
        profiles.contains("gap:7px"),
        "profiles must keep the same 7px rhythm as Hub space rows",
    );
    let row = css_rule(HUB_STYLES, ".account-menu__row {");
    assert!(
        row.contains("box-shadow:0 0 0 1px var(--ring)"),
        "each account row must carry its own ring",
    );
    assert!(
        !row.contains("border-bottom"),
        "separated account blocks must not retain fused row dividers",
    );
}

#[test]
fn it_serves_settings_as_a_routed_page_of_the_hub() {
    // `/settings` is a real route: the hub chrome with the settings
    // section already open (`view="settings"`), reached by href from the
    // account menu and the FAB alike. The legacy account panel lives at
    // /account, where the settings panel's ceremony links point.
    assert!(PROFILE_LIBRARY.contains("path: \"/settings\""));
    assert!(PROFILE_LIBRARY.contains("<ui-hub-account view=\"settings\">"));
    assert!(!PROFILE_LIBRARY.contains(".hub-settings"));
    assert!(HUB_ACCOUNT_MARKUP.contains("data-settings-view"));
    assert!(HUB_ACCOUNT_MARKUP.contains("href=\"/settings\""));
    // The panes live in the shared panel — one element, two seats: the
    // Hub's account tab and the FAB's settings dialog on the space route.
    assert!(HUB_ACCOUNT_MARKUP.contains("<ui-account-settings>"));
    assert!(SETTINGS_PANEL_MARKUP.contains("data-pane=\"account\""));
    assert!(SETTINGS_PANEL_MARKUP.contains("data-pane=\"devices\""));
    assert!(SETTINGS_PANEL_MARKUP.contains("href=\"/account\""));
    assert!(!SETTINGS_PANEL_MARKUP.contains("href=\"/settings\""));
}

#[test]
fn it_renders_join_refusals_as_neutral_edge_walls() {
    let failure = PROFILE_LIBRARY
        .split("view!:\n  this: tonk:join/failure")
        .nth(1)
        .and_then(|tail| tail.split("# ROUTING (profile branch)").next())
        .expect("join failure view");
    let route = PROFILE_LIBRARY
        .split("view!:\n  this: tonk:join/route")
        .nth(1)
        .and_then(|tail| tail.split("# The /inspector and /diagnose routes").next())
        .expect("join route view");

    for rejected in ["<wa-callout", "variant=\"danger\"", "{reason}"] {
        assert!(
            !failure.contains(rejected),
            "the closed-invitation wall must not expose `{rejected}`",
        );
    }
    assert!(failure.contains("this share link is closed"));
    assert!(route.contains("you do not have access to this space"));
    for wall in [("closed", failure), ("no-access", route)] {
        assert_eq!(
            wall.1.matches("class=\"ebtn solid\"").count(),
            1,
            "the {} wall must carry exactly one solid ink door",
            wall.0,
        );
        assert!(wall.1.contains("start a new space"));
        assert!(wall.1.contains("join this space"));
    }
}

#[test]
fn it_sizes_the_join_route_to_the_dynamic_mobile_viewport() {
    let route = PROFILE_LIBRARY
        .split("view!:\n  this: tonk:join/route")
        .nth(1)
        .and_then(|tail| tail.split("# The /inspector and /diagnose routes").next())
        .expect("join route view");
    let join_view = css_rule(route, ".join-view {");
    let fallback = join_view
        .find("min-height:100vh")
        .expect("join route must retain the legacy viewport fallback");
    let dynamic = join_view
        .find("min-height:100dvh")
        .expect("join route must use the dynamic mobile viewport");
    assert!(
        fallback < dynamic,
        "the dynamic viewport declaration must follow and override the fallback"
    );
}

#[test]
fn it_declares_mobile_target_and_input_floors_for_hub_and_join() {
    for contract in [
        ".hubbar, .hcell { height:44px; min-height:44px; }",
        ".account-menu__row, .sempty, .srow, .snew { min-height:44px; }",
    ] {
        assert!(
            HUB_STYLES.contains(contract),
            "mobile Hub CSS must contain `{contract}`"
        );
    }
    for contract in [
        ".edge-mast { left:16px; top:18px; width:98px; min-height:44px;",
        ".edge-field, .ebtn, .ebtn.solid button { min-height:44px; }",
        ".edge-field { height:44px; padding-bottom:0; align-items:stretch; }",
        ".edge-input { min-height:44px; font-size:16px; }",
        ".edge-noun, .edge-cur { align-self:flex-end; margin-bottom:8px; }",
    ] {
        assert!(
            PROFILE_LIBRARY.contains(contract),
            "mobile Hub/join CSS must contain `{contract}`"
        );
    }
}
