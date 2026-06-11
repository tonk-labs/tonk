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
/// display reports where it is and the embedder skins it (the built-in
/// presentation is a CSS placeholder, not a forced callout) — except
/// the genuinely-broken states ([`State::Malformed`], [`State::Offline`],
/// [`State::Unauthorized`]) which still surface a visible callout via
/// [`set_error`]. See `plan/tonk-display-states.md`.
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
            State::Offline => "offline",
            State::Malformed => "malformed",
        }
    }

    /// Whether this state injects a visible `<wa-callout>` via
    /// [`set_error`]. The recoverable absences ([`State::NoModel`],
    /// [`State::NoView`], [`State::NoEntity`]) are skinned by the
    /// embedder's CSS off `data-state`, not a forced callout, so a
    /// still-seeding card shows a quiet placeholder instead of a red
    /// error. The genuinely-broken states stay loud.
    #[cfg(any(target_arch = "wasm32", test))]
    pub(crate) fn is_loud(self) -> bool {
        matches!(
            self,
            State::Unauthorized | State::Offline | State::Malformed
        )
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
}
