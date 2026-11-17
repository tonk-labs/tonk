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
const GOOEY_LOOK: &str = include_str!("../../tonk-core/assets/library/gooey/tonk-look.css");
const GOOEY_HUB: &str = include_str!("../../tonk-core/assets/library/gooey/in-browser/hub.css");
/// gooey's task-page sheets, vendored beside the library.
const TASK_PAGES_CSS: &str =
    include_str!("../../tonk-core/assets/library/gooey/in-browser/fabb-pages.css");
const TASK_FORMS_CSS: &str =
    include_str!("../../tonk-core/assets/library/gooey/in-browser/fabb-forms.css");

/// The component libraries. Each is seeded the same way and lowers the
/// same way, so each belongs in the lowering gate — until now only
/// `core.yaml` and `profile.yaml` were covered, which meant a broken
/// `table.yaml` reached the seed before anything complained.
const TABLE_LIBRARY: &str = include_str!("../../tonk-core/assets/library/table.yaml");
const NOTEBOOK_LIBRARY: &str = include_str!("../../tonk-core/assets/library/notebook.yaml");
const PROSE_LIBRARY: &str = include_str!("../../tonk-core/assets/library/prose-editor.yaml");
const CODE_EDITOR_LIBRARY: &str = include_str!("../../tonk-core/assets/library/code-editor.yaml");
const ISSUE_LIBRARY: &str = include_str!("../../tonk-core/assets/library/issue.yaml");
const META_LIBRARY: &str = include_str!("../../tonk-core/assets/library/meta.yaml");
const VERSION_CONTROL_LIBRARY: &str =
    include_str!("../../tonk-core/assets/library/version-control.yaml");

/// Lower a library document the same way the seed does, asserting it
/// parses, analyzes with no running system, and lowers to claims. Like
/// the seed, it is parsed where the library lives and what it
/// `!include`s is inlined from the bundled copies first.
async fn assert_library_lowers(label: &str, file: &str, document: &str) {
    let parsed = tonk_notation::parse_at(tonk_library::location(file), document);
    let mut syntax = parsed
        .syntax
        .unwrap_or_else(|| panic!("{label} must parse with no diagnostics"));
    let unexpanded = tonk_notation::expand(&mut syntax, &tonk_library::Bundled).await;
    assert!(
        unexpanded.is_empty(),
        "{label} must include only files beside it: {unexpanded:#?}"
    );
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

/// The policy a field's declaration reads under: its `select`, or the
/// one its `cardinality` implies in the older spelling (`many` is
/// `all`, `one` and an unsaid cardinality are `last`). A built-in
/// serializes only a `select` that says more than the default, so a
/// query written in either spelling compares by policy.
fn policy_of(spec: &serde_json::Value) -> &str {
    if let Some(select) = spec.get("select").and_then(serde_json::Value::as_str) {
        return select;
    }
    match spec.get("cardinality").and_then(serde_json::Value::as_str) {
        Some("many") => "all",
        _ => "last",
    }
}

fn css_rule<'a>(document: &'a str, selector: &str) -> &'a str {
    document
        .split(selector)
        .nth(1)
        .and_then(|css| css.split('}').next())
        .unwrap_or_else(|| panic!("profile library must contain the `{selector}` rule"))
}

#[dialog_common::test]
async fn it_lowers_the_standard_library() {
    assert_library_lowers(
        "standard library (core.yaml)",
        "core.yaml",
        STANDARD_LIBRARY,
    )
    .await;
}

/// The meta library describes a real shape, not an aspirational one.
///
/// It is documentation that has to stay true: every attribute it names
/// is one the worker actually writes to a `meta` branch, so lowering it
/// is what keeps the description from drifting from the rows.
#[dialog_common::test]
async fn it_lowers_the_meta_library() {
    assert_library_lowers("meta library (meta.yaml)", "meta.yaml", META_LIBRARY).await;
}

#[dialog_common::test]
async fn it_lowers_the_profile_library() {
    assert_library_lowers(
        "profile library (profile.yaml)",
        "profile.yaml",
        PROFILE_LIBRARY,
    )
    .await;
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
            PROFILE_LIBRARY.contains(&attribute)
                || PROFILE_LIBRARY.contains(&attribute_last)
                || PROFILE_LIBRARY.contains(&format!("{handle}\n")),
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
    assert!(
        PROFILE_LIBRARY.contains("element!: &hub-menu"),
        "the menu's behaviour must be defined as branch data",
    );
    assert!(
        PROFILE_LIBRARY.contains("<hub-menu"),
        "and the account menu must actually be one",
    );
}

/// The menu's methods do not shadow a view's `show`.
///
/// One word meaning a dictionary of templates in one half of the file
/// and "reveal the menu" in the other is a trap for whoever reads it
/// next.
#[dialog_common::test]
fn it_names_the_menu_methods_open_and_close() {
    let menu = PROFILE_LIBRARY
        .split("element!: &hub-menu")
        .nth(1)
        .map(|rest| rest.split("\nelement!:").next().unwrap_or(rest))
        .and_then(|rest| rest.split("\nview!:").next())
        .expect("the hub-menu definition");
    assert!(menu.contains("    open: |"), "the menu opens with `open`");
    assert!(menu.contains("    close: |"), "and closes with `close`");
    assert!(
        menu.contains("event.target?.closest?.('tonk-dialog')?.open"),
        "an open nested modal must own Escape before its menu ancestor",
    );
    assert!(
        !menu.contains("    show: |") && !menu.contains("    hide: |"),
        "`show` belongs to views; the menu must not borrow it",
    );
}

/// The account bar renders its name from facts.
///
/// The bar used to be markup an element painted from a fetch, which is
/// why "a signup is up" had to live on the document body: the route view
/// re-renders whenever profile facts land, replacing the element
/// mid-ceremony. Rendering from facts is what removes that problem
/// rather than working around it.
#[dialog_common::test]
fn it_renders_the_account_bar_from_facts() {
    assert!(
        PROFILE_LIBRARY.contains(r#"model="tonk:account/name""#),
        "the account bar must render `tonk:account/name` as a display, not paint it",
    );
    assert!(
        PROFILE_LIBRARY.contains("xyz.tonk.ceremony/state"),
        "ceremony progress must be a fact the bar can read, not element state",
    );
}

/// Keep the profile command schema compatible without exposing account
/// switching or profile creation in settings.
#[dialog_common::test]
fn it_retains_the_profile_command_without_a_settings_switcher() {
    assert!(PROFILE_LIBRARY.contains("xyz.tonk.command.add-profile/time"));
    assert!(!PROFILE_LIBRARY.contains("on:add-profile=tonk:add-profile"));
    assert!(!PROFILE_LIBRARY.contains("class=\"settings-panel switch-panel\""));
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
/// The space chrome takes a branch that does not carry `tonk:repository`
/// for a space this device has not replicated, and hides the site behind
/// the absent-space panel. The profile's own repository is addressed as
/// a space by its DID and its branch carries this library, so the
/// concept has to be declared here or `/space/<profile did>` answers
/// "open this space" over a repository that loaded.
#[dialog_common::test]
fn it_declares_the_repository_concept_the_space_chrome_resolves() {
    let chrome = "<tonk-display with=\"{space-branch}@{space-repo}\" entity={space-repo} model=tonk:repository view=title>";
    assert!(
        PROFILE_LIBRARY.contains(chrome),
        "the space chrome decides a space is here by resolving `tonk:repository`"
    );
    let declared = PROFILE_LIBRARY.split("\nconcept!:").any(|concept| {
        let head = concept.split("\nview!:").next().unwrap_or_default();
        head.contains("\n  this: tonk:repository\n") && head.contains("the: xyz.tonk.repo/name")
    });
    assert!(
        declared,
        "the profile library must declare `tonk:repository` over `xyz.tonk.repo/name`, \
         or its own repository reads as a space that has not arrived"
    );
    assert!(
        PROFILE_LIBRARY.contains("view!:\n  this: tonk:repository\n  show:\n    title: |"),
        "the chrome asks for the `title` view; without it the display reports `no-view`"
    );
}

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
        PROFILE_LIBRARY.contains("space={space-repo}"),
        "the FAB space binding must be resolved by the renderer"
    );
    assert!(
        !PROFILE_LIBRARY.contains("space=\"{space-repo}\""),
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
        "<tonk-dialog appearance=\"hub\" data-space-remove-dialog",
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

/// The body of the `<tonk-display>` opened by `opening`, up to the close
/// that matches it rather than the first close of a display nested in it.
fn display_body<'a>(source: &'a str, opening: &str) -> Option<&'a str> {
    let body = source.split_once(opening)?.1;
    let mut depth = 1usize;
    let mut at = 0;
    while depth > 0 {
        let rest = &body[at..];
        let open = rest.find("<tonk-display");
        let close = rest.find("</tonk-display>")?;
        match open {
            Some(open) if open < close => {
                depth += 1;
                at += open + "<tonk-display".len();
            }
            _ => {
                depth -= 1;
                if depth == 0 {
                    return Some(&body[..at + close]);
                }
                at += close + "</tonk-display>".len();
            }
        }
    }
    None
}

#[dialog_common::test]
fn it_hides_space_absence_slots_before_display_initialization() {
    let chrome = display_body(
        PROFILE_LIBRARY,
        "<tonk-display with=\"{space-branch}@{space-repo}\" entity={space-repo} model=tonk:repository view=title>",
    )
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
    let directory_probe = display_body(
        PROFILE_LIBRARY,
        "<tonk-display with=\"{profile-branch}@profile:tonk\" entity={space-repo} model=space view=downloading>",
    )
    .expect("absent-space chrome must consult the profile directory");

    for state in ["no-model", "no-entity"] {
        assert!(
            directory_probe.contains(&format!(r#"slot="{state}" hidden"#)),
            "an absent profile directory row must render recovery for `{state}`"
        );
    }
    assert_eq!(
        directory_probe.matches("New to this space?").count(),
        2,
        "both absence states must explain how to obtain an invite link"
    );
    assert_eq!(
        directory_probe
            .matches("from <strong>share</strong>")
            .count(),
        2,
        "both absence states must say where an invite link comes from"
    );
    assert_eq!(
        directory_probe
            .matches("<space-login data-signed-out-only>")
            .count(),
        2,
        "both absence states must offer sign-in recovery while signed out"
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
        // What a visit to a space that keeps no public ticket ends on.
        "this space is private",
        "class=\"space-unknown-back\" href=\"/\">go to home",
        "<space-login data-signed-out-only><button type=\"button\"",
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
        1,
        "the absent-space panel must explain downloading"
    );
    // The two absence walls are task pages (gooey in-browser/PAGES.md):
    // a sheet, the login and invite recovery, and a way on.
    assert_eq!(
        PROFILE_LIBRARY
            .matches("<div class=\"fabb-page space-private\">")
            .count(),
        2,
        "both absence states must be task-page walls"
    );
    assert_eq!(
        PROFILE_LIBRARY
            .matches("<div class=\"fabb-actions\"><a href=\"/\">your spaces</a><space-login data-signed-out-only><button type=\"button\">sign in</button></space-login></div>")
            .count(),
        2,
        "both absence walls must offer your spaces and sign in"
    );
    // Signed in, a wall does not ask to sign in: each reads the hub's
    // own account-link marker, and the sign-in sentence and button go.
    assert_eq!(
        PROFILE_LIBRARY
            .matches("<tonk-display with=\"{profile-branch}@profile:tonk\" model=\"state:account-link\" view=\"label\" hidden>")
            .count(),
        2,
        "both absence walls must know whether this device holds the account"
    );
    assert_eq!(
        PROFILE_LIBRARY
            .matches("<p data-signed-out-only>Already use this space? Sign in with your account.")
            .count(),
        2
    );
    assert!(PROFILE_LIBRARY.contains(
        ".space-unknown .space-private:has([data-account-linked]) [data-signed-out-only] { display:none; }"
    ));
    // A ticket link's join runs on a task page too: its failure wall sits
    // under the logotype on the page's ground, not bare in the panel.
    let join_page = PROFILE_LIBRARY
        .split("<div class=\"fabb-page space-join\">")
        .nth(1)
        .and_then(|tail| tail.split("</div>").next())
        .expect("the space route's join sits in a task page");
    assert!(join_page.contains("<header class=\"fabb-top\"><svg class=\"fabb-logo\""));
    assert!(
        join_page.contains("<tonk-display class=\"join-status-slot\" model=\"tonk:join/status\">")
    );
    for contract in [
        ".space-unknown .space-join:not(:has(.join-status)) { display:none; }",
        ".space-unknown:has(.space-join .join-status) { padding:0; }",
        ".space-unknown .space-join :is(tonk-display, tonk-view, .join-card-wrap, .join-status) { display:contents; }",
    ] {
        assert!(PROFILE_LIBRARY.contains(contract), "missing `{contract}`");
    }
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
    // The hub page is `.tonk-hub`: its colours, type and sizes are Tonk's
    // look, the `--tonk-*` names `gooey/tonk-look.css` sets, which the
    // vendored `gooey/in-browser/hub.css` reads. Both travel with the view
    // as its own styles, included from the library directory.
    for contract in [
        "look: !include/text ./gooey/tonk-look.css",
        "hub: !include/text ./gooey/in-browser/hub.css",
        // Hidden until its stylesheets load, so it never flashes unstyled.
        "<div class=\"hub-page tonk-hub\" style=\"visibility:hidden\" data-styling>",
        ".hub-page[data-styling] { animation:hub-styled 0s linear 2s forwards; }",
        "<link rel=\"stylesheet\" with:src=\"look@space\">",
        "<link rel=\"stylesheet\" with:src=\"hub@space\">",
    ] {
        assert!(
            PROFILE_LIBRARY.contains(contract),
            "the Hub must draw from Tonk's look: missing `{contract}`",
        );
    }
    for (file, text) in [
        ("tonk-look.css", GOOEY_LOOK),
        ("in-browser/hub.css", GOOEY_HUB),
    ] {
        assert!(
            text.starts_with(&format!(
                "/* Vendored from tonk-labs/gooey {file} at ready-to-tonk "
            )),
            "gooey/{file} must name the gooey revision it was copied from",
        );
    }
    assert!(GOOEY_LOOK.contains(".tonk-hub { color-scheme: light dark; }"));
    assert!(GOOEY_HUB.contains(".hub-line { position: sticky;"));

    // The parts of the page the mocks have not reached yet keep their
    // older palette. Colors live in ONE place — the token block at the top of the hub's
    // own `style: ui`, which travels with the view. The hub must CONSUME
    // those tokens rather than restating a color at each use; a raw hex in
    // a rule is the drift this contract exists to prevent.
    //
    // The tokens are LITERAL by design (the fabb wireframes' values), not
    // aliases over a component library's theme: that is what lets the page
    // carry its own colors instead of depending on a stylesheet the host
    // injects into every guest.
    for consumed in ["color:var(--ink)", "var(--frost-solid)", "var(--wash-p)"] {
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
fn it_builds_a_responsive_hub_collection_with_a_settings_route() {
    for contract in [
        "<hub-bar class=\"hub-top\">",
        "<svg class=\"hub-logo\"",
        // Your name is a toggle: a chevron, and aria-expanded on the sheet.
        "<a class=\"hub-me account-trigger\" href=\"/settings\" data-account-trigger role=\"button\"",
        "aria-expanded=\"false\" aria-controls=\"account-sheet\">",
        "<section class=\"hub-sheet\" data-spaces-sheet aria-label=\"your spaces\">",
        "<div class=\"hub-line\">",
        "<h2 class=\"hub-line-name\" tabindex=\"-1\" data-spaces-line>spaces <span class=\"hub-count\" data-space-count></span></h2>",
        "<button class=\"hub-btn hub-btn--main\" type=\"button\" data-new-space data-space-create-open>new space</button>",
        "<hub-collection class=\"hub-rows\"",
        "<div class=\"hub-row space-card\"",
        "<a class=\"hub-open\" href=\"/space/{subject}\">",
        "<span class=\"hub-name\" title=\"{name}\">{name}</span>",
        "with=\"main@profile:tonk\"",
        "<nav class=\"mobile-nav\"",
    ] {
        assert!(
            PROFILE_LIBRARY.contains(contract),
            "the responsive Hub collection must contain `{contract}`",
        );
    }
    assert!(
        PROFILE_LIBRARY.contains("create new space"),
        "the responsive Hub collection must contain `create new space`",
    );
    // A row is a line in the sheet now, not a card with a picture.
    for rejected in [
        "class=\"space-preview\"",
        "class=\"space-description\"",
        "<hub-collection class=\"stack",
    ] {
        assert!(
            !PROFILE_LIBRARY.contains(rejected),
            "the Hub's rows must reject `{rejected}`",
        );
    }
    let rejected = "class=\"shead";
    assert!(
        !PROFILE_LIBRARY.contains(rejected),
        "the responsive Hub collection must reject `{rejected}`",
    );
    // An empty list says so inside the sheet in one short, quiet line, so
    // the sheet does not end at its line (Jack's call: a list still arriving
    // after sign-in shows it for a moment). It is the display's `empty`
    // slot, so it shows only once the list has answered.
    assert!(
        PROFILE_LIBRARY
            .contains("<div slot=\"empty\" class=\"hub-empty\"><p>No spaces yet.</p></div>"),
        "the empty Hub must say it has no spaces, in the sheet",
    );
    // The one place that may say so is the wall on a settings task page
    // (an app or a terminal asking for the account), where it is true.
    let wall_start = PROFILE_LIBRARY
        .find("<div class=\"fabb-page signed-out-wall\"")
        .expect("the settings task pages' wall");
    let wall_end = wall_start
        + PROFILE_LIBRARY[wall_start..]
            .find("</section>")
            .expect("the wall's end");
    let hub = format!(
        "{}{}",
        &PROFILE_LIBRARY[..wall_start],
        &PROFILE_LIBRARY[wall_end..]
    )
    .to_lowercase();
    for rejected in ["signed out", "no spaces available"] {
        assert!(
            !hub.contains(rejected),
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
        "<space-create",
        "data-space-create-open",
        "description: \".currentTarget.elements.description.value\"",
        "the: xyz.tonk.command.create-space/description",
        "form=\"create-space-mobile\"",
    ] {
        assert!(
            PROFILE_LIBRARY.contains(contract),
            "provider-free Hub access must preserve `{contract}`",
        );
    }
    assert!(
        !PROFILE_LIBRARY.contains("html:form=\"create-space"),
        "fixed create-form IDs must use the native form-owner attribute",
    );
}

#[dialog_common::test]
fn it_deals_stable_weighted_frames_from_space_identity() {
    let collection = PROFILE_LIBRARY
        .split("element!: &hub-collection")
        .nth(1)
        .and_then(|rest| rest.split("\n\n# The hub bar").next())
        .expect("the hub collection element");
    for contract in [
        "self.hash(subject)",
        "data-space-subject",
        "const stepDown = { 15: 7, 7: 3, 3: 1, 1: 1 }",
        "--fr-r-mobile",
        "attributeFilter: ['data-space-subject', 'data-space-thumbnail', 'data-sync-case']",
    ] {
        assert!(
            collection.contains(contract),
            "stable weighted frames must preserve `{contract}`",
        );
    }
    for rejected in ["Math.random", "dataset.index", "childElementCount"] {
        assert!(
            !collection.contains(rejected),
            "frame identity must not depend on `{rejected}`",
        );
    }
}

#[dialog_common::test]
fn it_copies_a_hub_spaces_address_without_minting() {
    assert!(
        PROFILE_LIBRARY.contains(
            r#"<button type="button" role="menuitem" data-space-copy-link data-space={subject}>"#
        ),
        "the Hub copy action is a plain menu item that names its space"
    );
    assert!(
        !PROFILE_LIBRARY.contains("<tonk-share space={subject}>"),
        "the Hub menu must not mint an invite to copy: that waits on a link"
    );
    let actions = PROFILE_LIBRARY
        .split("element!: &space-actions")
        .nth(1)
        .expect("space-actions element");
    assert!(
        actions.contains("const url = `${origin}/space/${button.dataset.space}`;"),
        "the copy action copies the space's own address"
    );
    for (state, label) in [("copied", "copied"), ("failed", "failed")] {
        assert!(
            PROFILE_LIBRARY.contains(&format!(
                "[data-space-copy-link][data-copy=\"{state}\"] [data-copy-label=\"{label}\"]"
            )),
            "the Hub copy action must display its `{label}` answer"
        );
    }
}

#[dialog_common::test]
fn it_answers_a_resend_activation_request_in_place() {
    for contract in [
        "<resend-activation><button type=\"button\" data-resend-activation>",
        "data-resend-status role=\"status\"",
        "element!: &resend-activation",
        "the: 'xyz.tonk.resend-activation/at', as: 'natural:64'",
        "the: 'xyz.tonk.resend-activation/status'",
        "result?.status === 'requested'",
        "result?.status === 'failed'",
    ] {
        assert!(
            PROFILE_LIBRARY.contains(contract),
            "the activation notice must show what came of a resend: `{contract}`"
        );
    }
}

#[dialog_common::test]
fn it_renames_a_space_from_the_hub_menu() {
    for contract in [
        "data-space={subject}",
        "the: xyz.tonk.command.rename-repository/name",
        "the: xyz.tonk.rename-repository/space",
        "data-space-rename-open",
        // The submit dispatches under its own receipt and closes the
        // dialog only on the worker's answer for that receipt.
        "the: 'xyz.tonk.command.rename-repository/name'",
        "the: 'xyz.tonk.rename-repository/space'",
        "the: 'xyz.tonk.repository-rename/status'",
        "result?.status === 'renamed'",
    ] {
        assert!(
            PROFILE_LIBRARY.contains(contract),
            "the Hub rename path must preserve `{contract}`"
        );
    }
}

#[dialog_common::test]
fn it_aligns_the_hub_space_actions_in_one_flex_context() {
    assert!(
        css_rule(PROFILE_LIBRARY, ".space-menu space-remove {").contains("display:contents"),
        "authority-aware removal must remain a direct row in the card menu"
    );
    assert!(
        PROFILE_LIBRARY.contains("data-space-actions-open aria-haspopup=\"menu\""),
        "the card menu opener must expose its menu semantics"
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
    assert!(PROFILE_LIBRARY.contains("<hub-bar class=\"hub-top\">"));
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
    assert!(PROFILE_LIBRARY.contains("on:add-passkey=tonk:add-passkey"));
    assert!(panel.contains("on:authorize-device=tonk:authorize-device"));
    assert!(!panel.contains("/api/"));
    // The account is a sheet: Tonk's line with `close`, then its sections,
    // each with its action on the heading line. What can't be undone asks
    // first, in place.
    for contract in [
        "<section class=\"hub-sheet\" id=\"account-sheet\" data-account-sheet aria-label=\"account settings\">",
        // The sheet's sections sit in the hub's 16px gutter.
        "<div class=\"hub-account\">",
        // A change that didn't land says so in the line, with try again.
        "<button class=\"hub-btn\" type=\"button\" data-account-retry hidden>try again</button>",
        "<h2 class=\"hub-line-name\" tabindex=\"-1\" data-account-line>account settings</h2>",
        "<a class=\"hub-btn\" href=\"/\" data-return-spaces data-account-close>close</a>",
        "<h3 id=\"sec-conn\">connections</h3>",
        "<h3 id=\"sec-device\">this device</h3><button class=\"hub-btn\" type=\"button\" data-sign-out-open>sign out on this device</button>",
        "<p>Sign out on this device? Your spaces stay in your account.</p>",
        "<button class=\"hub-btn\" type=\"button\" data-ask-keep>stay signed in</button><button class=\"hub-btn hub-btn--ring\" type=\"button\" data-sign-out-submit on:sign-out=tonk:sign-out>sign out</button>",
        "data-agent-connections hidden",
        // Connections load on opening; there is no refresh button, and
        // with none the note says how one connects.
        "data-connections-note",
        r#"<tonk-display model="tonk:account/registered" view="deletion">"#,
    ] {
        assert!(
            panel.contains(contract),
            "the account sheet must carry `{contract}`"
        );
    }
    assert!(!panel.contains("data-sign-out-dialog"));
    assert!(!panel.contains("data-connections-refresh"));
    assert!(!panel.contains("remove this device"));
    assert!(!panel.contains("confirm device removal"));
    assert!(PROFILE_LIBRARY.contains("data-add-passkey"));
    assert!(!panel.contains("switch-panel"));
    assert!(!panel.contains("data-add-profile"));
    assert_eq!(panel.matches("href=\"/settings\"").count(), 0);
    // The name, address and passkeys are facts, so the panel mounts the
    // view that renders them rather than carrying their markup; the
    // ceremony's progress is a row it words.
    assert!(
        panel.contains(r#"<tonk-display model="tonk:account/registered" view="settings">"#),
        "the account pane must render the registration facts, not paint them",
    );
    assert!(panel.contains(
        r#"<tonk-display model="state:ceremony" entity="state:ceremony" view="settings">"#
    ));

    // Deleting stands apart, on the registration view, because the
    // command carries the verified address and that view has it. It asks
    // first in place; the answer is `delete with passkey`.
    let deletion = PROFILE_LIBRARY
        .split("    deletion: |\n      <section class=\"hub-sec hub-apart\"")
        .nth(1)
        .and_then(|rest| rest.split("\n\n").next())
        .expect("the registration deletion facet");
    assert!(deletion.contains("data-delete-account-submit data-email={email}"));
    assert!(deletion.contains("on:delete-account=tonk:delete-account>"));
    assert!(deletion.contains("<p>Delete your account? This can’t be undone.</p>"));
    assert!(deletion.contains("data-ask-keep>keep account</button>"));
    assert!(deletion.contains("<span data-delete-submit-label>delete with passkey</span>"));
    assert!(
        deletion.contains(r#"<tonk-display model="tonk:space/owned" view="deletion">"#),
        "what the deletion deletes is counted from the owned-space facts",
    );
    assert!(!deletion.contains("data-delete-confirm"));
    // The name saves when you leave the field: no save button.
    let name_form = PROFILE_LIBRARY
        .split("<form class=\"hub-kv\" data-profile-rename-form")
        .nth(1)
        .and_then(|rest| rest.split("</form>").next())
        .expect("the display-name form");
    assert!(
        !name_form.contains("on:profile-rename-submit"),
        "the receipt-aware submit handler must own dispatch",
    );
    for contract in [
        "name=\"name\" type=\"text\"",
        "maxlength=\"50\" autocomplete=\"name\"",
    ] {
        assert!(
            name_form.contains(contract),
            "the display-name form must preserve `{contract}`",
        );
    }
    assert!(!name_form.contains("save changes"));
    assert!(
        PROFILE_LIBRARY.contains("if (error) error.textContent = 'Your name can’t be empty.';")
    );
    assert!(PROFILE_LIBRARY.contains("self.receipt(['changed your name to', name]);"));
    assert!(
        PROFILE_LIBRARY
            .contains("'This is your only passkey, so it can’t be removed. Add another first.'")
    );
}

#[dialog_common::test]
fn it_keeps_machine_instructions_in_the_production_copy_prompt() {
    let library = STANDARD_LIBRARY;
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

#[test]
fn it_keeps_ready_agent_invites_to_one_primary_action() {
    let library = STANDARD_LIBRARY;
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
    // The mock's #link-off: a revoked link reads as turned off, not
    // expired (nothing records an expiry).
    assert!(failure.contains("heading=\"this passlink was turned off\""));
    assert!(failure.contains("so it can’t be used to join. Ask them for a new one."));
    assert!(!failure.contains("link expired"));
    // A wall is never a dead end for someone signed in (PAGES.md).
    assert!(failure.contains("<a href=\"/\">your spaces</a>"));
    assert!(!failure.contains("edge-field edge-field--settled"));
    assert!(!failure.contains("paste a new link"));
    assert!(!failure.contains("tonk-join-retry"));
    assert!(!failure.contains("join this space"));
    assert!(!failure.contains("start a new space"));
    // The only form is the bare /join page's paste field, which joins
    // through the same command the page's own location does; the
    // failure wall above stays a closed wall with no field of its own.
    assert_eq!(route.matches("<form").count(), 1);
    assert_eq!(route.matches("<input").count(), 1);
    let paste = route
        .split("<join-paste")
        .nth(1)
        .and_then(|tail| tail.split("</join-paste>").next())
        .expect("the paste form lives in <join-paste>");
    assert!(paste.contains("<form") && paste.contains("<input"));
    assert!(route.contains("<join-paste class=\"join-paste\" on:join=tonk:join>"));
    assert!(
        route.contains(".join-view:not([data-path=\"/join\"]) .join-paste-task { display:none; }")
    );
    assert!(!route.contains("tonk-invite-link"));
    assert!(route.contains("<page-mount on:join=tonk:join>"));
    assert_eq!(failure.matches("class=\"fabb-actions\"").count(), 1);
}

#[dialog_common::test]
fn it_keeps_join_failure_chrome_and_actions_visually_consistent() {
    let route = PROFILE_LIBRARY
        .split("view!:\n  this: tonk:join/route")
        .nth(1)
        .and_then(|tail| tail.split("# The /inspector and /diagnose routes").next())
        .expect("join route view");

    // A task page (gooey in-browser/PAGES.md): the vendored parts draw
    // it, so the view sets no colour, size or font of its own.
    assert!(route.contains("<main class=\"join-view fabb-page\""));
    assert!(route.contains("<section class=\"fabb-task join-paste-task\""));
    assert!(!route.contains("--edge-"), "no tokens of the view's own");
    // The top is the logotype alone: a task page has no navigation
    // (PAGES.md), so the mark is not a link home.
    assert!(route.contains("<header class=\"fabb-top\"><svg class=\"fabb-logo\""));
    assert!(
        !PROFILE_LIBRARY.contains("<header class=\"fabb-top\"><a"),
        "no task page's logotype is a link",
    );
    assert!(
        route.contains(".join-status:has(.join-closed) .join-opening { display:none; }"),
        "retained failure content must suppress the opening row even while its display reconnects",
    );

    let opening = css_rule(route, ".join-opening {");
    for contract in [
        "position:fixed",
        "inset:0",
        "align-items:center",
        "justify-content:center",
        "background:var(--tonk-ground)",
    ] {
        assert!(
            opening.contains(contract),
            "the pending join must match the centred boot pulse with `{contract}`",
        );
    }
    assert!(PROFILE_LIBRARY.contains("class=\"join-opening-status\""));
    assert!(PROFILE_LIBRARY.contains("class=\"tonk-pulse\""));
    assert!(route.contains(
        ".join-view:not([data-path=\"/join\"]):has(.join-status-slot[data-state=\"ready\"]):not(:has(.join-closed))"
    ));

    // The answers are the hub's buttons, at the right edge.
    assert!(route.contains("<div class=\"fabb-actions\"><a href=\"/\">your spaces</a><button type=\"submit\" form=\"join-paste-form\">join</button></div>"));
}

#[dialog_common::test]
fn it_sizes_the_join_route_to_the_dynamic_mobile_viewport() {
    let route = PROFILE_LIBRARY
        .split("view!:\n  this: tonk:join/route")
        .nth(1)
        .and_then(|tail| tail.split("# The /inspector and /diagnose routes").next())
        .expect("join route view");
    // The join route is a task page: gooey's sheet sizes it, the legacy
    // viewport first and the dynamic one after it.
    assert!(route.contains("class=\"join-view fabb-page\""));
    let page = css_rule(TASK_PAGES_CSS, ".fabb-page {");
    let fallback = page
        .find("min-height: 100vh")
        .expect("legacy viewport fallback");
    let dynamic = page
        .find("min-height: 100dvh")
        .expect("dynamic mobile viewport");
    assert!(
        fallback < dynamic,
        "the dynamic viewport declaration must follow and override the fallback"
    );
}

#[dialog_common::test]
fn it_declares_mobile_target_and_input_floors_for_hub_and_join() {
    for contract in [
        ".mobile-nav > a, .mobile-nav > space-create { min-width:0; min-height:54px; }",
        ".mobile-nav > a, .mobile-nav .mobile-create { display:flex; width:100%; min-height:54px;",
        ".account-menu__row { min-height:44px; }",
    ] {
        assert!(
            PROFILE_LIBRARY.contains(contract),
            "mobile Hub CSS must contain `{contract}`"
        );
    }
    // The join route's answers are a task page's buttons: 44px to see on a
    // touch screen.
    assert!(
        TASK_FORMS_CSS.contains("@media (pointer:coarse){.fabb-actions :is(button,a){height:44px}")
    );
}

/// Lower a component library the way the seed reaches it: on top of
/// the standard library, whose `view` / `event` / `command` concepts it
/// references. `analyze_local` resolves names within one document, so
/// the concatenation is what stands in for "core is already seeded".
async fn assert_component_library_lowers(label: &str, file: &str, document: &str) {
    assert_library_lowers(label, file, &format!("{STANDARD_LIBRARY}\n{document}")).await;
}

#[dialog_common::test]
async fn it_lowers_the_table_library() {
    assert_component_library_lowers("table library (table.yaml)", "table.yaml", TABLE_LIBRARY)
        .await;
}

#[dialog_common::test]
async fn it_lowers_the_notebook_library() {
    assert_component_library_lowers(
        "notebook library (notebook.yaml)",
        "notebook.yaml",
        NOTEBOOK_LIBRARY,
    )
    .await;
}

#[dialog_common::test]
async fn it_lowers_the_code_editor_library() {
    assert_component_library_lowers(
        "code-editor library (code-editor.yaml)",
        "code-editor.yaml",
        CODE_EDITOR_LIBRARY,
    )
    .await;
}

/// A language is a component of its own, installable without the editor,
/// so it lowers on the standard library alone. One of each shape the
/// build generates: a Lezer grammar, a legacy stream mode, a SQL dialect.
#[dialog_common::test]
async fn it_lowers_language_components() {
    for (file, document) in [
        (
            "language/python.yaml",
            include_str!("../../tonk-core/assets/library/language/python.yaml"),
        ),
        (
            "language/clojure.yaml",
            include_str!("../../tonk-core/assets/library/language/clojure.yaml"),
        ),
        (
            "language/postgresql.yaml",
            include_str!("../../tonk-core/assets/library/language/postgresql.yaml"),
        ),
    ] {
        // Parsed at the library directory, as seeding parses every library
        // document, so its include (`./language/<id>.js`) resolves there.
        assert_component_library_lowers(file, "language.yaml", document).await;
    }
}

#[dialog_common::test]
async fn it_lowers_the_prose_editor_library() {
    assert_component_library_lowers(
        "prose-editor library (prose-editor.yaml)",
        "prose-editor.yaml",
        PROSE_LIBRARY,
    )
    .await;
}

#[dialog_common::test]
async fn it_lowers_the_issue_library() {
    assert_component_library_lowers("issue library (issue.yaml)", "issue.yaml", ISSUE_LIBRARY)
        .await;
}

#[dialog_common::test]
async fn it_lowers_the_version_control_library() {
    assert_component_library_lowers(
        "version-control library (version-control.yaml)",
        "version-control.yaml",
        VERSION_CONTROL_LIBRARY,
    )
    .await;
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
async fn an_event_declaration_lowers_without_the_library() {
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
      cardinality: :one
  maybe:
    time:
      description: A per-event nonce
      the: io.gozala.bump/time
      as: float
      cardinality: :one
"#;
    assert_library_lowers("a bare `event!:` document", "event.yaml", document).await;
}

/// The optional side-effect flags really are optional: a declaration
/// that suppresses neither must lower. Every declaration in the
/// library omits them, so a regression here would break all of them at
/// once.
#[dialog_common::test]
async fn an_event_declaration_may_omit_the_side_effect_flags() {
    let document = r#"event!: &on/plain
  type: "click"
  where:
    subject: "{this}"
"#;
    assert_library_lowers("an `event!:` with no flags", "event.yaml", document).await;
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
            policy_of(ours),
            policy_of(theirs),
            "`{field}`: the query and the built-in disagree on the select policy",
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
    let after = STANDARD_LIBRARY
        .split("concept!: &element\n")
        .nth(1)
        .expect("the library declares `element`");
    // The declaration ends at the next head: the first line at column
    // zero that is not a comment.
    let declared: String = after
        .lines()
        .take_while(|line| line.is_empty() || line.starts_with(' ') || line.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    // Built from the constant, not written out again: a literal here
    // would match the library whatever the constant said, and the test
    // would pass through exactly the drift it exists to catch.
    let domain = tonk_template::resolve::ELEMENT_METHOD_DOMAIN;
    assert!(
        declared.contains(&format!("the: \"{domain}\"")),
        "the wire predicate's domain ({domain}) is not what the library \
         declares for `element.method`",
    );
    assert!(
        declared.contains("as: {[symbol]: text}"),
        "the library declares `method` as something other than a keyed \
         dictionary of text",
    );
    // The predicate reads `method` under the default pick, `last`, which
    // the library spells by declaring no pick at all (the older
    // `cardinality: one` said the same). A `pick: :all` or a
    // `cardinality: many` on the element would be a set the predicate
    // does not mirror.
    assert!(
        !declared.contains("pick:") && !declared.contains("cardinality:"),
        "the library declares `element` with a pick the predicate does not \
         mirror: {declared}",
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
            policy_of(ours),
            policy_of(theirs),
            "`{field}`: the query and the built-in disagree on the select policy",
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
    // Named attributes (`attribute!: &name` … `  the: x`), so a field
    // that references one by name (`    member: expel-member/member`)
    // counts as carrying it, exactly as the analyzer resolves it.
    let mut named = std::collections::BTreeMap::new();
    let mut pending: Option<String> = None;
    for line in document.lines() {
        if let Some(name) = line.strip_prefix("attribute!: &") {
            pending = name.split_whitespace().next().map(str::to_owned);
        } else if let (Some(name), Some(the)) = (&pending, line.strip_prefix("  the: ")) {
            named.insert(name.clone(), the.trim().to_string());
            pending = None;
        } else if !line.starts_with(' ') && !line.trim().is_empty() {
            pending = None;
        }
    }

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
            // A field four spaces in whose value is a named attribute.
            if let Some(field) = body.strip_prefix("    ")
                && !field.starts_with(' ')
                && let Some((_, value)) = field.split_once(": ")
                && let Some(the) = named.get(value.trim())
            {
                attributes.insert(the.clone());
            }
        }
        out.insert(name, attributes);
    }
    out
}

#[test]
fn it_offers_only_scoped_agent_prompts_without_account_approval() {
    let library = STANDARD_LIBRARY;
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
    assert!(
        scoped.contains(
            "Only report connected after it prints &quot;Agent connection confirmed&quot;"
        )
    );
    assert!(scoped.contains("acknowledged receipt push"));
    assert!(scoped.contains("Multiple holders of this link share the same invitation authority"));
    assert!(scoped.contains("ask me for a fresh link"));
    assert!(!scoped.contains("join --agent"));
    assert!(!scoped.contains("--switch-account"));
    assert!(!scoped.contains("requires account {account}"));
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
    assert!(!receipt.contains("position: fixed"));
    assert!(!receipt.contains("role=\"status\""));
    let shell = STANDARD_LIBRARY
        .split("view!:\n  this: tonk:workspace/shell\n")
        .nth(1)
        .unwrap()
        .split("# The directory page")
        .next()
        .unwrap();
    assert!(
        !shell.contains("tonk:agent-connection"),
        "opening a space must not render shared receipts as notifications"
    );
    assert!(!receipt.contains("Your agent connected"));
    assert!(
        !STANDARD_LIBRARY
            .contains("entity=\"id:tonk:agent-connection\" model=tonk:agent-connection")
    );
}

/// `/seed/<url>` offers the New-space dialog with the URL carried as the
/// `seed` of the `space/create` it submits, and says where the definitions
/// come from before anything is created.
#[dialog_common::test]
fn it_offers_to_create_a_space_from_the_url_in_a_seed_link() {
    assert!(
        PROFILE_LIBRARY.contains(r#"path: "/seed/{*url}""#),
        "the profile must route /seed/<url>"
    );
    let view = PROFILE_LIBRARY
        .split("this: tonk:seed/route\n  show:")
        .nth(1)
        .expect("the seed route must have a view");
    assert!(
        view.contains(r#"<input type="hidden" name="seed" value="{url}">"#),
        "the dialog's form must carry the captured URL as its seed"
    );
    assert!(
        view.contains("<space-create autoopen>"),
        "the seed page must open its dialog on arrival"
    );
    assert!(
        view.contains("continue only if you trust where they come from"),
        "the dialog must warn that the definitions come from elsewhere"
    );
    assert!(
        PROFILE_LIBRARY.contains("parameters.seed = seed;")
            && PROFILE_LIBRARY.contains("the: 'xyz.tonk.command.create-space/seed'"),
        "<space-create> must forward a form's seed to space/create"
    );
}

/// A real seed link reaches the seed route, not the profile's catch-all,
/// with the source URL captured whole: its `//` and its own path intact.
#[dialog_common::test]
fn it_routes_a_seed_link_to_the_seed_page_with_the_url_intact() {
    let mut router = tonk_router::Router::new();
    for line in PROFILE_LIBRARY.lines() {
        let Some(path) = line
            .trim()
            .strip_prefix("path: \"")
            .and_then(|path| path.strip_suffix('"'))
        else {
            continue;
        };
        if let Ok(route) = tonk_router::Route::parse_pattern(path) {
            router.insert(route, path.to_owned());
        }
    }
    let matched = router
        .recognize("/seed/https://tonk.network/library/notebook.yaml")
        .expect("a seed link matches a route");
    assert_eq!(matched.value, "/seed/{*url}");
    assert_eq!(
        matched.params.get("url"),
        Some("https://tonk.network/library/notebook.yaml")
    );
}

/// A hub row draws the space's sync disc, derived on the profile branch
/// from facts this device holds: the space's upstream and this device's
/// pause preference. It shows no role or people count, since the profile
/// branch records neither and the hub shows only recorded facts.
#[dialog_common::test]
fn it_derives_each_hub_rows_sync_disc_and_no_unrecorded_role() {
    for contract in [
        "concept!: &space/sync",
        "concept!: &replica/sync-enabled",
        "the: xyz.tonk.space/sync",
        "the: xyz.tonk.sync/enabled",
        "description: A synced space whose replica is paused on this device is paused",
        "<span class=\"hub-disc\" role=\"img\" data-sync-case={sync}></span>",
    ] {
        assert!(
            PROFILE_LIBRARY.contains(contract),
            "the hub row must carry `{contract}`",
        );
    }
    for absent in [
        "concept!: &space/role",
        "data-role-case",
        "class=\"hub-meta\"",
    ] {
        assert!(
            !PROFILE_LIBRARY.contains(absent),
            "the hub row must not infer `{absent}`",
        );
    }
    // The cases rank: a paused space that has an upstream reads as paused,
    // and one nothing better speaks for reads as local.
    let sync = PROFILE_LIBRARY
        .split("concept!: &space/sync\n")
        .nth(1)
        .expect("the space/sync concept");
    let paused = sync.find("- case:paused").expect("paused case");
    let synced = sync.find("- case:synced").expect("synced case");
    let local = sync.find("- case:local").expect("local case");
    assert!(
        paused < synced && synced < local,
        "paused, then synced, then local"
    );
}

/// A hub row is a link that opens the space; a chevron beside it, a button
/// of its own, unfolds the space's details in place. Closed, the details
/// stay in the page, inert and out of the tab order.
#[dialog_common::test]
fn it_opens_a_hub_space_from_its_row_and_unfolds_details_with_a_chevron() {
    for contract in [
        "<a class=\"hub-open\" href=\"/space/{subject}\">",
        "<button type=\"button\" class=\"hub-toggle\" aria-expanded=\"false\" aria-controls=\"body-{subject}\"",
        "aria-label=\"links and details of {name}\"",
        "<div class=\"hub-body\" id=\"body-{subject}\" inert>",
    ] {
        assert!(
            PROFILE_LIBRARY.contains(contract),
            "the hub row must carry `{contract}`",
        );
    }
    // The address is not repeated in the open row: the row is its link.
    assert!(!PROFILE_LIBRARY.contains("data-space-address"));
    let collection = PROFILE_LIBRARY
        .split("element!: &hub-collection")
        .nth(1)
        .and_then(|rest| rest.split("\nelement!:").next())
        .expect("the hub collection element");
    for contract in [
        // one row open at a time
        "if (other !== row) self.fold(other, false);",
        // closed details leave the tab order
        "if (body) body.inert = !open;",
        // Escape folds the open row and hands focus back to its chevron
        "row.querySelector('.hub-toggle')?.focus();",
        // a keyboard press lands at once
        "page.setAttribute('data-still', '');",
    ] {
        assert!(
            collection.contains(contract),
            "the hub collection must keep `{contract}`",
        );
    }
}

/// While the spaces list loads, placeholder rows hold its place past
/// 400ms and Tonk's line says `loading`; when the service worker cannot
/// be reached before any row shows, the sheet says so with `try again`.
#[dialog_common::test]
fn it_shows_loading_rows_and_a_didnt_load_sheet_for_the_spaces_list() {
    for contract in [
        "<ul slot=\"loading\" class=\"hub-wait\" aria-hidden=\"true\">",
        "<div class=\"hub-skel\"><i></i><i></i></div>",
        ".hub-wait { list-style:none; margin:0; padding:0; animation:hub-wait-in 400ms step-end; }",
        "<div slot=\"offline\" class=\"hub-empty hub-failed\"><p>Your spaces didn’t load. Check your connection and try again.</p><button class=\"hub-btn hub-btn--main\" type=\"button\" data-spaces-retry>try again</button></div>",
        "tonk-display:has(.space-card) > .hub-failed { display:none; }",
        "count.textContent = list === 'loading' ? 'loading' : '';",
        "button.disabled = list === 'loading';",
        "if (event.target?.closest?.('[data-spaces-retry]')) { self.listRetry(); return; }",
    ] {
        assert!(
            PROFILE_LIBRARY.contains(contract),
            "the spaces sheet must carry `{contract}`",
        );
    }
    assert_eq!(
        PROFILE_LIBRARY
            .matches("<li class=\"hub-wait-row\"><div class=\"hub-skel\">")
            .count(),
        5,
        "five placeholder rows",
    );
}

/// A rename or a removal in the hub leaves a receipt in the spaces line,
/// in place of its name, and announces it. The hub follows the command to
/// its outcome, since a removed space's row leaves before `removed`.
#[dialog_common::test]
fn it_gives_a_receipt_in_the_spaces_line_after_a_change() {
    for contract in [
        "<span class=\"hub-receipt\" tabindex=\"-1\" data-hub-receipt hidden></span>",
        "<button class=\"hub-btn\" type=\"button\" data-hub-undo data-wait=\"undoing\" hidden>undo</button>",
        "<div class=\"hub-sr\" role=\"status\" aria-live=\"polite\" data-hub-live></div>",
        "document.createTextNode(' · by you')",
        "sheet.querySelector('[data-spaces-line]')?.classList.add('hub-sr');",
        "self.fail('Couldn’t undo that, so the change stays.', pending.run, 'undoing');",
        "self.closest('hub-discover')?.follow?.(id, 'repository-rename', 'renamed',",
        "self.closest('hub-discover')?.follow?.(id, 'space-removal', 'removed',",
        "[action === 'leave' ? 'left' : 'deleted', name]",
    ] {
        assert!(
            PROFILE_LIBRARY.contains(contract),
            "the spaces line must carry `{contract}`",
        );
    }
}

/// An open hub row lists the space's links from what the worker records,
/// with desktop's rules: the first open link is the default and resets in
/// place; any other link is removed. No link records a role or an expiry,
/// so the rows say neither.
#[dialog_common::test]
fn it_lists_a_spaces_links_in_its_open_hub_row() {
    for contract in [
        "element!: &space-links",
        "<space-links data-space={subject} data-name={name}></space-links>",
        "self.api(`${base}/invites`)",
        "member: field('xyz.tonk.membership/member', 'Entity'),",
        "self.api('/api/account/connections')",
        // The default is the oldest open link, whatever this tab made.
        "const first = rows.find((row) => row.kind === 'open');",
        "/invites/${encodeURIComponent(target)}/revoke",
        "/api/account/connections/${encodeURIComponent(target)}/revoke",
        "'keep the link', 'reset passlink', true]",
        "new ClipboardItem({ 'text/plain': pending.then(",
        "'No links yet. Make one to bring someone in.'",
    ] {
        assert!(
            PROFILE_LIBRARY.contains(contract),
            "the hub row's links must carry `{contract}`",
        );
    }
}

/// `new space` opens a row under the spaces line: a name, `make space` and
/// `not now`. An empty name and a name you already have are said under the
/// field; the hub stays, and the new space's row opens with its default
/// link made.
#[dialog_common::test]
fn it_makes_a_new_space_from_an_inline_row_under_the_line() {
    for contract in [
        "<space-create data-inline>",
        "<form class=\"hub-new\" data-space-create-form novalidate hidden>",
        "<label for=\"new-space-name\">name</label>",
        "<input type=\"hidden\" name=\"open\" value=\"false\">",
        "<button class=\"hub-btn\" type=\"button\" data-space-create-cancel>not now</button>",
        "data-space-create-submit data-wait=\"making\">make space</button>",
        "(inline ? 'Give the space a name.' : 'Enter a name for this space.')",
        ": `You already have a space called ${name.value}.`;",
        // `make space` keeps focus and says `making` only past 400ms.
        ".map((control) => hub?.waiting?.(control, 'making') ?? (() => {}));",
        // An error clears as the name is typed.
        "if (error?.textContent) error.textContent = '';",
        "self.closest('hub-discover')?.receipt?.(['made', parameters.name]);",
        "await links.makeDefault();",
        "self.querySelector('[data-link-copy]')?.focus();",
    ] {
        assert!(
            PROFILE_LIBRARY.contains(contract),
            "the inline new space row must carry `{contract}`",
        );
    }
}

#[dialog_common::test]
fn it_draws_a_missing_page_as_a_wall() {
    let view = PROFILE_LIBRARY
        .split("view!:\n  this: tonk:not-found")
        .nth(1)
        .and_then(|tail| tail.split("# ════").next())
        .expect("not-found view");
    // gooey in-browser/task-page.html#notfound: a whole-page status on a
    // task sheet, no disc, and `your spaces`. No Web Awesome, no tokens.
    assert!(view.contains("<main class=\"fabb-page nf\""));
    assert!(view.contains(">no page here</h1>"));
    assert!(view.contains(
        "<tonk-message kind=\"status\">There’s nothing at this address. Check the link, or ask whoever sent it.</tonk-message>"
    ));
    assert!(view.contains("<div class=\"fabb-actions\"><a href=\"/\">your spaces</a></div>"));
    for gone in ["<wa-button", "--wa-", "fabb-disc", "404"] {
        assert!(!view.contains(gone), "the wall must not draw `{gone}`");
    }
}

#[dialog_common::test]
fn it_shows_a_signed_out_wall_on_settings_task_pages() {
    // gooey in-browser/task-page.html#signed-out, on /settings/* only: a
    // wall whose sign in raises the account ceremony, in place of raising
    // it unasked. The account page and ?via keep raising theirs.
    assert!(PROFILE_LIBRARY.contains(
        "const signedOut = path.startsWith('/settings/') && self.unlinked()\n          && !self.viaRequest() && !self.viaAsk();"
    ));
    assert!(
        PROFILE_LIBRARY
            .contains("if (account && self.unlinked() && !self.viaRequest() && !signedOut) {")
    );
    assert!(
        PROFILE_LIBRARY.contains("<div class=\"fabb-page signed-out-wall\" data-signed-out-wall>")
    );
    assert!(PROFILE_LIBRARY.contains(">you’re signed out</h1>"));
    assert!(
        PROFILE_LIBRARY
            .contains("<tonk-message kind=\"status\">Sign in to see your spaces.</tonk-message>")
    );
    assert!(PROFILE_LIBRARY.contains(
        "} else if (hit('[data-signed-out-sign-in]')) {\n            self.link('needs-account');"
    ));
}

#[dialog_common::test]
fn it_draws_a_devices_request_as_the_connect_a_device_task() {
    // gooey in-browser/task-page.html #device, #device-connected,
    // #device-denied, #device-expired, #device-cancelled, #device-failed.
    let pane = PROFILE_LIBRARY
        .split("<div class=\"pane fabb-page\" data-pane=\"link\" hidden data-link-approval>")
        .nth(1)
        .and_then(|tail| {
            tail.split("<div class=\"pane fabb-page\" data-pane=\"local-link\"")
                .next()
        })
        .expect("the device task");
    assert!(pane.contains(">connect a device</h1>"));
    assert!(pane.contains("<dt>device</dt><dd data-link-name></dd>"));
    assert!(pane.contains("<dt>your account</dt>"));
    // the account reads by its name (the mock's "Alex"), the email only
    // while no name row has arrived
    assert!(pane.contains(
        "<dd data-link-account-cell>\n                          <tonk-display model=\"state:account-link\" view=\"label\">"
    ));
    assert!(PROFILE_LIBRARY.contains(
        "account-settings [data-link-account-cell]:has([data-account-name]) [data-link-account-email] { display:none; }"
    ));
    // the connected step names the device it connected
    assert!(pane.contains(
        "<span data-link-connected-name>The device</span> can now use your account. You can close this page."
    ));
    assert!(
        PROFILE_LIBRARY
            .contains("self.text('[data-link-connected-name]', request.name || 'The device');")
    );
    assert!(
        pane.contains("BUG-246"),
        "a slot for the code the device shows"
    );
    // the safe answer first, the main one last, naming how much it grants
    assert!(pane.contains(
        "<button type=\"button\" data-link-decline>don’t allow</button>\n                      <button type=\"button\" data-link-approve"
    ));
    assert!(pane.contains("data-wait=\"connecting\">allow whole account</button>"));
    for step in ["ask", "connected", "denied", "expired"] {
        assert!(
            pane.contains(&format!("data-link-step=\"{step}\"")),
            "{step}"
        );
    }
    // the terminal's answer comes back as ?link=ok|denied|invalid
    assert!(PROFILE_LIBRARY.contains(
        "return { ok: 'connected', denied: 'denied', invalid: 'denied' }[outcome] ?? null;"
    ));
    // a cancelled or failed passkey is the same step, the reason in the form
    assert!(PROFILE_LIBRARY.contains(
        "if (state === 'refused') self.linkError('The passkey prompt was cancelled. Try again when you’re ready.');"
    ));
    // a failure says what stayed, in the mock's words, without the
    // worker's detail
    assert!(PROFILE_LIBRARY.contains(
        "self.linkError('Tonk couldn’t reach the device, so nothing was connected. Check that it’s still waiting, then try again.');"
    ));
    // except a handoff for another account, whose detail names both
    assert!(PROFILE_LIBRARY.contains(
        "else if (state === 'failed' && /requires account/.test(detail)) self.linkError(`Tonk couldn’t finish, so nothing was connected."
    ));
    // the other two panes keep their copy on the same parts
    assert!(PROFILE_LIBRARY.contains(">attach a local space to this account</h1>"));
    assert!(PROFILE_LIBRARY.contains(">sign in through another tonk</h1>"));
}
