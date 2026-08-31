//! Reflect lifecycle phase onto the host as `data-state`, and
//! surface error-state messages as a visible `<wa-callout>` inside
//! the host so users see what went wrong without needing to wire
//! the `tonk-display:error` event.
//!
//! The [`State`] enum, its `as_str` mapping, and [`error_title`]
//! are target-independent so they can be unit-tested natively.
//! The `set` / `set_error` DOM functions live behind a `wasm32`
//! cfg gate further down.

use tonk_host::error::ErrorKind;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;
#[cfg(target_arch = "wasm32")]
use web_sys::{Element, window};

/// The lifecycle states authors can target from CSS via `data-state`.
///
/// Every non-`Ready` state stays subscribed: none is terminal. The
/// display reports where it is; an embedder can skin any state with a
/// light-DOM `slot="<state>"` child, which the display projects (showing
/// the matching one, hiding the rest — see [`update_slot_children`]). For
/// a state with no embedder slot, the display mounts a built-in fallback:
/// a neutral, informative callout for the recoverable absences
/// ([`State::NoModel`] / [`State::NoView`] / [`State::NoEntity`], via
/// [`set_absence`]) and a loud danger callout for the broken states
/// ([`State::Malformed`] / [`State::Offline`] / [`State::Unauthorized`] /
/// [`State::Unknown`], via [`set_error`]). See
/// `plan/tonk-display-states.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// A resolve query is in flight; nothing known yet.
    Loading,
    /// Row(s) rendered by a model-specific view.
    Ready,
    /// No model-specific view defined; the built-in `_:_` fallback
    /// view is rendering. Rows render, but via the generic fallback.
    DefaultView,
    /// The `model` concept is not defined on the branch (yet). The
    /// model subscription stays open and recovers when it lands.
    NoModel,
    /// Model resolved; an explicit `view` was requested but is not
    /// defined and there is no `_:_` fallback to fall through to.
    NoView,
    /// Single mode: concept + view resolved, the entity **row** is
    /// absent (not synced yet / retracted). Stays subscribed.
    NoEntity,
    /// Directory mode: the collection has **zero instances**. A
    /// legitimate steady state (an empty repo), distinct from a missing
    /// single row. `<tonk-fallback>` keys its launchpad on this.
    Empty,
    /// A query returned 403 — no access to this repo/branch.
    Unauthorized,
    /// A query returned 404 — this device holds no such repository or
    /// branch. Terminal, unlike [`State::Offline`]: the transport gave
    /// its final answer, so nothing retries and nothing recovers on its
    /// own. Reached by landing on a space you have never joined.
    Unknown,
    /// Transport failure / the service worker is unreachable.
    Offline,
    /// Author/protocol error — a bad `model`/`entity` attribute or a
    /// decode failure. Recovers when the author fixes the attribute.
    Malformed,
}

impl State {
    /// The `data-state` attribute value for this state. Tests
    /// pin the mapping so CSS authors can rely on these strings.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            State::Loading => "loading",
            State::Ready => "ready",
            State::DefaultView => "default-view",
            State::NoModel => "no-model",
            State::NoView => "no-view",
            State::NoEntity => "no-entity",
            State::Empty => "empty",
            State::Unauthorized => "unauthorized",
            State::Unknown => "unknown",
            State::Offline => "offline",
            State::Malformed => "malformed",
        }
    }

    /// Whether this state is a *loud* failure — it drives a danger
    /// callout via [`set_error`]. The recoverable absences
    /// ([`State::NoModel`], [`State::NoView`], [`State::NoEntity`]) are
    /// not loud: they get a neutral informative fallback via
    /// [`set_absence`] (or the embedder's slot), so a still-seeding card
    /// reads as a quiet placeholder rather than a red error.
    #[cfg(any(target_arch = "wasm32", test))]
    pub(crate) fn is_loud(self) -> bool {
        matches!(
            self,
            State::Unauthorized | State::Offline | State::Malformed | State::Unknown
        )
    }
}

/// Short label for the error callout's `<strong>` heading, chosen from
/// the classified [`State`] where the state is more specific than the
/// [`ErrorKind`] behind it. A `404` and a dropped connection are both
/// `ErrorKind::Network`, but only one of them is a connection problem,
/// so titling on `kind` alone told users their network was down when
/// the repository simply was not here.
pub fn state_title(state: State, kind: ErrorKind) -> &'static str {
    match state {
        State::Unknown => "Not here",
        State::Unauthorized => "No access",
        _ => error_title(kind),
    }
}

/// Short label for the error callout's `<strong>` heading. Pure
/// mapping from the upstream `ErrorKind` to a user-facing string.
/// Kept here (rather than next to the wasm-only error rendering)
/// so the mapping can be unit-tested natively.
pub fn error_title(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::UnknownSource => "Not found",
        ErrorKind::Network => "Connection failed",
        ErrorKind::Parse => "Couldn't read response",
        ErrorKind::Descriptor => "Invalid configuration",
    }
}

/// Classify an upstream [`ErrorKind`] into the lifecycle [`State`] it
/// should drive. A network failure is `Offline` (transport, recovers on
/// reconnect); a bad descriptor / decode is `Malformed` (author or
/// protocol error). `UnknownSource` here means a query result was
/// well-formed but empty in a context the caller treats as a hard error
/// — it maps to `Malformed` so the recoverable absences stay distinct
/// (those set their state directly, not through the error path).
pub fn state_for(kind: ErrorKind) -> State {
    match kind {
        ErrorKind::Network => State::Offline,
        ErrorKind::Parse | ErrorKind::Descriptor | ErrorKind::UnknownSource => State::Malformed,
    }
}

/// Sentinel `data-` attribute we tag the injected callout with so
/// we can find and replace it on the next state transition without
/// disturbing whatever else the renderer mounted.
#[cfg(target_arch = "wasm32")]
const ERROR_CALLOUT_ATTR: &str = "data-tonk-display-error";

/// Set `data-state` on `host`. Idempotent — safe to call repeatedly
/// with the same state.
///
/// Transitioning to any non-loud state removes a callout a prior loud
/// state injected, so a recoverable absence (`no-model` / `no-entity`)
/// or a recovery to `ready` never leaves a stale red box behind.
#[cfg(target_arch = "wasm32")]
pub fn set(host: &Element, state: State) {
    let _ = host.set_attribute("data-state", state.as_str());
    if !state.is_loud() {
        remove_error_callout(host);
    }
    // Any state set through this path (rather than `set_absence`) clears a
    // lingering absence fallback — e.g. recovery to `ready` /
    // `default-view` after a `no-model`, so the informative callout does
    // not sit beside the rendered content.
    remove_absence_callout(host);
    // The default-view notice coexists with rendered content, so it is
    // cleared on every transition and re-injected only for `DefaultView`.
    remove_default_notice(host);
    if matches!(state, State::DefaultView) {
        set_default_notice(host);
    }
    // Project the matching slot child for this state and hide the rest.
    // `ready` / `default-view` render the view output, so no slot child
    // shows; `loading` / `empty` may have their own slot child.
    let project = match state {
        State::Ready | State::DefaultView => None,
        other => Some(other),
    };
    update_slot_children(host, project);
}

/// Sentinel marking the built-in *default-view* notice, kept distinct from the
/// error and absence sentinels so they never clobber each other.
#[cfg(target_arch = "wasm32")]
const DEFAULT_NOTICE_ATTR: &str = "data-tonk-display-default-notice";

/// Inject a `<wa-callout variant="warning">` telling the viewer that the model
/// has no view of its own, so the built-in default presentation (the `_:_` view
/// or the notation dump) is shown instead. Same shape as the absence callouts
/// ([`set_absence`]) — a `circle-info` icon and a plain text label — but
/// `warning` (theme yellow), not the `danger` red of a missing model/view: a
/// model without its own view still renders something, so it is a heads-up, not
/// an error. Unlike the absence callouts it coexists with the rendered fallback
/// content rather than replacing it, so it is **prepended** (sits on top, above
/// the default presentation). An embedder can suppress it with a
/// `slot="default-view"` child.
#[cfg(target_arch = "wasm32")]
fn set_default_notice(host: &Element) {
    // Let an embedder own the notice via `slot="default-view"`; if it did,
    // skip the built-in one.
    if host
        .query_selector("[slot=\"default-view\"]")
        .ok()
        .flatten()
        .is_some()
    {
        return;
    }
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    let Ok(callout) = document.create_element("wa-callout") else {
        return;
    };
    let _ = callout.set_attribute("variant", "warning");
    let _ = callout.set_attribute(DEFAULT_NOTICE_ATTR, "");
    if let Ok(icon) = document.create_element("wa-icon") {
        let _ = icon.set_attribute("slot", "icon");
        let _ = icon.set_attribute("name", "circle-info");
        let _ = callout.append_child(&icon);
    }
    // Name the model so the viewer knows exactly which one lacks a view.
    let model = host.get_attribute("model").unwrap_or_default();
    let text = if model.is_empty() {
        "No view for this model; showing the default.".to_owned()
    } else {
        format!("No view for {model}; showing the default.")
    };
    let label = document.create_text_node(&text);
    let _ = callout.append_child(&label);
    // Prepend so the notice sits on top of the rendered default content.
    let _ = host.insert_before(&callout, host.first_child().as_ref());
}

#[cfg(target_arch = "wasm32")]
fn remove_default_notice(host: &Element) {
    let selector = format!("[{DEFAULT_NOTICE_ATTR}]");
    let Ok(found) = host.query_selector_all(&selector) else {
        return;
    };
    for i in 0..found.length() {
        if let Some(node) = found.item(i)
            && let Some(el) = node.dyn_ref::<Element>()
        {
            el.remove();
        }
    }
}

/// Transition the host to a loud error `state` and surface a
/// `<wa-callout variant="danger">` inside the host with the given
/// `title` + `message`. The shape matches Web Awesome's reference
/// danger callout: icon in the `icon` slot, a bold title line, a
/// `<br>`, then the message body. Replaces any existing callout
/// from a prior error so the user always sees the most recent
/// failure.
///
/// `state` is the classified loud state (`offline` / `unauthorized` /
/// `malformed`) so `data-state` and the callout stay in step.
#[cfg(target_arch = "wasm32")]
pub fn set_error(host: &Element, state: State, title: &str, message: &str) {
    let _ = host.set_attribute("data-state", state.as_str());
    remove_error_callout(host);
    remove_absence_callout(host);
    // An embedder may slot the loud state too (`slot="offline"` etc.). If
    // it did, show that child and skip the built-in danger callout;
    // otherwise hide every slot child and inject the callout below.
    if update_slot_children(host, Some(state)) {
        return;
    }
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    let Ok(callout) = document.create_element("wa-callout") else {
        return;
    };
    let _ = callout.set_attribute("variant", "danger");
    let _ = callout.set_attribute(ERROR_CALLOUT_ATTR, "");

    // `<wa-icon slot="icon">` renders inside the callout's start
    // affordance so the surface reads as an alert at a glance.
    if let Ok(icon) = document.create_element("wa-icon") {
        let _ = icon.set_attribute("slot", "icon");
        let _ = icon.set_attribute("name", "circle-exclamation");
        let _ = callout.append_child(&icon);
    }
    // `<strong>` title line — short label naming the failure kind.
    if let Ok(strong) = document.create_element("strong") {
        strong.set_text_content(Some(title));
        let _ = callout.append_child(&strong);
    }
    // Line break between title and detail message, matching the WA
    // reference example.
    if let Ok(br) = document.create_element("br") {
        let _ = callout.append_child(&br);
    }
    let message_text = document.create_text_node(message);
    let _ = callout.append_child(&message_text);

    let _ = host.append_child(&callout);
}

#[cfg(target_arch = "wasm32")]
fn remove_error_callout(host: &Element) {
    // We only ever inject a single callout, but query for the
    // sentinel attribute defensively in case more than one snuck in.
    let selector = format!("[{ERROR_CALLOUT_ATTR}]");
    let Ok(found) = host.query_selector_all(&selector) else {
        return;
    };
    for i in 0..found.length() {
        if let Some(node) = found.item(i)
            && let Some(el) = node.dyn_ref::<Element>()
        {
            el.remove();
        }
    }
}

/// Sentinel marking the built-in *absence* fallback callout, kept
/// distinct from [`ERROR_CALLOUT_ATTR`] so an informative absence
/// fallback and a loud error never clobber each other's sentinel.
#[cfg(target_arch = "wasm32")]
const ABSENCE_CALLOUT_ATTR: &str = "data-tonk-display-absence";

/// Enter a recoverable-absence `state` (`no-model` / `no-view` /
/// `no-entity`), naming what was missing: a prose `label` plus a
/// `notation` snippet (e.g. `{ this: did:key:… }`) rendered as
/// syntax-highlighted, monospace `<tonk-notation>`.
///
/// The embedder may handle the state itself by providing a light-DOM
/// child with `slot="<state>"` (e.g. `<span slot="no-model">…</span>`).
/// `<tonk-display>` is light-DOM (no shadow root), so a bare `slot=`
/// attribute would otherwise render *always*; this fn drives the
/// projection manually — it shows the child whose `slot` matches the
/// current state and hides every other absence/loading slot child (see
/// [`update_slot_children`]). When the host provides *no* slot for this
/// state, a built-in `<wa-callout variant="neutral">` is mounted naming
/// the missing concept, so a bare `<tonk-display>` (e.g. the display
/// route) reads as an informative message rather than a blank element.
///
/// Neutral, not danger: a missing concept on a still-syncing branch is
/// expected, not an error — it recovers when the definition lands. The
/// loud `danger` callout stays reserved for [`set_error`]
/// (offline/unauthorized/malformed).
#[cfg(target_arch = "wasm32")]
pub fn set_absence(host: &Element, state: State, label: &str, notation: &str) {
    let _ = host.set_attribute("data-state", state.as_str());
    // A prior loud error must not linger under an absence.
    remove_error_callout(host);
    remove_absence_callout(host);

    // Show the matching slot child (if any) and hide the rest. A present
    // matching child is the embedder opting out of the built-in fallback.
    let has_slot = update_slot_children(host, Some(state));
    if has_slot {
        return;
    }

    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    let Ok(callout) = document.create_element("wa-callout") else {
        return;
    };
    // A missing model/view concept is a config/authoring problem the
    // display can't render around → `danger`. A missing *instance*
    // (`no-entity`) is also `danger`: the entity is on the branch but
    // does not match the concept (some required attribute is absent), so
    // the diagnostic that accompanies it explains why the match failed —
    // not a quiet still-syncing placeholder. The variant carries the
    // severity; the icon is the same info glyph for every absence.
    let variant = match state {
        State::NoModel | State::NoView | State::NoEntity => "danger",
        _ => "neutral",
    };
    let _ = callout.set_attribute("variant", variant);
    let _ = callout.set_attribute(ABSENCE_CALLOUT_ATTR, "");
    if let Ok(icon) = document.create_element("wa-icon") {
        let _ = icon.set_attribute("slot", "icon");
        let _ = icon.set_attribute("name", "circle-info");
        let _ = callout.append_child(&icon);
    }
    // The callout is just the message strip — a short title like "Model
    // not found". It carries no query detail itself.
    let label_text = document.create_text_node(label);
    let _ = callout.append_child(&label_text);
    let _ = host.append_child(&callout);

    // The query that matched nothing is a SEPARATE sibling, not part of
    // the callout: a styleable `<div class="tonk-display-query">` whose
    // visibility CSS owns (hidden by default, revealed on hover / focus /
    // a dev toggle — the embedder decides). The query renders through
    // `<tonk-notation>` (syntax-highlighted, monospace — the same renderer
    // the app uses for entity dumps), which reads its source from a
    // `<script type="text/tonk-notation">` child.
    let notation_body: Option<Element> = match (
        document.create_element("tonk-notation"),
        document.create_element("script"),
    ) {
        (Ok(notation_el), Ok(script)) => {
            let _ = script.set_attribute("type", "text/tonk-notation");
            script.set_text_content(Some(notation));
            let _ = notation_el.append_child(&script);
            Some(notation_el)
        }
        _ => match document.create_element("code") {
            Ok(code) => {
                code.set_text_content(Some(notation));
                Some(code)
            }
            Err(_) => None,
        },
    };
    if let Some(body) = notation_body
        && let Ok(query) = document.create_element("div")
    {
        let _ = query.set_attribute("class", "tonk-display-query");
        // Same sentinel so `remove_absence_callout` clears the query
        // sibling alongside the callout on the next transition.
        let _ = query.set_attribute(ABSENCE_CALLOUT_ATTR, "");
        let _ = query.append_child(&body);
        let _ = host.append_child(&query);
    }
}

/// Loud `no-entity` diagnostic: the entity is on the branch but does not
/// match its model concept because one or more required attributes are
/// absent. Renders the concept as an entity dump where every required
/// attribute is a line — present ones carry their value, missing ones
/// render as a squiggled `_` with a `<wa-tooltip>` naming the absent
/// attribute URI — so the viewer sees *why* the match failed without
/// hand-querying.
///
/// `present` is `(field, value)` for attributes the entity carries;
/// `missing` is `(field, attribute_uri)` for the absent ones. The
/// notation source stays clean (`field: _`); the tooltip text rides
/// out-of-band as a `data-error-<field>` attribute on the
/// `<tonk-notation>`, which its renderer turns into the squiggle +
/// tooltip. An embedder may still own the state via a `slot="no-entity"`
/// child.
#[cfg(target_arch = "wasm32")]
pub fn set_no_entity_diagnostic(
    host: &Element,
    model: &str,
    entity: &str,
    present: &[(String, String)],
    missing: &[(String, String)],
    mistyped: &[(String, String, String)],
) {
    let _ = host.set_attribute("data-state", State::NoEntity.as_str());
    remove_error_callout(host);
    remove_absence_callout(host);

    // An embedder may skin `no-entity` itself; if it did, show that child
    // and skip the built-in diagnostic.
    if update_slot_children(host, Some(State::NoEntity)) {
        return;
    }

    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };

    // The loud callout strip — danger, like a missing model/view.
    if let Ok(callout) = document.create_element("wa-callout") {
        let _ = callout.set_attribute("variant", "danger");
        let _ = callout.set_attribute(ABSENCE_CALLOUT_ATTR, "");
        if let Ok(icon) = document.create_element("wa-icon") {
            let _ = icon.set_attribute("slot", "icon");
            let _ = icon.set_attribute("name", "circle-info");
            let _ = callout.append_child(&icon);
        }
        // Say what is actually wrong: "missing" over a value that is
        // right there (just differently typed) sends the author
        // hunting the wrong problem.
        let label = document.create_text_node(if missing.is_empty() && !mistyped.is_empty() {
            "Concept mismatch: attribute value type differs"
        } else {
            "Concept mismatch: required attribute missing"
        });
        let _ = callout.append_child(&label);
        let _ = host.append_child(&callout);
    }

    // Build the entity dump as notation: one line per required attribute,
    // present values verbatim, missing ones as `_`. The renderer paints
    // every blank as a variable; the `data-error-<line>` attributes below
    // upgrade the missing ones to a squiggle + tooltip. Errors are keyed by
    // **line index** (not field name) because notation renders arbitrary
    // expressions where a field name is not unique — a line number points
    // at exactly one row whatever the shape. Line 0 is the head; the first
    // field (`this`) is line 1.
    let mut source = format!("{model}:\n  this: {entity}\n");
    let mut line = 2usize; // head (0), `this` (1) already emitted.
    for (field, value) in present {
        source.push_str(&format!("  {field}: {value}\n"));
        line += 1;
    }
    let mut error_lines: Vec<(usize, String)> = Vec::new();
    // A field whose value EXISTS but under a different value type than
    // the concept declares: show the value, squiggle it with the type
    // story — "missing" would send the author hunting a fact that is
    // right there.
    for (field, value, message) in mistyped {
        source.push_str(&format!("  {field}: {value}\n"));
        error_lines.push((line, message.clone()));
        line += 1;
    }
    for (field, uri) in missing {
        source.push_str(&format!("  {field}: _\n"));
        error_lines.push((line, format!("Attribute {uri} is missing")));
        line += 1;
    }

    let notation_el = match (
        document.create_element("tonk-notation"),
        document.create_element("script"),
    ) {
        (Ok(notation_el), Ok(script)) => {
            let _ = script.set_attribute("type", "text/tonk-notation");
            script.set_text_content(Some(&source));
            let _ = notation_el.append_child(&script);
            // Name each missing attribute out-of-band, keyed by line index,
            // so the renderer can decorate exactly that line.
            for (index, message) in &error_lines {
                let _ = notation_el.set_attribute(&format!("data-error-{index}"), message);
            }
            Some(notation_el)
        }
        _ => None,
    };

    if let Some(body) = notation_el
        && let Ok(query) = document.create_element("div")
    {
        let _ = query.set_attribute("class", "tonk-display-query");
        let _ = query.set_attribute(ABSENCE_CALLOUT_ATTR, "");
        let _ = query.append_child(&body);
        let _ = host.append_child(&query);
    }
}

/// Project the host's `slot="…"` children manually (no shadow root):
/// show the child whose `slot` equals `current`'s `data-state` value,
/// hide every other slot child. Returns whether a child matching
/// `current` was found (so the caller knows the embedder handled the
/// state). `current = None` hides every slot child (used when entering a
/// rendered state — `ready` / `default-view` — where the view output,
/// not a slot, is the content).
///
/// Only `data-state`-named slots are touched, so this never disturbs a
/// view template's own `slot=` usage for unrelated states.
#[cfg(target_arch = "wasm32")]
pub fn update_slot_children(host: &Element, current: Option<State>) -> bool {
    let Ok(children) = host.query_selector_all("[slot]") else {
        return false;
    };
    let target = current.map(State::as_str);
    let mut matched = false;
    for i in 0..children.length() {
        let Some(node) = children.item(i) else {
            continue;
        };
        let Some(el) = node.dyn_ref::<Element>() else {
            continue;
        };
        // Only manage slots whose name is a lifecycle state; leave a view
        // template's own slot children (e.g. Web Awesome part slots)
        // untouched.
        let Some(slot) = el.get_attribute("slot") else {
            continue;
        };
        if !is_state_slot(&slot) {
            continue;
        }
        // Direct children of the host only — a nested display's slots are
        // that display's to manage.
        if el.parent_element().as_ref() != Some(host) {
            continue;
        }
        let show = target == Some(slot.as_str());
        if show {
            let _ = el.remove_attribute("hidden");
            matched = true;
        } else {
            let _ = el.set_attribute("hidden", "");
        }
    }
    matched
}

/// Every lifecycle state, so the slot vocabulary and the `data-state`
/// vocabulary cannot drift apart: a new [`State`] that is not listed
/// here is a state authors silently cannot skin.
pub(crate) const ALL: &[State] = &[
    State::Loading,
    State::Ready,
    State::DefaultView,
    State::NoModel,
    State::NoView,
    State::NoEntity,
    State::Empty,
    State::Unauthorized,
    State::Unknown,
    State::Offline,
    State::Malformed,
];

/// True if `slot` names a lifecycle [`State`] (so it is a slot
/// `<tonk-display>` manages, not a view template's own slot).
#[cfg(any(target_arch = "wasm32", test))]
fn is_state_slot(slot: &str) -> bool {
    ALL.iter().any(|state| state.as_str() == slot)
}

#[cfg(target_arch = "wasm32")]
fn remove_absence_callout(host: &Element) {
    let selector = format!("[{ABSENCE_CALLOUT_ATTR}]");
    let Ok(found) = host.query_selector_all(&selector) else {
        return;
    };
    for i in 0..found.length() {
        if let Some(node) = found.item(i)
            && let Some(el) = node.dyn_ref::<Element>()
        {
            el.remove();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    #[test]
    fn it_maps_states_to_data_state_attribute_values() {
        // CSS rules in `styles.css` (e.g. `tonk-display[data-state="loading"]`)
        // rely on these exact strings — a typo here silently breaks
        // every loading/error skin downstream.
        assert_eq!(State::Loading.as_str(), "loading");
        assert_eq!(State::Ready.as_str(), "ready");
        assert_eq!(State::DefaultView.as_str(), "default-view");
        assert_eq!(State::NoModel.as_str(), "no-model");
        assert_eq!(State::NoView.as_str(), "no-view");
        assert_eq!(State::NoEntity.as_str(), "no-entity");
        assert_eq!(State::Empty.as_str(), "empty");
        assert_eq!(State::Unauthorized.as_str(), "unauthorized");
        assert_eq!(State::Offline.as_str(), "offline");
        assert_eq!(State::Malformed.as_str(), "malformed");
    }

    #[test]
    fn it_keeps_recoverable_absences_quiet_and_breakage_loud() {
        // The recoverable absences are skinned by embedder CSS off
        // `data-state`, not a forced callout, so a still-seeding card
        // never flashes a red box. The genuinely-broken states stay loud.
        assert!(!State::Loading.is_loud());
        assert!(!State::NoModel.is_loud());
        assert!(!State::NoView.is_loud());
        assert!(!State::NoEntity.is_loud());
        assert!(!State::DefaultView.is_loud());
        assert!(State::Malformed.is_loud());
        assert!(State::Offline.is_loud());
        assert!(State::Unauthorized.is_loud());
    }

    #[test]
    fn it_classifies_error_kinds_into_loud_states() {
        assert_eq!(state_for(ErrorKind::Network), State::Offline);
        assert_eq!(state_for(ErrorKind::Parse), State::Malformed);
        assert_eq!(state_for(ErrorKind::Descriptor), State::Malformed);
        assert_eq!(state_for(ErrorKind::UnknownSource), State::Malformed);
    }

    #[test]
    fn it_maps_every_error_kind_to_a_user_facing_title() {
        // The mapping is what users see in the danger callout's
        // bold heading. Pin the four variants so an addition to
        // `ErrorKind` upstream surfaces here as a missing match arm.
        assert_eq!(error_title(ErrorKind::UnknownSource), "Not found");
        assert_eq!(error_title(ErrorKind::Network), "Connection failed");
        assert_eq!(error_title(ErrorKind::Parse), "Couldn't read response");
        assert_eq!(error_title(ErrorKind::Descriptor), "Invalid configuration");
    }

    /// Build a fresh detached host element so tests don't interact
    /// across runs. Wasm-only — `web_sys::window()` is `None` on
    /// native test builds.
    #[cfg(target_arch = "wasm32")]
    fn host() -> Element {
        web_sys::window()
            .expect("window")
            .document()
            .expect("document")
            .create_element("tonk-display")
            .expect("create tonk-display")
    }

    #[cfg(target_arch = "wasm32")]
    #[dialog_common::test]
    fn it_reflects_state_as_a_data_state_attribute() {
        let host = host();
        set(&host, State::Loading);
        assert_eq!(host.get_attribute("data-state").as_deref(), Some("loading"));
        set(&host, State::Ready);
        assert_eq!(host.get_attribute("data-state").as_deref(), Some("ready"));
    }

    #[cfg(target_arch = "wasm32")]
    #[dialog_common::test]
    fn it_does_not_inject_a_callout_for_a_recoverable_absence() {
        // `no-model` is a still-seeding card, not an error: the
        // embedder skins it from `data-state` (CSS placeholder). The
        // display must not force a red callout into the host.
        let host = host();
        set(&host, State::NoModel);
        assert_eq!(
            host.get_attribute("data-state").as_deref(),
            Some("no-model")
        );
        assert!(
            host.query_selector("wa-callout").unwrap().is_none(),
            "a recoverable absence must not inject a callout",
        );
    }

    #[cfg(target_arch = "wasm32")]
    #[dialog_common::test]
    fn it_clears_a_prior_callout_when_recovering_to_a_quiet_state() {
        // A loud failure injects a callout; a later recovery to a
        // quiet state (e.g. the concept lands → no-model→no-entity, or
        // straight to ready) must clear it so no red box latches.
        let host = host();
        set_error(&host, State::Offline, "Connection failed", "boom");
        assert!(host.query_selector("wa-callout").unwrap().is_some());

        set(&host, State::NoEntity);
        assert!(
            host.query_selector("wa-callout").unwrap().is_none(),
            "recovering to a quiet state must remove the callout",
        );
        assert_eq!(
            host.get_attribute("data-state").as_deref(),
            Some("no-entity")
        );
    }

    #[cfg(target_arch = "wasm32")]
    #[dialog_common::test]
    fn it_injects_a_danger_fallback_naming_the_missing_model() {
        // A bare `<tonk-display>` (no slot child) with a missing model
        // must NOT render blank — it shows a callout with the query that
        // matched nothing. A missing model concept is a config error the
        // display can't render around → `danger`.
        let host = host();
        set_absence(&host, State::NoModel, "Not found", "concept:\n  this: test");

        assert_eq!(
            host.get_attribute("data-state").as_deref(),
            Some("no-model")
        );
        let callout = host
            .query_selector("wa-callout")
            .unwrap()
            .expect("a bare absence must inject a fallback callout");
        assert_eq!(
            callout.get_attribute("variant").as_deref(),
            Some("danger"),
            "a missing model is a danger, not informative",
        );
        assert_eq!(
            callout.text_content().unwrap().trim(),
            "Not found",
            "the callout strip carries just the message, not the query",
        );
        // The query is a SEPARATE sibling — a styleable `.tonk-display-query`
        // wrapping `<tonk-notation>` — so CSS owns its visibility.
        let query = host
            .query_selector(".tonk-display-query")
            .unwrap()
            .expect("the query is a separate styleable sibling");
        let script = query
            .query_selector("tonk-notation script[type=\"text/tonk-notation\"]")
            .unwrap()
            .expect("notation source script present");
        assert!(script.text_content().unwrap().contains("this: test"));
    }

    #[cfg(target_arch = "wasm32")]
    #[dialog_common::test]
    fn it_injects_a_danger_fallback_for_a_missing_view() {
        // `no-view` (an explicit `view=` whose concept is absent) is also
        // a config error → `danger`, naming the missing view query.
        let host = host();
        set_absence(
            &host,
            State::NoView,
            "Not found",
            "view:\n  this: tonk:view/x\n  model: person",
        );
        assert_eq!(host.get_attribute("data-state").as_deref(), Some("no-view"));
        let callout = host
            .query_selector("wa-callout")
            .unwrap()
            .expect("no-view injects a fallback callout");
        assert_eq!(callout.get_attribute("variant").as_deref(), Some("danger"));
        // The missing view is named in the separate query sibling.
        let query = host
            .query_selector(".tonk-display-query")
            .unwrap()
            .expect("no-view emits a query sibling");
        assert!(
            query.text_content().unwrap().contains("tonk:view/x"),
            "names the missing view",
        );
    }

    #[cfg(target_arch = "wasm32")]
    #[dialog_common::test]
    fn it_injects_a_danger_fallback_for_a_missing_entity() {
        // `no-entity` is loud (`danger`): the entity is on the branch but
        // does not match the concept (a required attribute is absent), so
        // the accompanying diagnostic explains why — not a quiet
        // still-syncing placeholder.
        let host = host();
        set_absence(
            &host,
            State::NoEntity,
            "Not found",
            "person:\n  this: did:key:zX",
        );
        assert_eq!(
            host.get_attribute("data-state").as_deref(),
            Some("no-entity")
        );
        let callout = host
            .query_selector("wa-callout")
            .unwrap()
            .expect("no-entity injects a fallback callout");
        assert_eq!(
            callout.get_attribute("variant").as_deref(),
            Some("danger"),
            "a concept mismatch is an error, not a quiet placeholder",
        );
    }

    #[cfg(target_arch = "wasm32")]
    #[dialog_common::test]
    fn it_shows_the_slot_child_and_suppresses_the_fallback() {
        // When the embedder slots its own content for the state, the
        // built-in fallback is suppressed and the slotted child shows.
        let host = host();
        let document = web_sys::window().unwrap().document().unwrap();
        let mine = document.create_element("span").unwrap();
        mine.set_attribute("slot", "no-model").unwrap();
        mine.set_attribute("hidden", "").unwrap();
        mine.set_text_content(Some("Untitled"));
        host.append_child(&mine).unwrap();

        set_absence(&host, State::NoModel, "No matching concept ", "{ this: x }");

        assert!(
            host.query_selector("wa-callout").unwrap().is_none(),
            "a provided slot suppresses the built-in fallback",
        );
        assert!(
            !mine.has_attribute("hidden"),
            "the matching slot child is shown",
        );
    }

    #[cfg(target_arch = "wasm32")]
    #[dialog_common::test]
    fn it_projects_only_the_matching_slot_child() {
        // The display projects manually (light DOM, no shadow root): only
        // the child whose `slot` matches the current state is visible.
        let host = host();
        let document = web_sys::window().unwrap().document().unwrap();
        for state in ["no-model", "no-entity", "loading"] {
            let s = document.create_element("span").unwrap();
            s.set_attribute("slot", state).unwrap();
            s.set_attribute("hidden", "").unwrap();
            s.set_text_content(Some(state));
            host.append_child(&s).unwrap();
        }

        set_absence(
            &host,
            State::NoEntity,
            "Nothing found for ",
            "x: { this: y }",
        );

        let shown = host
            .query_selector("[slot=\"no-entity\"]")
            .unwrap()
            .unwrap();
        assert!(!shown.has_attribute("hidden"), "matching slot is shown");
        let other = host.query_selector("[slot=\"no-model\"]").unwrap().unwrap();
        assert!(
            other.has_attribute("hidden"),
            "non-matching slot stays hidden",
        );

        // Recovery to `ready` hides every slot child.
        set(&host, State::Ready);
        assert!(
            host.query_selector("[slot=\"no-entity\"]")
                .unwrap()
                .unwrap()
                .has_attribute("hidden"),
            "ready hides all slot children",
        );
    }

    #[cfg(target_arch = "wasm32")]
    #[dialog_common::test]
    fn it_renders_the_danger_callout_on_set_error() {
        let host = host();
        set_error(&host, State::Malformed, "Not found", "no entity matched");

        // `data-state` flips to the loud state we passed.
        assert_eq!(
            host.get_attribute("data-state").as_deref(),
            Some("malformed")
        );

        // Exactly one callout, marked danger, carrying our sentinel.
        let callout = host
            .query_selector("wa-callout")
            .expect("query")
            .expect("callout mounted");
        assert_eq!(callout.get_attribute("variant").as_deref(), Some("danger"));
        assert!(callout.has_attribute("data-tonk-display-error"));

        // Icon, title, and message body are all present in that
        // order. The callout's text content is the concatenated
        // text of every child node; checking it covers both the
        // <strong> title and the trailing text node.
        let strong = callout
            .query_selector("strong")
            .expect("query strong")
            .expect("title present");
        assert_eq!(strong.text_content().as_deref(), Some("Not found"));
        let icon = callout
            .query_selector("wa-icon")
            .expect("query icon")
            .expect("icon present");
        assert_eq!(icon.get_attribute("slot").as_deref(), Some("icon"));
        assert_eq!(
            icon.get_attribute("name").as_deref(),
            Some("circle-exclamation"),
        );
        assert!(
            callout
                .text_content()
                .unwrap()
                .contains("no entity matched")
        );
    }

    #[cfg(target_arch = "wasm32")]
    #[dialog_common::test]
    fn it_replaces_a_prior_callout_when_set_error_runs_again() {
        let host = host();
        set_error(&host, State::Malformed, "Not found", "first");
        set_error(&host, State::Offline, "Connection failed", "second");

        // Only one callout should remain — the most recent.
        let count = host
            .query_selector_all("wa-callout")
            .expect("query")
            .length();
        assert_eq!(count, 1, "expected exactly one callout, got {count}");
        let text = host
            .query_selector("wa-callout")
            .unwrap()
            .unwrap()
            .text_content()
            .unwrap();
        assert!(text.contains("Connection failed"), "stale title: {text}");
        assert!(text.contains("second"), "stale message: {text}");
    }

    #[cfg(target_arch = "wasm32")]
    #[dialog_common::test]
    fn it_removes_the_callout_when_transitioning_away_from_error() {
        let host = host();
        set_error(&host, State::Offline, "Connection failed", "boom");
        assert!(host.query_selector("wa-callout").unwrap().is_some());

        set(&host, State::Loading);
        // The callout is gone now that we're no longer in a loud state.
        assert!(host.query_selector("wa-callout").unwrap().is_none());
        assert_eq!(host.get_attribute("data-state").as_deref(), Some("loading"));
    }

    /// The callout heading follows the classified state where the state
    /// knows more than the kind: a `404` and a dropped connection are
    /// both `Network`, but only one is a connection problem.
    #[dialog_common::test]
    fn it_titles_an_unknown_repository_without_blaming_the_connection() {
        assert_eq!(state_title(State::Unknown, ErrorKind::Network), "Not here");
        assert_eq!(
            state_title(State::Unauthorized, ErrorKind::Network),
            "No access"
        );
        assert_eq!(
            state_title(State::Offline, ErrorKind::Network),
            "Connection failed"
        );
    }

    #[dialog_common::test]
    fn it_names_the_unknown_state_for_css_authors() {
        assert_eq!(State::Unknown.as_str(), "unknown");
        assert!(State::Unknown.is_loud());
    }

    /// `ALL` drives the slot vocabulary, so a state missing from it is
    /// one authors cannot skin. The `match` is exhaustive: adding a
    /// variant fails to compile here until it is listed, which is the
    /// point — the previous hand-written slot list had silently fallen
    /// behind the enum.
    #[dialog_common::test]
    fn it_lists_every_state_for_slot_projection() {
        for state in ALL {
            // Exhaustive by construction — a new variant breaks this arm.
            match state {
                State::Loading
                | State::Ready
                | State::DefaultView
                | State::NoModel
                | State::NoView
                | State::NoEntity
                | State::Empty
                | State::Unauthorized
                | State::Unknown
                | State::Offline
                | State::Malformed => {}
            }
        }
        let mut names: Vec<&str> = ALL.iter().map(|s| s.as_str()).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "every state needs a distinct slot name");
        assert!(is_state_slot("unknown"), "unknown must be skinnable");
        assert!(!is_state_slot("icon"), "a view's own slot is left alone");
    }
}
