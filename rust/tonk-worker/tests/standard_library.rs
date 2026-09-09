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

/// The component libraries. Each is seeded the same way and lowers the
/// same way, so each belongs in the lowering gate — until now only
/// `core.yaml` and `profile.yaml` were covered, which meant a broken
/// `table.yaml` reached the seed before anything complained.
const TABLE_LIBRARY: &str = include_str!("../../tonk-core/assets/library/table.yaml");
const NOTEBOOK_LIBRARY: &str = include_str!("../../tonk-core/assets/library/notebook.yaml");
const PROSE_LIBRARY: &str = include_str!("../../tonk-core/assets/library/prose.yaml");
const ISSUE_LIBRARY: &str = include_str!("../../tonk-core/assets/library/issue.yaml");

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
fn it_distinguishes_leaving_from_deleting_a_space() {
    let rendered_words = PROFILE_LIBRARY
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        rendered_words
            .contains("Leave {name}? This removes the space and its local data from this device."),
        "the Hub must call a joined-space removal leaving",
    );
    assert!(
        rendered_words
            .contains("You'll need another invite link to join again. Other members keep access."),
        "the leave confirmation must explain how access is recovered and who keeps it",
    );
    assert!(
        rendered_words.contains("data-space-provider={provider}"),
        "the Hub must expose hosted ownership to the action component",
    );
    assert!(
        rendered_words.contains("data-space-founded={founded-at}"),
        "the Hub must distinguish a local-only creation from a joined space",
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
fn it_hides_space_absence_slots_before_display_initialization() {
    let chrome = PROFILE_LIBRARY
        .split("<tonk-display with={id} entity={id} model=tonk:repository view=title>")
        .nth(1)
        .and_then(|source| source.split("</tonk-display>").next())
        .expect("space chrome must contain its repository title display");
    for slot in ["no-model", "no-entity", "no-view"] {
        let marker = format!("slot=\"{slot}\"");
        let attributes = chrome
            .split(&marker)
            .nth(1)
            .and_then(|source| source.split('>').next())
            .expect("space fallback slot must exist");
        assert!(
            attributes.split_whitespace().any(|attr| attr == "hidden"),
            "{slot} must start hidden: light-DOM slots can paint before the display initializes"
        );
    }
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
        "padding:48px 16px 80px",
        ".space-unknown-mast { position:relative; display:block; width:132px;",
        "margin:0 auto 56px",
        ".space-unknown-wall { width:min(576px, 100%); margin:0 auto;",
    ] {
        assert!(
            absent.contains(contract),
            "the absent-space state must use the shared upper-page geometry `{contract}`"
        );
    }
    for contract in [
        "class=\"space-unknown-mast\"",
        "class=\"space-unknown-wall\"",
        "invalid link",
        "class=\"space-unknown-home\" href=\"/\">go to home",
    ] {
        assert!(
            PROFILE_LIBRARY.contains(contract),
            "the absent-space markup must preserve `{contract}`"
        );
    }
    assert_eq!(
        PROFILE_LIBRARY
            .matches("class=\"space-unknown-narrator\"")
            .count(),
        1,
        "the absent-space explanation must render as one card"
    );
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
    for contract in [
        ".hubcol",
        "width:min(576px, calc(100vw - 32px))",
        ".hc-view",
    ] {
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
    for (selector, width) in [(".hc-acct {", "width:144px"), (".hc-view {", "width:432px")] {
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
    // The empty stack carries NO words. An account with no spaces and an
    // account whose spaces are still downloading are indistinguishable
    // from the stack, so any sentence here is wrong in one of those two
    // cases. The waiting is stated where it is known — the account cell
    // holds a skeleton while the link runs.
    assert_eq!(
        PROFILE_LIBRARY.matches("no spaces yet").count(),
        0,
        "the empty Hub must not claim a roster fact it cannot tell from a pending download",
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
fn it_mints_an_invite_when_copying_a_hub_space_link() {
    assert!(
        PROFILE_LIBRARY.contains("<ui-copy-link space={subject}"),
        "the Hub copy action must name the space whose invite it mints"
    );
    assert!(
        !PROFILE_LIBRARY.contains("ui-copy-link url=\"/space/{subject}\""),
        "the Hub must not copy its member-only route as though it were an invite"
    );
    for (state, label) in [
        ("idle", "idle"),
        ("copying", "copying"),
        ("copied", "copied"),
        ("blocked", "failed"),
        ("failed", "failed"),
    ] {
        assert!(
            HUB_STYLES.contains(&format!(
                "data-share-state=\"{state}\"] [data-share-copy-label=\"{label}\"]"
            )),
            "the Hub invite action must display its `{label}` answer in `{state}` state"
        );
    }
}

#[test]
fn it_aligns_the_hub_space_actions_in_one_flex_context() {
    assert!(
        css_rule(HUB_STYLES, ".verbs ui-copy-link,").contains("display:contents"),
        "the copy-link host must not offset its button from delete or leave"
    );
    assert!(
        css_rule(HUB_STYLES, ".verbs {").contains("gap:18px"),
        "desktop Hub actions must remain a close visual group"
    );
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
    // account menu and the FAB alike. Every account act lives in this
    // panel; nothing links out to a top-level page. `/settings/link` is
    // the same page opened by a terminal asking for access.
    assert!(PROFILE_LIBRARY.contains("path: \"/settings\""));
    assert!(PROFILE_LIBRARY.contains("path: \"/settings/link\""));
    assert!(PROFILE_LIBRARY.contains("<ui-hub-account view=\"settings\">"));
    assert!(!PROFILE_LIBRARY.contains(".hub-settings"));
    assert!(HUB_ACCOUNT_MARKUP.contains("data-settings-view"));
    assert!(HUB_ACCOUNT_MARKUP.contains("href=\"/settings\""));
    // The panes live in the shared panel — one element, two seats: the
    // Hub's account tab and the FAB's settings dialog on the space route.
    // Device revocation is no longer a separate settings pane.
    assert!(HUB_ACCOUNT_MARKUP.contains("<ui-account-settings>"));
    assert!(SETTINGS_PANEL_MARKUP.contains("data-pane=\"account\""));
    assert!(!SETTINGS_PANEL_MARKUP.contains("data-pane=\"devices\""));
    assert!(SETTINGS_PANEL_MARKUP.contains("data-pane=\"link\""));
    assert!(SETTINGS_PANEL_MARKUP.contains("data-delete-account-open"));
    assert!(SETTINGS_PANEL_MARKUP.contains("data-sign-out-open"));
    assert!(SETTINGS_PANEL_MARKUP.contains("<div class=\"sect\">sign out</div>"));
    assert!(
        SETTINGS_PANEL_MARKUP.contains("disconnect this account; keep local spaces on this device")
    );
    assert!(SETTINGS_PANEL_MARKUP.contains("sign out on this device"));
    assert!(SETTINGS_PANEL_MARKUP.contains("heading=\"confirm sign out\""));
    assert!(SETTINGS_PANEL_MARKUP.contains(
        "this disconnects the account from this browser. local spaces stay on this device, including spaces that have not been backed up or synced. you can sign into this or another account later."
    ));
    assert!(SETTINGS_PANEL_MARKUP.contains("data-sign-out-submit>sign out</button>"));
    assert!(!SETTINGS_PANEL_MARKUP.contains("remove this device"));
    assert!(!SETTINGS_PANEL_MARKUP.contains("confirm device removal"));
    assert!(
        !SETTINGS_PANEL_MARKUP
            .contains("remove all data associated with this account from this device")
    );
    assert!(SETTINGS_PANEL_MARKUP.contains("data-add-passkey"));
    assert!(!SETTINGS_PANEL_MARKUP.contains("href=\"/account\""));
    assert!(!SETTINGS_PANEL_MARKUP.contains("href=\"/settings\""));
    assert!(SETTINGS_PANEL_MARKUP.contains("data-settings-name"));
    // Editable settings fields use native text inputs and native carets.
    let name_row = SETTINGS_PANEL_MARKUP
        .split("<span>display name</span>")
        .nth(1)
        .and_then(|rest| rest.split("</div>").next())
        .expect("the display-name row");
    assert!(
        !name_row.contains("<i class=\"cur\""),
        "an unfocused display-name field must not draw an editing cursor",
    );
    assert!(
        SETTINGS_PANEL_MARKUP.contains("data-delete-confirm type=\"text\""),
        "the deletion confirm is a native text input",
    );
    assert!(
        SETTINGS_PANEL_MARKUP.contains("data-delete-confirm-label>delete account</b>"),
        "the deletion confirm must say exactly what to type",
    );
    assert!(
        !SETTINGS_PANEL_MARKUP.contains("<i class=\"cur\""),
        "settings inputs must not draw terminal-style cursors",
    );
}

#[test]
fn it_keeps_machine_instructions_in_the_production_copy_prompt() {
    let copied = STANDARD_LIBRARY
        .split("copy-label=\"Copy prompt\"")
        .nth(1)
        .and_then(|tail| tail.split("</wa-copy-button>").next())
        .expect("the agent prompt copy button");
    let command = "npx --yes @tonk/cli connect '{link}'";
    assert_eq!(
        copied.matches(command).count(),
        1,
        "the clipboard prompt must carry one production CLI command",
    );
    assert!(
        !STANDARD_LIBRARY
            .split("copy-label=\"Copy prompt\"")
            .next()
            .unwrap_or_default()
            .contains(command),
        "machine instructions must not be visible before the copy button",
    );
    assert!(
        copied.contains(
            "Only report connected after it prints &quot;Agent connection confirmed&quot;"
        ),
        "the clipboard prompt must define the success boundary",
    );
    assert!(
        copied.contains("npx --yes @tonk/cli --space NAME connect"),
        "the resume command must work without a globally installed CLI",
    );
    assert!(
        copied.contains("npx --yes @tonk/cli connect INVITE --name NEW_NAME"),
        "the prompt must explain how to reclaim after revoked saved authority",
    );
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
    assert!(failure.contains("this share link expired"));
    assert!(failure.contains("ask the person who shared this space to send you a new link"));
    assert!(failure.contains("go to home"));
    assert!(!failure.contains("edge-field edge-field--settled"));
    assert!(!failure.contains("paste a new link"));
    assert!(!failure.contains("tonk-join-retry"));
    assert!(!failure.contains("join this space"));
    assert!(!failure.contains("start a new space"));
    assert!(!route.contains("<form"));
    assert!(!route.contains("<input"));
    assert!(!route.contains("tonk-invite-link"));
    assert!(route.contains("<tonk-page on:join=tonk:join>"));
    assert_eq!(failure.matches("class=\"ebtn solid\"").count(), 1);
}

#[test]
fn it_keeps_join_failure_chrome_and_actions_visually_consistent() {
    let route = PROFILE_LIBRARY
        .split("view!:\n  this: tonk:join/route")
        .nth(1)
        .and_then(|tail| tail.split("# The /inspector and /diagnose routes").next())
        .expect("join route view");

    assert!(
        css_rule(route, ".edge-statement {").contains("color:var(--edge-ink)"),
        "join statements must override the global heading colour with local mode ink",
    );
    assert!(
        route.contains(".join-status:has(.edge-wall--closed) .join-opening { display:none; }"),
        "retained failure content must suppress the opening row even while its display reconnects",
    );

    let opening = css_rule(route, ".join-opening {");
    for contract in [
        "position:fixed",
        "inset:0",
        "align-items:center",
        "justify-content:center",
        "background:var(--edge-page)",
    ] {
        assert!(
            opening.contains(contract),
            "the pending join must match the centred boot pulse with `{contract}`",
        );
    }
    assert!(PROFILE_LIBRARY.contains("class=\"join-opening-status\""));
    assert!(PROFILE_LIBRARY.contains("class=\"tonk-pulse\""));
    assert!(route.contains(
        ".join-view:has(.join-status-slot[data-state=\"ready\"]):not(:has(.edge-wall--closed))"
    ));

    let action = css_rule(route, ".ebtn {");
    for contract in [
        "height:40px",
        "border:0",
        "border-radius:0",
        "font:inherit",
        "line-height:1",
        "white-space:normal",
    ] {
        assert!(
            action.contains(contract),
            "join actions must normalize links and native buttons with `{contract}`",
        );
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
    for contract in [
        "padding:48px 16px 80px",
        ".edge-mast { position:relative; display:block; width:132px;",
        "margin:0 auto 56px",
        ".edge-wall { width:min(576px, 100%); margin:0 auto;",
    ] {
        assert!(
            route.contains(contract),
            "the join state must use the shared upper-page geometry `{contract}`"
        );
    }
}

#[test]
fn it_declares_mobile_target_and_input_floors_for_hub_and_join() {
    for contract in [
        ".hubbar, .hcell { height:44px; min-height:44px; }",
        ".account-menu__row, .srow, .snew { min-height:44px; }",
    ] {
        assert!(
            HUB_STYLES.contains(contract),
            "mobile Hub CSS must contain `{contract}`"
        );
    }
    for contract in [
        ".edge-mast { width:98px; min-height:44px; margin-bottom:40px;",
        ".ebtn { height:44px; min-height:44px; }",
    ] {
        assert!(
            PROFILE_LIBRARY.contains(contract),
            "mobile Hub/join CSS must contain `{contract}`"
        );
    }
}

/// Lower a component library the way the seed reaches it: on top of
/// the standard library, whose `view` / `event` / `command` concepts it
/// references. `analyze_local` resolves names within one document, so
/// the concatenation is what stands in for "core is already seeded".
fn assert_component_library_lowers(label: &str, document: &str) {
    assert_library_lowers(label, &format!("{STANDARD_LIBRARY}\n{document}"));
}

#[dialog_common::test]
fn it_lowers_the_table_library() {
    assert_component_library_lowers("table library (table.yaml)", TABLE_LIBRARY);
}

#[dialog_common::test]
fn it_lowers_the_notebook_library() {
    assert_component_library_lowers("notebook library (notebook.yaml)", NOTEBOOK_LIBRARY);
}

#[dialog_common::test]
fn it_lowers_the_prose_library() {
    assert_component_library_lowers("prose library (prose.yaml)", PROSE_LIBRARY);
}

#[dialog_common::test]
fn it_lowers_the_issue_library() {
    assert_component_library_lowers("issue library (issue.yaml)", ISSUE_LIBRARY);
}

// The `on:` binding gates that used to live here — a dangling
// `on:<name>`, a binding that cannot fill its command — scanned the
// YAML with a second, indentation-sensitive parser that shipped two
// truncation bugs of its own. The analyzer now enforces both
// invariants at lowering (`E_UNKNOWN_EVENT_DECLARATION`,
// `E_EVENT_COMMAND_MISMATCH`), the `it_lowers_the_*` tests above run
// that path over every shipped library, and
// `the_binding_and_interpolation_checks_reach_the_shipped_libraries`
// in `tonk-analyzer` pins that those checks actually fire on this
// corpus.

/// An `event!:` declaration lowers with no library seeded at all.
///
/// This is what makes `event` a built-in rather than a library concept:
/// like `command!:` and `rule!:`, a declaration is schema an author
/// writes, so it has to resolve on a bare branch. Declared in the
/// standard library instead, it would work only where that library had
/// been seeded — and a lean repo, the profile meta-branch, or a `tonk
/// eval` fixture would all fail to parse one.
#[dialog_common::test]
fn an_event_declaration_lowers_without_the_library() {
    let document = r#"event!: &on/tap
  type: "click"
  prevent-default: true
  where:
    subject: "{this}"
    time: ".timeStamp"

command!: &bump
  with:
    subject:
      description: The thing being bumped
      the: io.gozala.bump/subject
      as: entity
      cardinality: one
  maybe:
    time:
      description: A per-event nonce
      the: io.gozala.bump/time
      as: float
      cardinality: one
"#;
    assert_library_lowers("a bare `event!:` document", document);
}

/// The optional side-effect flags really are optional: a declaration
/// that suppresses neither must lower. Every declaration in the
/// library omits them, so a regression here would break all of them at
/// once.
#[dialog_common::test]
fn an_event_declaration_may_omit_the_side_effect_flags() {
    let document = r#"event!: &on/plain
  type: "click"
  where:
    subject: "{this}"
"#;
    assert_library_lowers("an `event!:` with no flags", document);
}

/// The wire predicate `tonk-template` builds matches the built-in.
///
/// Two hand-maintained copies of one shape: the built-in descriptor is
/// the source of truth and the query builder mirrors it, because that
/// crate emits JSON rather than descriptor types. Drift would show up
/// as an event declaration that resolves but reads back empty, which is
/// the silent class of failure again — so it is asserted rather than
/// trusted to a comment.
#[dialog_common::test]
fn the_event_query_predicate_matches_the_builtin() {
    let builtin = tonk_schema::builtin::lookup_concept("event").expect("`event` is a built-in");
    // `.concept()` unwraps the durability tag; the wire predicate has
    // no such wrapper.
    let serialized =
        serde_json::to_value(builtin.descriptor.concept()).expect("descriptor serializes");
    let builtin_with = serialized
        .get("with")
        .and_then(serde_json::Value::as_object)
        .expect("the built-in has a `with` map");

    let predicate = tonk_template::resolve::event_predicate();
    let query_with = predicate
        .get("with")
        .and_then(serde_json::Value::as_object)
        .expect("the predicate has a `with` map");

    // The query pins only the required fields — the optional flags are
    // read separately, since pinning them would make a declaration that
    // omits them match nothing.
    for field in query_with.keys() {
        let ours = &query_with[field];
        let theirs = builtin_with
            .get(field)
            .unwrap_or_else(|| panic!("the built-in has no `{field}` field"));
        assert_eq!(
            ours.get("the"),
            theirs.get("the"),
            "`{field}`: the query and the built-in disagree on `the`",
        );
        assert_eq!(
            ours.get("cardinality"),
            theirs.get("cardinality"),
            "`{field}`: the query and the built-in disagree on cardinality",
        );
    }
    assert!(
        query_with.contains_key("type") && query_with.contains_key("where"),
        "the query must pin both required fields",
    );
}

/// The wire query the display builds for a view must describe the same
/// concept the analyzer lowers against.
///
/// `view` is a built-in now, so the two can drift the way `event`'s
/// pair could: the display's predicate is hand-mirrored JSON, and a
/// `the` or a cardinality that disagrees resolves nothing at render
/// time with no error anywhere. The `bindings` field is checked
/// through its own query for the same reason the event flags are —
/// it is optional, so pinning it in the view query would make a view
/// that carries none match nothing.
#[dialog_common::test]
fn the_view_queries_match_the_builtin() {
    let builtin = tonk_schema::builtin::lookup_concept("view").expect("`view` is a built-in");
    let serialized =
        serde_json::to_value(builtin.descriptor.concept()).expect("descriptor serializes");
    let builtin_with = serialized
        .get("with")
        .and_then(serde_json::Value::as_object)
        .expect("the built-in has a `with` map");

    let predicate = tonk_template::resolve::view_predicate();
    let query_with = predicate
        .get("with")
        .and_then(serde_json::Value::as_object)
        .expect("the predicate has a `with` map");
    assert!(
        query_with.contains_key("show") && !query_with.contains_key("bindings"),
        "the view query pins `show` only; `bindings` is optional and read separately",
    );

    // The bindings query carries its own copy of the field, so check
    // it against the built-in too.
    let bindings_query = tonk_template::resolve::view_bindings_query("tonk:demo")
        .expect("the bindings query builds");
    let bindings_query =
        serde_json::to_value(&bindings_query).expect("the bindings query serializes");
    let bindings_with = bindings_query
        .get("predicate")
        .and_then(|predicate| predicate.get("with"))
        .and_then(serde_json::Value::as_object)
        .expect("the bindings query has a `with` map");

    for (field, ours) in query_with.iter().chain(bindings_with.iter()) {
        let theirs = builtin_with
            .get(field)
            .unwrap_or_else(|| panic!("the built-in has no `{field}` field"));
        assert_eq!(
            ours.get("the"),
            theirs.get("the"),
            "`{field}`: the query and the built-in disagree on `the`",
        );
        assert_eq!(
            ours.get("cardinality"),
            theirs.get("cardinality"),
            "`{field}`: the query and the built-in disagree on cardinality",
        );
    }
    assert!(
        bindings_with.contains_key("bindings"),
        "the bindings query must pin the field it exists to read",
    );
}

/// A command with a Rust handler must match on attributes its notation
/// declaration actually carries.
///
/// The two halves are written in different files and nothing but this
/// connects them: the YAML `command!:` declares what a transient carries,
/// and the Rust concept declares what the handler decodes. When they
/// drift the transient still commits, the handler never runs, and the UI
/// looks like it worked. `notebook.yaml` says it in its own words — "a
/// field the struct requires but the command does not declare is simply
/// absent from the descriptor, so the decode fails and the binding falls
/// through in silence".
///
/// That is not hypothetical: moving `notebook/create` into its own
/// namespace in the YAML without moving `CreateNotebook` with it silently
/// broke notebook creation on any freshly seeded branch, and nothing
/// failed.
///
/// The invariant is containment, not equality. A concept may deliberately
/// match on FEWER attributes than the command declares — `CreateSpace` is
/// matched name-only so a frozen older descriptor still decodes it, and
/// reads the optional remote from the raw facts. What is never sound is
/// the other direction: an attribute the handler requires that no
/// declaration produces.
#[dialog_common::test]
fn every_handled_command_matches_attributes_its_declaration_carries() {
    use dialog_reactor::Decode as _;

    let mut declared = std::collections::BTreeMap::new();
    for document in [
        STANDARD_LIBRARY,
        PROFILE_LIBRARY,
        TABLE_LIBRARY,
        NOTEBOOK_LIBRARY,
        PROSE_LIBRARY,
        ISSUE_LIBRARY,
    ] {
        for (name, attributes) in parse_command_attributes(document) {
            declared
                .entry(name)
                .or_insert_with(std::collections::BTreeSet::new)
                .extend(attributes);
        }
    }

    // Every notation command the worker decodes in typed Rust. A command
    // consumed by a `rule!:` has no Rust concept and is not listed.
    //
    // `tonk/rename-repository` is deliberately absent, and the reason is
    // worth stating: the name belongs to TWO different commands. The one
    // `core.yaml` declares carries `{subject, name}` and is consumed by a
    // space-side `rule!:`. `tonk_schema::command::RenameRepository`
    // carries `{space, name}` and is dispatched from the profile branch by
    // the FAB, which inlines its own descriptor because the space-side
    // rule cannot consume a claim made on the profile branch. They share a
    // notation name and nothing else, so pairing them here would compare
    // two unrelated things. The FAB's claim is pinned against the struct
    // in `fab_drift.rs` instead, which is where that pairing lives.
    let handled: Vec<(&str, Vec<String>)> = vec![
        (
            "space/create",
            tonk_schema::command::CreateSpace::trigger_attributes(),
        ),
        (
            "space/enable-sync",
            tonk_schema::command::CreateSpace::trigger_attributes(),
        ),
        (
            "space/remove",
            tonk_schema::command::RemoveSpace::trigger_attributes(),
        ),
        (
            "tonk/invite",
            tonk_schema::command::Invite::trigger_attributes(),
        ),
        (
            "tonk/pause-sync",
            tonk_schema::command::PauseSync::trigger_attributes(),
        ),
        (
            "profile/rename",
            tonk_schema::command::ProfileRename::trigger_attributes(),
        ),
        (
            "member/expel",
            tonk_schema::command::ExpelMember::trigger_attributes(),
        ),
        (
            "tonk/join",
            tonk_schema::command::Join::trigger_attributes(),
        ),
        (
            "tonk/load",
            tonk_schema::command::Load::trigger_attributes(),
        ),
    ];

    for (name, required) in handled {
        let carries = declared
            .get(name)
            .unwrap_or_else(|| panic!("no `command!: &{name}` in any shipped library"));
        let missing: Vec<&String> = required
            .iter()
            .filter(|attribute| !carries.contains(*attribute))
            .collect();
        assert!(
            missing.is_empty(),
            "`{name}`: the handler matches on {missing:?}, which its notation \
             declaration does not carry — the transient would commit and the \
             handler would never run. Declared: {carries:?}",
        );
    }
}

/// Every `the:` a `command!:` declaration names, keyed by command name.
fn parse_command_attributes(
    document: &str,
) -> std::collections::BTreeMap<String, std::collections::BTreeSet<String>> {
    let mut out = std::collections::BTreeMap::new();
    let mut lines = document.lines().peekable();
    while let Some(line) = lines.next() {
        let Some(name) = line.trim_start().strip_prefix("command!: &") else {
            continue;
        };
        let name = name
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_string();
        let mut attributes = std::collections::BTreeSet::new();
        // Peek, and skip blanks rather than consuming terminators.
        while let Some(body) = lines.peek().copied() {
            // A blank line does not end the declaration: `description: |`
            // block scalars contain them, and treating one as the end
            // silently truncated the scan — which is how this gate first
            // reported that `tonk/invite` declared no attributes at all.
            if body.trim().is_empty() {
                lines.next();
                continue;
            }
            if !body.starts_with("  ") {
                break;
            }
            lines.next();
            // Exactly a field's own `the:`, six spaces in. Prose inside a
            // block scalar is indented deeper and must not be read as a
            // declaration however it happens to begin.
            if let Some(attribute) = body.strip_prefix("      the: ") {
                attributes.insert(attribute.trim().to_string());
            }
        }
        out.insert(name, attributes);
    }
    out
}
