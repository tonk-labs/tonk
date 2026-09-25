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
const META_LIBRARY: &str = include_str!("../../tonk-core/assets/library/meta.yaml");

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

#[dialog_common::test]
fn it_lowers_the_standard_library() {
    assert_library_lowers("standard library (core.yaml)", STANDARD_LIBRARY);
}

/// The meta library describes a real shape, not an aspirational one.
///
/// It is documentation that has to stay true: every attribute it names
/// is one the worker actually writes to a `meta` branch, so lowering it
/// is what keeps the description from drifting from the rows.
#[dialog_common::test]
fn it_lowers_the_meta_library() {
    assert_library_lowers("meta library (meta.yaml)", META_LIBRARY);
}

#[dialog_common::test]
fn it_lowers_the_profile_library() {
    assert_library_lowers("profile library (profile.yaml)", PROFILE_LIBRARY);
}

/// The switcher row binds the handle a command can actually act on.
///
/// The command carries `handle`, read off the button's `data-handle`,
/// which the row fills from the DURABLE `xyz.tonk.roster/name`. Binding
/// the label instead would look identical on screen and fail at the
/// worker, which validates the handle against the roster — so this pins
/// which field the button carries, not merely that it carries one.
#[dialog_common::test]
fn it_switches_profiles_by_handle_not_by_label() {
    let row = PROFILE_LIBRARY
        .split("data-handle=")
        .nth(1)
        .expect("the switcher row must bind a handle");
    let bound = row.split_whitespace().next().unwrap_or_default();
    assert_eq!(
        bound, "{name}",
        "the handle must come from the durable roster name, not the overlay label",
    );
    assert!(
        PROFILE_LIBRARY.contains("on:switch-profile=tonk:switch-profile"),
        "the row must dispatch the switch command",
    );
}

/// The account cell offers to link one when no account is linked.
///
/// A fact-backed label renders nothing when its fact is absent, and an
/// empty account cell is unusable: that cell IS how an account gets
/// linked, so a person with none would have nothing to click.
///
/// The slot is `empty`, not `no-entity`. The display runs in directory
/// mode here — no `entity=` — where an absent row is "empty"; the
/// `no-entity` slot never shows without an entity to be absent. Both
/// spellings look right and only one renders, which is why this pins
/// the one that does.
#[dialog_common::test]
fn it_offers_to_link_an_account_when_none_is() {
    // The label mounts, not every account-name display: the settings
    // pane renders the same fact through the `editable` facet, which is
    // a field to type in rather than a cell to click, and offering to
    // link an account there would be a second, worse door.
    let mounts: Vec<&str> = PROFILE_LIBRARY
        .match_indices("<span data-account-label>")
        .map(|(at, _)| {
            let rest = &PROFILE_LIBRARY[at..];
            rest.split("</tonk-display>").next().unwrap_or(rest)
        })
        .collect();
    assert_eq!(
        mounts.len(),
        1,
        "one hub page carries the account label, whichever path it is on",
    );
    for mount in mounts {
        assert!(
            mount.contains(r#"<span slot="empty">add an account</span>"#),
            "an unlinked account must still offer the link; got {mount}",
        );
    }
}

/// The hub keeps the handles its e2e suite drives it by.
///
/// These are `data-*` attributes with no styling or behaviour attached:
/// their whole job is to be findable from outside the view. Rewriting
/// the bar's markup dropped four of them at once, and each one only
/// surfaced as a separate e2e failure a full run apart.
///
/// Listed here rather than left to the browser suite because a unit
/// test says which handle vanished in seconds, where the e2e says only
/// that something timed out after half a minute.
#[dialog_common::test]
fn it_keeps_the_handles_the_suite_drives_the_hub_by() {
    for handle in [
        "data-account-trigger",
        "data-account-label",
        "data-account-menu",
        "data-add-profile",
        "data-open-settings",
        "data-return-spaces",
        "data-settings-name",
        "data-settings-email",
        "data-settings-passkey-device",
    ] {
        // Matched as a real attribute, not anywhere in the text: a
        // comment naming the handle would otherwise satisfy this, which
        // is exactly how the first version of this test passed while
        // the attribute was gone.
        let attribute = format!("{handle} ");
        let attribute_last = format!("{handle}>");
        assert!(
            PROFILE_LIBRARY.contains(&attribute) || PROFILE_LIBRARY.contains(&attribute_last),
            "`{handle}` is how the suite finds this control; without it the \
             test times out rather than saying what moved",
        );
    }
}

/// With no account, the account page raises the signup itself.
///
/// The bar's cells are links, so the page they lead to is the only
/// door to linking an account; a version that merely showed an empty
/// panel would strand a new browser with nothing to click.
#[dialog_common::test]
fn it_raises_the_signup_from_an_unlinked_account_cell() {
    let panel = PROFILE_LIBRARY
        .split("element!: &account-settings")
        .nth(1)
        .and_then(|rest| rest.split("\nview!:").next())
        .expect("the account-settings definition");
    assert!(
        panel.contains("    unlinked: |"),
        "the panel must be able to tell whether an account is linked",
    );
    assert!(
        panel.contains("self.link('needs-account');"),
        "and raise the ceremony in place when none is",
    );
    assert!(
        PROFILE_LIBRARY.contains(
            "    directory: |\n      <span data-account-name data-of={this}>{name}</span>"
        ),
        "the account name needs a directory facet, or the bar's label is the display's default notation",
    );
    // What the marker renders, and that it renders nothing for a profile
    // with no link row, is answered by rendering it (`tonk_display::view`);
    // here only the model the bar reads is pinned down.
    assert!(
        PROFILE_LIBRARY.contains("concept!: &account/link\n  this: state:account-link"),
        "linked is device state the worker publishes, not a fact read off the branch",
    );
}

/// The account menu's behaviour is branch data, not Rust.
///
/// Roving focus, Escape and focus restoration are DOM work with no fact
/// behind them, which is why they stayed in the element while the bar's
/// contents became views. An `element!:` is where that kind of
/// behaviour goes instead.
///
/// The concept is repeated in this library on purpose: the hub is
/// sealed to the profile meta branch and resolves a definition by
/// querying THIS branch, so one that lives only in core.yaml cannot be
/// reached. This asserts the copy is here, because without it the tag
/// renders inert and the menu silently stops responding to keys.
#[dialog_common::test]
fn it_carries_the_menu_element_on_the_branch_that_renders_it() {
    assert!(
        PROFILE_LIBRARY.contains("concept!: &element"),
        "the element concept must be seeded on the profile branch, not only in core.yaml",
    );
}

/// The account bar renders its name and switcher from facts.
///
/// The bar used to be markup an element painted from a fetch, which is
/// why "a signup is up" had to live on the document body: the route view
/// re-renders whenever profile facts land, replacing the element
/// mid-ceremony. Rendering from facts is what removes that problem
/// rather than working around it.
#[dialog_common::test]
fn it_renders_the_account_bar_from_facts() {
    for model in ["tonk:account/name", "tonk:profile/row"] {
        assert!(
            PROFILE_LIBRARY.contains(&format!(r#"model="{model}""#)),
            "the account bar must render `{model}` as a display, not paint it",
        );
    }
    assert!(
        PROFILE_LIBRARY.contains("xyz.tonk.ceremony/state"),
        "ceremony progress must be a fact the bar can read, not element state",
    );
}

/// Adding an account dispatches a command rather than fetching.
///
/// The ceremony that follows is a top-page passkey dialog the worker
/// cannot raise, so the command's handler asks the page to open it. What
/// this pins is the dispatch: if the row went back to calling
/// `/api/profiles/add` directly, the element would be back with it.
#[dialog_common::test]
fn it_adds_an_account_through_a_command() {
    assert!(
        PROFILE_LIBRARY.contains("on:add-profile=tonk:add-profile"),
        "the add-account row must dispatch the command",
    );
    assert!(
        PROFILE_LIBRARY.contains("xyz.tonk.command.add-profile/time"),
        "the command must carry a timestamp so a retry re-fires",
    );
}

/// The overlay fields the switcher renders are declared as its concept's
/// fields, so a missing one is a compile-time error rather than a blank row.
#[dialog_common::test]
fn it_declares_every_field_the_switcher_renders() {
    for attribute in [
        "xyz.tonk.roster/name",
        "xyz.tonk.roster/label",
        "xyz.tonk.roster/provider",
        "xyz.tonk.roster/active",
    ] {
        assert!(
            PROFILE_LIBRARY.contains(attribute),
            "the switcher concept must declare `{attribute}`",
        );
    }
}

#[dialog_common::test]
fn it_titles_a_downloading_space_from_the_directory_name() {
    let downloading = PROFILE_LIBRARY
        .split("    downloading: |\n")
        .nth(1)
        .and_then(|tail| tail.split("\n\n# ===").next())
        .expect("profile library downloading view");

    assert!(
        downloading.contains(r#"<tab-title text="{name} — Tonk"></tab-title>"#),
        "the downloading view must use the available directory name for the browser tab",
    );
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

#[dialog_common::test]
fn it_reads_form_controls_at_properties_they_have() {
    assert_form_reads_resolve("standard library (core.yaml)", STANDARD_LIBRARY);
    assert_form_reads_resolve("profile library (profile.yaml)", PROFILE_LIBRARY);
}

#[dialog_common::test]
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

#[dialog_common::test]
fn it_defaults_the_space_alias_to_blank_in_core() {
    assert!(
        STANDARD_LIBRARY.contains("entity: tonk:blank"),
        "core.yaml must seed the default tonk/space -> tonk:blank alias",
    );
}

#[dialog_common::test]
fn a_blank_space_waits_for_explicit_agent_invitation_intent() {
    let blank = STANDARD_LIBRARY
        .split("concept!: &blank")
        .nth(1)
        .and_then(|tail| tail.split("# The Enable-sync command").next())
        .expect("blank-space declaration");
    assert!(blank.contains("class=\"blank-canvas\""));
    assert!(!blank.contains("tonk:agent-handoff"));
    assert!(!blank.contains("page-mount"));
    assert!(!blank.contains("Generating link"));
}

#[dialog_common::test]
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

#[dialog_common::test]
fn it_uses_the_shared_native_dialog_for_hub_space_removal() {
    for contract in [
        "<space-remove ",
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

#[dialog_common::test]
fn it_keeps_keyboard_focus_visible_on_inverted_hub_controls() {
    assert!(
        PROFILE_LIBRARY
            .contains("box-shadow:inset 0 0 0 2px var(--on-ink), inset 0 0 0 4px var(--ink);"),
        "Hub focus rings need both palette poles so selected and ordinary controls stay visible",
    );
}

#[dialog_common::test]
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

#[dialog_common::test]
fn it_recovers_from_every_absent_space_directory_state() {
    let directory_probe = PROFILE_LIBRARY
        .split("<tonk-display with=\"{profile-branch}@profile:tonk\" entity={id} model=space view=downloading>")
        .nth(1)
        .and_then(|source| source.split("</tonk-display>").next())
        .expect("absent-space chrome must consult the profile directory");

    for state in ["no-model", "no-entity"] {
        assert!(
            directory_probe.contains(&format!(r#"slot="{state}" hidden"#)),
            "an absent profile directory row must render recovery for `{state}`"
        );
    }
    assert_eq!(
        directory_probe
            .matches("New to this space? Ask someone in it to send you an invite link")
            .count(),
        2,
        "both absence states must explain how to obtain an invite link"
    );
    assert_eq!(
        directory_probe.matches("<space-login>").count(),
        2,
        "both absence states must offer sign-in recovery"
    );
    assert!(
        !directory_probe.contains("you don't have access"),
        "missing local state is not proof of denied access"
    );
}

#[dialog_common::test]
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
        "open this space",
        "class=\"space-unknown-back\" href=\"/\">go to home",
        "<space-login><button type=\"button\"",
    ] {
        assert!(
            PROFILE_LIBRARY.contains(contract),
            "the absent-space markup must preserve `{contract}`"
        );
    }
    // Downloading has one narrator. Both missing-directory states offer
    // sign-in and invite guidance. A space that is merely still arriving
    // must never be sent through either recovery path.
    assert_eq!(
        PROFILE_LIBRARY
            .matches("class=\"space-unknown-narrator\"")
            .count(),
        5,
        "the absent-space panel must explain downloading, login, and invite recovery"
    );
    assert!(
        PROFILE_LIBRARY.contains("model=space view=downloading"),
        "the absent-space panel must consult the directory row before accusing the link"
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

#[dialog_common::test]
fn it_keeps_the_hub_on_the_shared_theme_tokens() {
    // Colors live in ONE place — the token block at the top of the hub's
    // own `style: ui`, which travels with the view. The hub must CONSUME
    // those tokens rather than restating a color at each use; a raw hex in
    // a rule is the drift this contract exists to prevent.
    //
    // The tokens are LITERAL by design (the fabb wireframes' values), not
    // aliases over a component library's theme: that is what lets the page
    // carry its own colors instead of depending on a stylesheet the host
    // injects into every guest.
    for consumed in [
        "background:var(--page)",
        "color:var(--ink)",
        "background:var(--cur)",
        "var(--frost-solid)",
        "var(--wash-p)",
    ] {
        assert!(
            PROFILE_LIBRARY.contains(consumed),
            "the Hub must consume the shared theme token `{consumed}`",
        );
    }
    // The literals belong to the token block and nowhere else: declared
    // once, consumed by name everywhere after.
    for declared in [
        "--page:#",
        "--ink:#",
        "--cur:#",
        "--panel:#",
        "--frost:rgba(",
    ] {
        assert!(
            PROFILE_LIBRARY.contains(declared),
            "the Hub's token block must declare `{declared}` — the palette \
             travels with the view, not with an injected stylesheet",
        );
    }
}

#[dialog_common::test]
fn it_builds_one_centered_hub_launcher_with_a_settings_route() {
    for contract in [
        ".hubcol",
        "width:min(576px, calc(100vw - 32px))",
        ".hc-view",
    ] {
        assert!(
            PROFILE_LIBRARY.contains(contract),
            "the centered Hub launcher must contain `{contract}`",
        );
    }
    assert!(
        PROFILE_LIBRARY.contains("create new space"),
        "the centered Hub launcher must contain `create new space`",
    );
    let hubbar = PROFILE_LIBRARY
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
            css_rule(PROFILE_LIBRARY, selector).contains(width),
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
        // The account affordance, whatever renders it. This named
        // `<ui-hub-account>` while the bar was an element; the contract
        // is that a provider-free Hub still offers the account tab, not
        // which tag draws it.
        "<hub-bar",
        "href=\"/space/{subject}\"",
        "class=\"snew-form\"",
    ] {
        assert!(
            PROFILE_LIBRARY.contains(contract),
            "provider-free Hub access must preserve `{contract}`",
        );
    }
}

#[dialog_common::test]
fn it_mints_an_invite_when_copying_a_hub_space_link() {
    assert!(
        PROFILE_LIBRARY.contains("<tonk-share space={subject}>"),
        "the Hub copy action must name the space whose invite it mints"
    );
    assert!(
        PROFILE_LIBRARY
            .contains(r#"<tonk-share space={subject}><button type="button" class="copy-verb">"#),
        "the copy verb is a plain button inside the share, never a form submit"
    );
    for (state, label) in [
        ("idle", "idle"),
        ("copying", "copying"),
        ("copied", "copied"),
        ("blocked", "failed"),
        ("failed", "failed"),
    ] {
        assert!(
            PROFILE_LIBRARY.contains(&format!(
                "data-share-state=\"{state}\"] [data-share-copy-label=\"{label}\"]"
            )),
            "the Hub invite action must display its `{label}` answer in `{state}` state"
        );
    }
}

#[dialog_common::test]
fn it_aligns_the_hub_space_actions_in_one_flex_context() {
    assert!(
        css_rule(PROFILE_LIBRARY, ".verbs tonk-share {").contains("display:contents"),
        "the share host must not offset its button from delete or leave"
    );
    assert!(
        css_rule(PROFILE_LIBRARY, ".verbs {").contains("gap:18px"),
        "desktop Hub actions must remain a close visual group"
    );
}

#[dialog_common::test]
fn it_separates_the_account_roster_into_independent_blocks() {
    let menu = css_rule(PROFILE_LIBRARY, ".account-menu {");
    for contract in ["display:flex", "flex-direction:column", "gap:7px"] {
        assert!(
            menu.contains(contract),
            "the account roster must contain `{contract}`",
        );
    }
    let profiles = css_rule(PROFILE_LIBRARY, ".account-menu__profiles {");
    assert!(
        profiles.contains("gap:7px"),
        "profiles must keep the same 7px rhythm as Hub space rows",
    );
    let row = css_rule(PROFILE_LIBRARY, ".account-menu__row {");
    assert!(
        row.contains("box-shadow:0 0 0 1px var(--ring)"),
        "each account row must carry its own ring",
    );
    assert!(
        !row.contains("border-bottom"),
        "separated account blocks must not retain fused row dividers",
    );
}

/// `element!:` dictionary keys are hyphenated, never camelCase.
///
/// The runtime camelCases a hyphenated key onto the prototype
/// (`link-request` becomes `self.linkRequest`), while a camelCase key
/// is lowered by the notation and never becomes callable. The settings
/// panel shipped with `linkRequest:` once and every act on it failed
/// with "is not a function", so the spelling is pinned here.
#[dialog_common::test]
fn it_hyphenates_every_element_dictionary_key() {
    for (label, library) in [
        ("profile.yaml", PROFILE_LIBRARY),
        ("core.yaml", STANDARD_LIBRARY),
    ] {
        for definition in library.split("\nelement!: &").skip(1) {
            let tag = definition.lines().next().unwrap_or("").trim();
            let body = definition.split("\n\n").next().unwrap_or("");
            let mut in_dictionary = false;
            for line in body.lines() {
                if let Some(section) = line.strip_prefix("  ")
                    && !section.starts_with(' ')
                {
                    in_dictionary = matches!(
                        section.trim_end_matches(':'),
                        "method" | "attribute" | "getter" | "setter"
                    );
                    continue;
                }
                if !in_dictionary {
                    continue;
                }
                let Some(entry) = line.strip_prefix("    ") else {
                    continue;
                };
                if entry.starts_with(' ') || entry.starts_with('#') {
                    continue;
                }
                let key = entry.split(':').next().unwrap_or("").trim();
                assert!(
                    !key.chars().any(|c| c.is_ascii_uppercase()),
                    "{label}: <{tag}> key `{key}` must be hyphenated, not camelCase",
                );
            }
        }
    }
}

#[dialog_common::test]
fn it_serves_settings_as_a_routed_page_of_the_hub() {
    // `/settings` and `/settings/link` are real routes, reached by href
    // from the account menu or opened by a terminal asking for access,
    // and both resolve to the hub itself: one view for `/` and the
    // settings path, so moving between them is a path change the view
    // re-renders in place rather than a page swap. The bar carries the
    // path and picks the section that shows.
    for route in ["/settings", "/settings/link"] {
        let definition = PROFILE_LIBRARY
            .split(&format!("path: \"{route}\"\n"))
            .nth(1)
            .expect("the route is declared");
        assert!(
            definition.starts_with("  concept: tonk:hub"),
            "{route} resolves to the hub, not a page of its own",
        );
    }
    assert!(PROFILE_LIBRARY.contains("<hub-bar class=\"hub-bar\">"));
    assert!(!PROFILE_LIBRARY.contains("tab=\"account\">"));
    assert!(!PROFILE_LIBRARY.contains(".hub-settings"));
    assert!(PROFILE_LIBRARY.contains("href=\"/settings\""));

    // The panel is markup on the branch under an `element!:` that does
    // only what a view cannot; the Rust element is gone.
    let panel = PROFILE_LIBRARY
        .split("<account-settings>\n")
        .nth(1)
        .and_then(|rest| rest.split("</account-settings>").next())
        .expect("the settings panel markup");
    assert!(PROFILE_LIBRARY.contains("element!: &account-settings"));
    assert!(!PROFILE_LIBRARY.contains("<ui-account-settings"));
    // Two panes, the account and a terminal's request; device revocation
    // is no longer a pane.
    assert!(panel.contains("data-pane=\"account\""));
    assert!(!panel.contains("data-pane=\"devices\""));
    assert!(panel.contains("data-pane=\"link\""));
    // The acts are commands the click asserts, not fetches an element
    // makes: nothing here names an `/api/` path.
    assert!(panel.contains("on:sign-out=tonk:sign-out"));
    assert!(panel.contains("on:add-passkey=tonk:add-passkey"));
    assert!(panel.contains("on:authorize-device=tonk:authorize-device"));
    assert!(!panel.contains("/api/"));
    assert!(panel.contains("data-delete-account-open"));
    assert!(panel.contains("data-sign-out-open"));
    assert!(panel.contains("<div class=\"sect\">sign out</div>"));
    assert!(panel.contains("disconnect this account; keep local spaces on this device"));
    assert!(panel.contains("sign out on this device"));
    assert!(panel.contains("heading=\"confirm sign out\""));
    assert!(panel.contains(
        "this disconnects the account from this browser. local spaces stay on this device, including spaces that have not been backed up or synced. you can sign into this or another account later."
    ));
    assert!(panel.contains("data-sign-out-submit on:sign-out=tonk:sign-out>sign out</button>"));
    assert!(!panel.contains("remove this device"));
    assert!(!panel.contains("confirm device removal"));
    assert!(!panel.contains("remove all data associated with this account from this device"));
    assert!(panel.contains("data-add-passkey"));
    // The account page's one link out is to its settings, and that is
    // the panel's own row.
    assert!(panel.contains("href=\"/settings\" data-open-settings"));
    assert_eq!(panel.matches("href=\"/settings\"").count(), 1);
    // The name, address and passkeys are facts, so the panel mounts the
    // view that renders them rather than carrying their markup; the
    // ceremony's progress is a row it words.
    assert!(
        panel.contains(r#"<tonk-display model="tonk:account/registered" view="settings">"#),
        "the account pane must render the registration facts, not paint them",
    );
    assert!(panel.contains(r#"<tonk-display model="state:ceremony" view="settings">"#));

    // The deletion dialog rides the registration view, because the
    // command carries the verified address and that view has it.
    let registered = PROFILE_LIBRARY
        .split("    settings: |\n      <div data-account-registered>")
        .nth(1)
        .and_then(|rest| rest.split("\n\n").next())
        .expect("the registration settings facet");
    assert!(registered.contains("data-delete-account-submit data-email={email}"));
    assert!(registered.contains("on:delete-account=tonk:delete-account disabled"));
    assert!(
        registered.contains(r#"<tonk-display model="tonk:space/owned" view="deletion">"#),
        "what the deletion deletes is listed from the owned-space facts",
    );
    // Editable settings fields use native text inputs and native carets.
    let name_row = PROFILE_LIBRARY
        .split("<span>display name</span>")
        .nth(1)
        .and_then(|rest| rest.split("</div>").next())
        .expect("the display-name row");
    assert!(
        !name_row.contains("<i class=\"cur\""),
        "an unfocused display-name field must not draw an editing cursor",
    );
    assert!(
        registered.contains("data-delete-confirm type=\"text\""),
        "the deletion confirm is a native text input",
    );
    assert!(
        registered.contains("data-delete-confirm-label>delete account</b>"),
        "the deletion confirm must say exactly what to type",
    );
    assert!(
        !registered.contains("<i class=\"cur\""),
        "settings inputs must not draw terminal-style cursors",
    );
}

#[dialog_common::test]
fn it_keeps_machine_instructions_in_the_production_copy_prompt() {
    for library in [
        STANDARD_LIBRARY,
        include_str!("../../tonk-core/assets/library/onboarding-agent.yaml"),
    ] {
        let copied = library
            .split("copy-label=\"copy prompt\"")
            .nth(1)
            .and_then(|tail| tail.split("</wa-copy-button>").next())
            .expect("the agent prompt copy button");
        let command = "npx --yes @tonk/cli join '{link}'";
        assert_eq!(
            copied.matches(command).count(),
            1,
            "the clipboard prompt must carry one production CLI command",
        );
        assert!(
            !library
                .split("copy-label=\"copy prompt\"")
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
            copied.contains("npx --yes @tonk/cli --space NAME join"),
            "the resume command must work without a globally installed CLI",
        );
        assert!(
            copied.contains("If access expires or is revoked, ask me for a fresh link."),
            "the prompt must request fresh authority after expiry or revocation",
        );
        assert!(
            library.contains("join --via ${JSON.stringify(page.origin)}"),
            "non-production prompts must select the exact issuing deployment",
        );
        assert!(
            library.contains("page = new URL(this.getAttribute(\"link\"))"),
            "sandboxed space views must derive loopback from the invitation origin",
        );
        assert!(
            library.contains("const executable = local ? \"tonk\" : \"npx --yes @tonk/cli\""),
            "loopback prompts must use the locally built CLI",
        );
    }
}

#[test]
fn it_keeps_ready_agent_invites_to_one_primary_action() {
    for library in [
        STANDARD_LIBRARY,
        include_str!("../../tonk-core/assets/library/onboarding-agent.yaml"),
    ] {
        let ready = library
            .split("<div data-agent-mode=\"scoped\" hidden>")
            .nth(1)
            .and_then(|tail| tail.split("</tonk-agent-prompt>").next())
            .expect("the ready agent prompt");
        assert!(ready.contains("class=\"agent-prompt__copy\""));
        assert!(
            !ready.contains("<button"),
            "a ready reusable invite needs no competing regeneration action",
        );
        assert!(!ready.contains("creating a new invite"));
        assert!(
            library.contains("data-invite-action=\"new\""),
            "lost and historical invitations must retain their recovery action",
        );
    }
}

#[dialog_common::test]
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
    assert!(route.contains("<page-mount on:join=tonk:join>"));
    assert_eq!(failure.matches("class=\"ebtn solid\"").count(), 1);
}

#[dialog_common::test]
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

#[dialog_common::test]
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

#[dialog_common::test]
fn it_declares_mobile_target_and_input_floors_for_hub_and_join() {
    for contract in [
        ".hubbar, .hcell { height:44px; min-height:44px; }",
        ".account-menu__row, .srow, .snew { min-height:44px; }",
    ] {
        assert!(
            PROFILE_LIBRARY.contains(contract),
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
/// The element-method predicate is a hand-mirrored copy of what the
/// LIBRARY declares, the same way the view predicate mirrors a built-in.
/// Nothing makes them agree, and a disagreement does not error: a query
/// whose `the:` or shape has drifted simply returns no rows, which reads
/// as "this element has no methods" everywhere it surfaces.
fn the_element_method_query_matches_the_library() {
    // Pull the declaration straight out of the seeded document rather
    // than restating it here — restating is how the copies drift.
    let declared = STANDARD_LIBRARY
        .split("concept!: &element")
        .nth(1)
        .expect("the library declares `element`");
    // Built from the constant, not written out again: a literal here
    // would match the library whatever the constant said, and the test
    // would pass through exactly the drift it exists to catch.
    let domain = tonk_template::resolve::ELEMENT_METHOD_DOMAIN;
    assert!(
        declared.contains(&format!("the: {domain}")),
        "the wire predicate's domain ({domain}) is not what the library \
         declares for `element.method`",
    );
    assert!(
        declared.contains("as: {[symbol]: text}"),
        "the library declares `method` as something other than a keyed \
         dictionary of text",
    );
    assert!(
        declared.contains("cardinality: one"),
        "the library declares `method` at a cardinality the predicate \
         does not mirror",
    );

    let predicate = tonk_template::resolve::element_method_predicate();
    let method = predicate
        .get("with")
        .and_then(|with| with.get("method"))
        .expect("the predicate declares `method`");
    assert_eq!(
        method.get("the").and_then(|the| the.get("domain")),
        Some(&serde_json::json!(domain)),
    );
    assert_eq!(
        method.get("the").and_then(|the| the.get("keyed")),
        Some(&serde_json::json!("dictionary")),
        "a keyed collection's `the:` names a domain and a key kind",
    );
    assert_eq!(method.get("as"), Some(&serde_json::json!("Text")));
    assert_eq!(method.get("cardinality"), Some(&serde_json::json!("one")));

    // The built query binds the key operand as well as the field. An
    // entry is a `(key, value)` pair; requesting only the field leaves
    // every entry keyless once folded.
    let query = tonk_template::resolve::element_method_query("did:key:zDemo")
        .expect("the method query builds");
    let query = serde_json::to_value(&query).expect("the method query serializes");
    let terms = query
        .get("terms")
        .and_then(serde_json::Value::as_object)
        .expect("the query has terms");
    assert!(
        terms.contains_key("method") && terms.contains_key("method/key"),
        "a keyed collection binds both the field and its key operand: {terms:?}",
    );
    assert_eq!(terms.get("this"), Some(&serde_json::json!("did:key:zDemo")));
}

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
        query_with.contains_key("show")
            && !query_with.contains_key("bindings")
            && !query_with.contains_key("embeds"),
        "the view query pins `show` only; `bindings` and `embeds` are optional \
         and read separately",
    );

    // The embeds query carries its own copy of the field too, and it
    // is the one that decides which entity a `with:src` reads from —
    // so a drift here is a query asking the wrong subject, which is
    // exactly the failure the compiled `embeds` field exists to make
    // impossible.
    let embeds_query =
        tonk_template::resolve::view_embeds_query("tonk:demo").expect("the embeds query builds");
    let embeds_query = serde_json::to_value(&embeds_query).expect("the embeds query serializes");
    let embeds_with = embeds_query
        .get("predicate")
        .and_then(|predicate| predicate.get("with"))
        .and_then(serde_json::Value::as_object)
        .expect("the embeds query has a `with` map");

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

    for (field, ours) in query_with
        .iter()
        .chain(bindings_with.iter())
        .chain(embeds_with.iter())
    {
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
    assert!(
        embeds_with.contains_key("embeds"),
        "the embeds query must pin the field it exists to read",
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

#[test]
fn it_offers_only_scoped_agent_prompts_without_account_approval() {
    for library in [
        STANDARD_LIBRARY,
        include_str!("../../tonk-core/assets/library/onboarding-agent.yaml"),
    ] {
        assert!(library.contains("/^#tonk-agent-v[12]=/.test(hash)"));
        assert!(!library.contains("data-agent-mode=\"legacy\""));
        assert!(!library.contains("--switch-account"));
        let unsupported = library
            .split("<div data-agent-mode=\"unsupported\" hidden>")
            .nth(1)
            .unwrap()
            .split("<div data-agent-mode=\"scoped\" hidden>")
            .next()
            .unwrap();
        assert!(!unsupported.contains("wa-copy-button"));
        assert!(unsupported.contains("tonk join"));
        assert!(!unsupported.contains("tonk link"));
        assert!(library.contains("on:new-agent-invite=tonk:new-agent-invite"));
        assert!(library.contains("event!: &on/new-agent-invite"));
        assert!(library.contains("the: xyz.tonk.agent-handoff/fresh"));

        let scoped = library
            .split("<div data-agent-mode=\"scoped\" hidden>")
            .nth(1)
            .and_then(|tail| tail.split("</wa-copy-button>").next())
            .expect("separate scoped prompt, hidden until its envelope is selected");
        assert_eq!(
            scoped.matches("npx --yes @tonk/cli join '{link}'").count(),
            1
        );
        assert!(scoped.contains("npx --yes @tonk/cli --space NAME join"));
        assert!(scoped.contains(
            "Only report connected after it prints &quot;Agent connection confirmed&quot;"
        ));
        assert!(scoped.contains("acknowledged receipt push"));
        assert!(
            scoped.contains("Multiple holders of this link share the same invitation authority")
        );
        assert!(scoped.contains("ask me for a fresh link"));
        assert!(!scoped.contains("join --agent"));
        assert!(!scoped.contains("--switch-account"));
        assert!(!scoped.contains("requires account {account}"));
    }
    let playground = include_str!("../../tonk-core/assets/library/onboarding-agent.yaml");
    assert!(playground.contains("<page-mount on:invite=tonk:agent-handoff></page-mount>"));
    let scoped = playground
        .split("<div data-agent-mode=\"scoped\" hidden>")
        .nth(1)
        .unwrap();
    assert!(scoped.contains("Do not change the space home, other pages, shared components, shared schemas, or space-wide settings."));
    assert!(scoped.contains("Agent playground&quot; page"));
    assert!(!scoped.contains("Finish with `npx --yes @tonk/cli space home"));
}

#[test]
fn it_renders_all_grant_set_receipts_without_claiming_agent_presence() {
    let receipt = STANDARD_LIBRARY
        .split("view!:\n  this: tonk:agent-connection\n")
        .nth(1)
        .unwrap()
        .split("# A space member")
        .next()
        .unwrap();
    assert!(receipt.contains("directory: |"));
    assert!(receipt.contains("entity={this} model=tonk:agent-connection"));
    assert!(receipt.contains("agent setup confirmed"));
    assert!(receipt.contains("not whether the agent is online"));
    assert!(receipt.contains("data-this={this}"));
    assert!(!receipt.contains("Your agent connected"));
    assert!(
        !STANDARD_LIBRARY
            .contains("entity=\"id:tonk:agent-connection\" model=tonk:agent-connection")
    );
}
