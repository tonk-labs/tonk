//! Background-sync chrome for the workspace top bar.
//!
//! One dumb light-DOM custom element, cut from the same cloth as
//! [`super::share`]: it resolves its repo/branch from the nearest
//! `with="branch@repo"` ancestor and otherwise holds no app policy.
//!
//! - `<tonk-sync-state>` is the status indicator *and* the pause/resume
//!   button in one. It shows exactly three states for the branch in
//!   scope:
//!     - `synced`  — auto-sync on and up to date with the remote.
//!     - `syncing` — auto-sync on and mid-reconcile (pushing/pulling).
//!       The remote's `ahead` / `behind` / `diverged` states all fold
//!       into this one: if sync is running, those deltas are being
//!       reconciled right now.
//!     - `paused`  — auto-sync off. Overrides any real drift.
//!
//!   It re-reads the read-only `sync/status` route whenever the
//!   controller dispatches `tonk:status-refresh` or a commit lands, and
//!   clicking the pill flips the per-repository `tonk:auto-sync:{repo}`
//!   `localStorage` preference — the exact key the Leptos
//!   `sync_controller` reads. For a branch with no upstream it instead
//!   reveals an "Enable sync" trigger that opens the notation
//!   `#enable-sync` dialog, stamping the repo it resolved from its
//!   ancestor into the dialog's hidden `name` input (the workspace
//!   `{name}` is a display label, not the repo name, so the form can't
//!   read it declaratively).
//!
//! It coordinates with the controller only through that shared
//! `localStorage` key and the `tonk:status-refresh` / `tonk:committed`
//! window events — no direct coupling to `tonk-ui`.

// Pure helpers, compiled for native *test* builds as well as wasm so
// the mapping/preference logic is unit-tested natively. The DOM
// elements below that consume them are wasm-only. The `wasm32 || test`
// gate keeps a native non-test build (which mounts no elements) from
// carrying them as dead code.
#[cfg(any(target_arch = "wasm32", test))]
mod logic {
    use tonk_schema::SyncState;

    /// Per-repository `localStorage` key for the auto-sync pause
    /// preference. Identical contract to `sync_controller`.
    pub(crate) fn pref_key(repo: &str) -> String {
        format!("tonk:auto-sync:{repo}")
    }

    /// Interpret a stored auto-sync preference. Default on — only an
    /// explicit `"off"` pauses, so a missing or unrecognized value
    /// leaves background sync running.
    pub(crate) fn pref_is_enabled(stored: Option<&str>) -> bool {
        stored != Some("off")
    }

    /// Resolve a branch name, defaulting to `main` when it is absent or
    /// empty (the convention the display route encodes).
    pub(crate) fn branch_or_default(name: Option<String>) -> String {
        name.filter(|n| !n.is_empty())
            .unwrap_or_else(|| "main".to_string())
    }

    /// The pill's `(label, modifier-class)` for the paused preference
    /// and for an absent / unreachable remote. `paused` comes from the
    /// preference (not a `SyncState`); `offline` covers both `NoUpstream`
    /// (no remote configured) and a failed status fetch (remote
    /// unavailable), which read identically to the user.
    pub(crate) const PAUSED_CHIP: (&str, &str) = ("paused", "is-paused");
    pub(crate) const OFFLINE_CHIP: (&str, &str) = ("offline", "is-offline");

    /// Label and modifier class for a live sync state.
    ///
    /// The pill shows only three *running* labels: `synced` when up to
    /// date, `syncing…` for any drift (`Ahead` / `Behind` / `Diverged`
    /// all mean auto-sync is mid-reconcile, so they collapse into one),
    /// and `offline` when there is no upstream to sync against. (`paused`
    /// is not a `SyncState`; it comes from the preference and is painted
    /// by the element directly.)
    pub(crate) fn state_chip(state: SyncState) -> (&'static str, &'static str) {
        match state {
            SyncState::Synced => ("synced", "is-synced"),
            SyncState::Ahead | SyncState::Behind | SyncState::Diverged => {
                ("syncing…", "is-syncing")
            }
            SyncState::NoUpstream => OFFLINE_CHIP,
        }
    }
}

// The two custom elements that consume the logic above. Light DOM,
// in the `<tonk-share>` mold: resolve context from ancestors, hold no
// app policy, coordinate with the controller only through the shared
// `localStorage` key and the window events.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod dom {
    use std::cell::RefCell;
    use std::rc::Rc;

    use custom_elements::CustomElement;
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::spawn_local;
    use web_sys::{CustomEvent, CustomEventInit, Element, Event, HtmlElement, window};

    use tonk_schema::SyncState;

    use super::logic::{
        OFFLINE_CHIP, PAUSED_CHIP, branch_or_default, pref_is_enabled, pref_key, state_chip,
    };
    use crate::ancestors::repo_from_context;

    /// Window event the controller dispatches to ask the chip to
    /// re-read its read-only sync status. Repo name rides in `detail`.
    const STATUS_REFRESH_EVENT: &str = "tonk:status-refresh";

    /// Window event a local commit dispatches; the chip re-reads on it
    /// so a fresh write reflects immediately.
    const COMMITTED_EVENT: &str = "tonk:committed";

    /// A retained listener closure, kept alive for the element's
    /// lifetime so the listener stays valid; dropped on disconnect.
    type Listener = Rc<RefCell<Option<Closure<dyn FnMut(Event)>>>>;

    /// Whether background sync is enabled for `repo` (default on). The
    /// exact `localStorage` contract `sync_controller` reads.
    fn is_enabled(repo: &str) -> bool {
        let stored = window()
            .and_then(|w| w.local_storage().ok().flatten())
            .and_then(|s| s.get_item(&pref_key(repo)).ok().flatten());
        pref_is_enabled(stored.as_deref())
    }

    /// Persist whether background sync is enabled for `repo`, returning
    /// whether the write landed. A `false` return means the preference
    /// did not change (no `localStorage`, quota, privacy mode), so the
    /// caller must not paint the new state as in effect.
    #[must_use]
    fn set_enabled(repo: &str, enabled: bool) -> bool {
        let Some(window) = window() else {
            return false;
        };
        let Ok(Some(storage)) = window.local_storage() else {
            return false;
        };
        let value = if enabled { "on" } else { "off" };
        storage.set_item(&pref_key(repo), value).is_ok()
    }

    /// Read the branch of the nearest `with` ancestor, defaulting to
    /// `main` when absent, empty, or bare-repo (no branch part). Falls back
    /// to the bridge context inside the sealed guest (the routing context
    /// may live outside the iframe).
    fn branch_from_ancestor(this: &HtmlElement) -> String {
        let name = this
            .closest("[with]")
            .ok()
            .flatten()
            .and_then(|el| el.get_attribute("with"))
            .filter(|v| !v.is_empty() && !v.contains('{'))
            .and_then(|v| v.parse::<tonk_host::location::Location>().ok())
            .and_then(|location| location.branch().map(str::to_owned))
            .or_else(|| tonk_host::bridge::context_field("branch"));
        branch_or_default(name)
    }

    /// Fetch the read-only sync status for `repo`/`branch`, returning
    /// the classified [`SyncState`] or `None` on any failure. Gated on
    /// service-worker readiness so a cold-start call doesn't land on the
    /// asset server.
    ///
    /// The chip degrades to invisible on `None`, so the *cold-start*
    /// failures (no window, a fetch rejected before the worker is up)
    /// stay quiet. But once we have a *response*, its shape is a
    /// contract the worker is meant to honour — a non-200, undecodable
    /// body, malformed JSON, or missing/unknown `state` is a real defect
    /// a dev should see, so those leave a `console` breadcrumb.
    async fn fetch_sync_status(repo: &str, branch: &str) -> Option<SyncState> {
        tonk_host::ready::wait().await;
        // GET a host-RELATIVE path: `host_fetch_text` performs it on the real
        // origin — directly in the top document, or over the `window.tonk`
        // bridge from the sealed guest (which has no reachable origin of its
        // own). Either way the chip needs no `window.location`.
        let path = format!("/api/repository/{repo}/branch/{branch}/sync/status");
        let text = match tonk_host::bridge::host_fetch_text(&path).await {
            Ok(text) => text,
            // Quiet on failure: a cold-start or unreachable remote is expected
            // and the chip degrades to `offline` rather than logging noise.
            Err(_) => return None,
        };
        // Read just the `state` field; the rest of the response (local /
        // remote revisions) is the inspector's concern.
        let value: serde_json::Value = match serde_json::from_str(&text) {
            Ok(value) => value,
            Err(err) => {
                tonk_common::log!("sync status: malformed JSON from {path}: {err}");
                return None;
            }
        };
        let Some(state) = value.get("state") else {
            tonk_common::log!("sync status: response from {path} missing `state` field");
            return None;
        };
        match serde_json::from_value::<SyncState>(state.clone()) {
            Ok(state) => Some(state),
            Err(err) => {
                tonk_common::log!("sync status: unrecognized `state` from {path}: {err}");
                None
            }
        }
    }

    /// Dispatch [`STATUS_REFRESH_EVENT`] on `window` with `repo` in the
    /// detail, so the pill re-reads status immediately after a toggle.
    fn request_status_refresh(repo: &str) {
        let init = CustomEventInit::new();
        init.set_detail(&JsValue::from_str(repo));
        let Ok(event) = CustomEvent::new_with_event_init_dict(STATUS_REFRESH_EVENT, &init) else {
            return;
        };
        if let Some(win) = window() {
            let _ = win.dispatch_event(&event);
        }
    }

    // ---- `<tonk-sync-state>` --------------------------------------

    /// CSS class for the pill button (with an `is-*` state modifier).
    const STATE_CHIP: &str = "workspace__sync";

    /// Title/`aria-label` on the pill while auto-sync is running and
    /// reachable — the `synced` / `syncing…` states, where a click pauses.
    const RUNNING_LABEL: &str = "Auto-sync on — click to pause";
    /// Title/`aria-label` on the pill while paused (and reachable), where
    /// a click resumes.
    const PAUSED_LABEL: &str = "Auto-sync paused — click to resume";
    /// Title/`aria-label` on the offline pill — no upstream or an
    /// unreachable remote. Not a toggle hint: the pill isn't clickable.
    const OFFLINE_LABEL: &str = "Offline — no remote to sync";

    /// Install the pill's click listener on the host: flip the
    /// preference and ask for a status refresh. The refresh dispatches
    /// `tonk:status-refresh`, which this element's own listener catches
    /// and repaints from — the same route the controller's sweep uses,
    /// so the pill has a single refresh path. A rejected write leaves
    /// the pill as-is. The click bubbles up from the inner button, so a
    /// click anywhere on the pill toggles.
    ///
    /// Only the actionable pill toggles: an `offline` pill (no upstream
    /// or unreachable) and the "Enable sync" trigger (shown in place of
    /// the pill for a no-upstream branch) have nothing to pause, so a
    /// click on them is a no-op — and the trigger's own listener still
    /// runs to open the enable-sync dialog.
    fn install_toggle_click(this: &HtmlElement, slot: &Listener) {
        let host = this.clone();
        let listener = Closure::wrap(Box::new(move |_event: Event| {
            if !is_togglable(&host) {
                return;
            }
            let Some(repo) = repo_from_context(&host) else {
                return;
            };
            let next = !is_enabled(&repo);
            if set_enabled(&repo, next) {
                request_status_refresh(&repo);
            }
        }) as Box<dyn FnMut(Event)>);
        let _ = this.add_event_listener_with_callback("click", listener.as_ref().unchecked_ref());
        *slot.borrow_mut() = Some(listener);
    }

    /// Whether the host is currently showing an actionable pill — a
    /// `synced` / `syncing…` / `paused` state the user can pause or
    /// resume. False for the inert `offline` pill and when the
    /// enable-sync trigger is showing instead of a pill (no pill child).
    fn is_togglable(host: &HtmlElement) -> bool {
        let (_, offline_class) = OFFLINE_CHIP;
        host.query_selector(&format!(":scope > .{STATE_CHIP}"))
            .ok()
            .flatten()
            .is_some_and(|button| !button.class_list().contains(offline_class))
    }

    /// CSS class for the "Enable sync" trigger button rendered in place
    /// of the chip when the branch has no upstream.
    const ENABLE_BUTTON: &str = "workspace__enable-sync";

    /// Label for the enable-sync trigger.
    const ENABLE_LABEL: &str = "Enable sync";

    /// `id` of the notation enable-sync form (`core.yaml`) whose hidden
    /// `name` input this element stamps with the resolved repo on click.
    const ENABLE_SYNC_FORM_ID: &str = "enable-sync-form";

    /// Per-element state for `<tonk-sync-state>`.
    #[derive(Default)]
    pub(crate) struct TonkSyncState {
        refresh: Listener,
        committed: Listener,
        /// Pause/resume toggle on the pill (the synced/syncing/paused
        /// states); a no-op while the enable-sync trigger or an offline
        /// pill is showing.
        click: Listener,
        /// Enable-sync trigger: stamps the resolved repo into the
        /// notation enable-sync form before its dialog opens.
        trigger: Listener,
    }

    impl CustomElement for TonkSyncState {
        fn shadow() -> bool {
            false
        }

        fn observed_attributes() -> &'static [&'static str] {
            &[]
        }

        fn inject_children(&mut self, _this: &HtmlElement) {}

        fn connected_callback(&mut self, this: &HtmlElement) {
            refresh_state(this);
            install_state_listeners(this, &self.refresh, &self.committed, refresh_state);
            install_toggle_click(this, &self.click);
            install_trigger_click(this, &self.trigger);
        }

        fn disconnected_callback(&mut self, _this: &HtmlElement) {
            if let Some(win) = window() {
                if let Some(listener) = self.refresh.borrow().as_ref() {
                    let _ = win.remove_event_listener_with_callback(
                        STATUS_REFRESH_EVENT,
                        listener.as_ref().unchecked_ref(),
                    );
                }
                if let Some(listener) = self.committed.borrow().as_ref() {
                    let _ = win.remove_event_listener_with_callback(
                        COMMITTED_EVENT,
                        listener.as_ref().unchecked_ref(),
                    );
                }
            }
            self.refresh.borrow_mut().take();
            self.committed.borrow_mut().take();
            // The click and trigger listeners are element-local (on
            // `this`), so removal drops with the element; just free the
            // closures.
            self.click.borrow_mut().take();
            self.trigger.borrow_mut().take();
        }
    }

    /// Resolve repo+branch and repaint the pill. With no repository
    /// ancestor there is nothing to show, so the host is cleared.
    ///
    /// State priority, highest first:
    ///   1. `offline` — no upstream, or an unreachable remote (a `None`
    ///      fetch). Nothing to sync *or* pause, so it overrides the
    ///      preference entirely (and the pill goes inert).
    ///   2. `paused`  — preference off and the remote reachable. Overrides
    ///      the real drift (`ahead` / `behind` / `diverged`).
    ///   3. `synced` / `syncing…` — running and reachable.
    ///
    /// The status is fetched on every refresh — even while paused —
    /// because only the fetch can tell `offline` from `paused`. The
    /// paused pill is painted optimistically first so it appears at once;
    /// the fetch then confirms it or overrides it with `offline`. A
    /// repaint never clears first, so a refresh holds the last good pill
    /// until the new one is ready.
    fn refresh_state(this: &HtmlElement) {
        let Some(repo) = repo_from_context(this) else {
            this.set_inner_html("");
            return;
        };
        // Optimistic: show `paused` immediately so the pill is present
        // without waiting on the network. The fetch below may override
        // it with `offline`.
        if !is_enabled(&repo) {
            paint(this, PAUSED_CHIP, PAUSED_LABEL);
        }
        let branch = branch_from_ancestor(this);
        let host = this.clone();
        spawn_local(async move {
            match fetch_sync_status(&repo, &branch).await {
                // No upstream: offer the "Enable sync" trigger instead of
                // a pill — a local-only branch can't sync until it has a
                // remote, so the actionable affordance is to add one.
                Some(SyncState::NoUpstream) => paint_enable_sync(&host),
                // Unreachable remote (a `None` fetch): there *is* an
                // upstream, but we can't reach it — the inert offline pill.
                None => paint(&host, OFFLINE_CHIP, OFFLINE_LABEL),
                // Reachable + running: the live state, click pauses.
                Some(state) if is_enabled(&repo) => {
                    paint(&host, state_chip(state), RUNNING_LABEL);
                }
                // Reachable + paused: the preference overrides real drift.
                Some(_) => paint(&host, PAUSED_CHIP, PAUSED_LABEL),
            }
        });
    }

    /// Resolve repo+branch and repaint the read-only badge. The
    /// read-only twin of [`refresh_state`]: same state priority, but it
    /// never offers the enable-sync trigger — a no-upstream or
    /// unreachable branch reads as `offline` (its `state_chip`). With no
    /// repository ancestor there is nothing to show, so the host clears.
    fn refresh_badge(this: &HtmlElement) {
        let Some(repo) = repo_from_context(this) else {
            this.set_inner_html("");
            return;
        };
        // Optimistic: show `paused` immediately so the badge is present
        // without waiting on the network. The fetch below may override
        // it with `offline`.
        if !is_enabled(&repo) {
            paint_badge(this, PAUSED_CHIP);
        }
        let branch = branch_from_ancestor(this);
        let host = this.clone();
        spawn_local(async move {
            match fetch_sync_status(&repo, &branch).await {
                // Unreachable remote: there *is* an upstream we can't
                // reach. No upstream at all also reads `offline` via its
                // chip below — the badge never offers an enable affordance.
                None => paint_badge(&host, OFFLINE_CHIP),
                Some(state) if is_enabled(&repo) => {
                    paint_badge(&host, state_chip(state));
                }
                Some(_) => paint_badge(&host, PAUSED_CHIP),
            }
        });
    }

    /// Paint the pill button: set its `is-*` modifier + label and the
    /// `title`/`aria-label`. The title is supplied by the caller — a
    /// toggle hint for the actionable states (`RUNNING_LABEL` /
    /// `PAUSED_LABEL`) or a plain status note for the inert `offline`
    /// pill (`OFFLINE_LABEL`). Clears any stale enable-sync trigger, so a
    /// branch that just gained an upstream swaps the trigger for the pill.
    fn paint(host: &HtmlElement, chip: (&'static str, &'static str), title: &str) {
        let (label, class) = chip;
        remove_child(host, ENABLE_BUTTON);
        if let Some(button) = ensure_pill_button(host) {
            let _ = button.set_attribute("class", &format!("{STATE_CHIP} {class}"));
            button.set_text_content(Some(label));
            let _ = button.set_attribute("title", title);
            let _ = button.set_attribute("aria-label", title);
        }
    }

    // ---- `<tonk-sync-badge>` --------------------------------------

    /// CSS class for the read-only status badge (with an `is-*` state
    /// modifier), styled by the consuming app — e.g. the `tonk-ui` Hub
    /// cards. Distinct from the interactive pill's [`STATE_CHIP`].
    const BADGE_CHIP: &str = "sync-badge";

    /// Paint the read-only status badge: set its `is-*` modifier and
    /// label on a non-interactive `<span>`. Unlike [`paint`], there is no
    /// pause/resume toggle and no enable-sync trigger — a no-upstream or
    /// unreachable branch reads as `offline` via its chip. The status
    /// rides on the `title`/`aria-label` so the dot-and-label badge is
    /// legible to assistive tech.
    fn paint_badge(host: &HtmlElement, chip: (&'static str, &'static str)) {
        let (label, class) = chip;
        if let Some(span) = ensure_badge_span(host) {
            let _ = span.set_attribute("class", &format!("{BADGE_CHIP} {class}"));
            span.set_text_content(Some(label));
            let _ = span.set_attribute("title", &format!("Sync status: {label}"));
            let _ = span.set_attribute("aria-label", &format!("Sync status: {label}"));
        }
    }

    /// Find or create the badge `<span>` as the element's only child. A
    /// plain span, not a button: the badge is a status indicator the
    /// surrounding card link owns the click for.
    fn ensure_badge_span(host: &HtmlElement) -> Option<Element> {
        if let Ok(Some(existing)) = host.query_selector(&format!(":scope > .{BADGE_CHIP}")) {
            return Some(existing);
        }
        let document = window()?.document()?;
        let span = document.create_element("span").ok()?;
        let _ = span.set_attribute("class", BADGE_CHIP);
        let _ = span.set_attribute("part", "badge");
        let _ = host.append_child(&span);
        Some(span)
    }

    /// Reveal the "Enable sync" trigger for a no-upstream branch,
    /// replacing any stale pill. The button opens the notation
    /// `#enable-sync` dialog through Web Awesome's `data-dialog`
    /// handler; the repo it attaches to is stamped into the dialog's
    /// hidden `name` input by [`install_trigger_click`] on click.
    fn paint_enable_sync(host: &HtmlElement) {
        remove_child(host, STATE_CHIP);
        let _ = ensure_enable_button(host);
    }

    /// Remove the element's only child matching `class`, if present.
    fn remove_child(host: &HtmlElement, class: &str) {
        if let Ok(Some(child)) = host.query_selector(&format!(":scope > .{class}")) {
            child.remove();
        }
    }

    /// Find or create the pill `<button>` as the element's only child.
    /// A real button so the whole pill is focusable and click-to-toggle;
    /// the host's click listener catches the bubble.
    fn ensure_pill_button(host: &HtmlElement) -> Option<Element> {
        if let Ok(Some(existing)) = host.query_selector(&format!(":scope > .{STATE_CHIP}")) {
            return Some(existing);
        }
        let document = window()?.document()?;
        let button = document.create_element("button").ok()?;
        let _ = button.set_attribute("class", STATE_CHIP);
        let _ = button.set_attribute("type", "button");
        let _ = button.set_attribute("part", "button");
        let _ = host.append_child(&button);
        Some(button)
    }

    /// Find or create the enable-sync trigger button as the element's
    /// only child. Carries `data-dialog="open enable-sync"` so a click
    /// opens the notation dialog (Web Awesome's global handler).
    fn ensure_enable_button(host: &HtmlElement) -> Option<Element> {
        if let Ok(Some(existing)) = host.query_selector(&format!(":scope > .{ENABLE_BUTTON}")) {
            return Some(existing);
        }
        let document = window()?.document()?;
        let button = document.create_element("button").ok()?;
        let _ = button.set_attribute("class", ENABLE_BUTTON);
        let _ = button.set_attribute("type", "button");
        let _ = button.set_attribute("part", "button");
        let _ = button.set_attribute("data-dialog", "open enable-sync");
        button.set_text_content(Some(ENABLE_LABEL));
        let _ = host.append_child(&button);
        Some(button)
    }

    /// Install the trigger's click listener on the host: stamp the repo
    /// resolved from the `with` ancestor into the notation
    /// enable-sync form's hidden `name` input, so the asserted command
    /// targets the right repository. Harmless when the element is
    /// showing a chip (the chip carries no `data-dialog`, so no dialog
    /// opens — the stamp is just unused).
    fn install_trigger_click(this: &HtmlElement, slot: &Listener) {
        let host = this.clone();
        let listener = Closure::wrap(Box::new(move |_event: Event| {
            if let Some(repo) = repo_from_context(&host) {
                stamp_enable_sync_repo(&repo);
            }
        }) as Box<dyn FnMut(Event)>);

        let _ = this.add_event_listener_with_callback("click", listener.as_ref().unchecked_ref());
        *slot.borrow_mut() = Some(listener);
    }

    /// Write `repo` into the notation enable-sync form's hidden `name`
    /// input. The input is a native `<input>`, but we set the `value`
    /// *property* (via reflection) since that's the slot the event
    /// layer reads on submit (`elements.name.value`).
    fn stamp_enable_sync_repo(repo: &str) {
        let Some(document) = window().and_then(|w| w.document()) else {
            return;
        };
        let Some(form) = document.get_element_by_id(ENABLE_SYNC_FORM_ID) else {
            return;
        };
        let Ok(Some(input)) = form.query_selector("[name=\"name\"]") else {
            return;
        };
        let input_js: &JsValue = input.as_ref();
        let _ = js_sys::Reflect::set(
            input_js,
            &JsValue::from_str("value"),
            &JsValue::from_str(repo),
        );
    }

    /// Install the window listeners shared by the pill and the badge:
    /// `tonk:status-refresh` (ignored unless its `detail` repo matches
    /// this element's) and `tonk:committed` (always). Both re-fetch and
    /// repaint through the caller's `refresh` function — `refresh_state`
    /// for the interactive pill, `refresh_badge` for the read-only badge.
    fn install_state_listeners(
        this: &HtmlElement,
        refresh_slot: &Listener,
        committed_slot: &Listener,
        refresh: fn(&HtmlElement),
    ) {
        let Some(win) = window() else {
            return;
        };

        let host = this.clone();
        let refresh_listener = Closure::wrap(Box::new(move |event: Event| {
            let event_repo = event
                .dyn_ref::<CustomEvent>()
                .and_then(|e| e.detail().as_string());
            if let (Some(this_repo), Some(event_repo)) = (repo_from_context(&host), event_repo)
                && this_repo == event_repo
            {
                refresh(&host);
            }
        }) as Box<dyn FnMut(Event)>);
        let _ = win.add_event_listener_with_callback(
            STATUS_REFRESH_EVENT,
            refresh_listener.as_ref().unchecked_ref(),
        );
        *refresh_slot.borrow_mut() = Some(refresh_listener);

        let host = this.clone();
        let committed_listener = Closure::wrap(Box::new(move |_event: Event| {
            refresh(&host);
        }) as Box<dyn FnMut(Event)>);
        let _ = win.add_event_listener_with_callback(
            COMMITTED_EVENT,
            committed_listener.as_ref().unchecked_ref(),
        );
        *committed_slot.borrow_mut() = Some(committed_listener);
    }

    /// Per-element state for `<tonk-sync-badge>` — the read-only status
    /// indicator. It carries only the window listeners (no click toggle
    /// or enable-sync trigger): the surrounding context owns the click.
    #[derive(Default)]
    pub(crate) struct TonkSyncBadge {
        refresh: Listener,
        committed: Listener,
    }

    impl CustomElement for TonkSyncBadge {
        fn shadow() -> bool {
            false
        }

        fn observed_attributes() -> &'static [&'static str] {
            &[]
        }

        fn inject_children(&mut self, _this: &HtmlElement) {}

        fn connected_callback(&mut self, this: &HtmlElement) {
            refresh_badge(this);
            install_state_listeners(this, &self.refresh, &self.committed, refresh_badge);
        }

        fn disconnected_callback(&mut self, _this: &HtmlElement) {
            if let Some(win) = window() {
                if let Some(listener) = self.refresh.borrow().as_ref() {
                    let _ = win.remove_event_listener_with_callback(
                        STATUS_REFRESH_EVENT,
                        listener.as_ref().unchecked_ref(),
                    );
                }
                if let Some(listener) = self.committed.borrow().as_ref() {
                    let _ = win.remove_event_listener_with_callback(
                        COMMITTED_EVENT,
                        listener.as_ref().unchecked_ref(),
                    );
                }
            }
            self.refresh.borrow_mut().take();
            self.committed.borrow_mut().take();
        }
    }

    /// Register the elements. Idempotent.
    pub(crate) fn register() {
        let Some(elements) = window().map(|w| w.custom_elements()) else {
            return;
        };
        if elements.get("tonk-sync-state").is_undefined() {
            TonkSyncState::define("tonk-sync-state");
        }
        if elements.get("tonk-sync-badge").is_undefined() {
            TonkSyncBadge::define("tonk-sync-badge");
        }
    }

    #[cfg(test)]
    mod dom_tests {
        use super::*;
        use wasm_bindgen_test::wasm_bindgen_test_configure;

        wasm_bindgen_test_configure!(run_in_browser);

        #[dialog_common::test]
        async fn it_shows_paused_on_connect_and_toggles_the_preference_on_click() {
            register();
            let document = window().unwrap().document().unwrap();
            let body = document.body().unwrap();
            let storage = window().unwrap().local_storage().unwrap().unwrap();
            let key = "tonk:auto-sync:toggletest";
            // Start paused so the pill paints synchronously on connect —
            // the running branch would await a status fetch that never
            // resolves in this DOM-only test (no service worker).
            storage.set_item(key, "off").unwrap();

            let state = document.create_element("tonk-sync-state").unwrap();
            state.set_attribute("with", "main@toggletest").unwrap();
            // Defined element → connectedCallback runs synchronously on
            // append, so the pill is present by the time it returns.
            body.append_child(&state).unwrap();

            let button = state
                .query_selector(".workspace__sync")
                .unwrap()
                .expect("pill injected on connect");

            // Paused: the neutral `is-paused` pill reading "paused",
            // labelled for the resume action.
            assert_eq!(button.text_content().as_deref(), Some("paused"));
            assert_eq!(button.class_name(), "workspace__sync is-paused");
            assert_eq!(button.get_attribute("title").as_deref(), Some(PAUSED_LABEL));

            // Click → running: the preference flips to "on". (The running
            // pill awaits a status fetch that can't resolve here, so we
            // assert on the persisted preference — the load-bearing side
            // effect — rather than the repainted label.)
            button.dyn_ref::<HtmlElement>().unwrap().click();
            assert_eq!(storage.get_item(key).unwrap().as_deref(), Some("on"));

            // Click again → paused: flips back and repaints synchronously.
            button.dyn_ref::<HtmlElement>().unwrap().click();
            assert_eq!(storage.get_item(key).unwrap().as_deref(), Some("off"));
            assert_eq!(button.text_content().as_deref(), Some("paused"));

            state.remove();
            let _ = storage.remove_item(key);
        }

        #[dialog_common::test]
        async fn it_ignores_clicks_while_offline() {
            register();
            let document = window().unwrap().document().unwrap();
            let body = document.body().unwrap();
            let storage = window().unwrap().local_storage().unwrap().unwrap();
            let key = "tonk:auto-sync:offlinetest";
            // Running (on) — `offline` is a *running* sub-state.
            storage.set_item(key, "on").unwrap();

            let state = document.create_element("tonk-sync-state").unwrap();
            state.set_attribute("with", "main@offlinetest").unwrap();
            body.append_child(&state).unwrap();

            // Force the offline pill (the connect-time fetch can't resolve
            // in this DOM-only test, so paint it directly).
            let host_el = state.dyn_ref::<HtmlElement>().unwrap();
            paint(host_el, OFFLINE_CHIP, OFFLINE_LABEL);
            let button = state
                .query_selector(".workspace__sync")
                .unwrap()
                .expect("offline pill present");
            assert_eq!(button.class_name(), "workspace__sync is-offline");

            // Click → no-op: there's nothing to pause, so the preference
            // stays "on" and the pill stays offline.
            button.dyn_ref::<HtmlElement>().unwrap().click();
            assert_eq!(storage.get_item(key).unwrap().as_deref(), Some("on"));
            assert_eq!(button.class_name(), "workspace__sync is-offline");

            state.remove();
            let _ = storage.remove_item(key);
        }

        #[dialog_common::test]
        async fn it_paints_each_running_state_on_the_pill() {
            let document = window().unwrap().document().unwrap();
            let body = document.body().unwrap();
            let host = document.create_element("tonk-sync-state").unwrap();
            body.append_child(&host).unwrap();
            let host_el = host.dyn_ref::<HtmlElement>().unwrap();

            // Synced → green "synced" pill, filled dot (via the class).
            paint(host_el, state_chip(SyncState::Synced), RUNNING_LABEL);
            let button = host
                .query_selector(".workspace__sync")
                .unwrap()
                .expect("pill painted for a concrete state");
            assert_eq!(button.text_content().as_deref(), Some("synced"));
            assert_eq!(button.class_name(), "workspace__sync is-synced");

            // Any drift collapses to the single "syncing…" state.
            paint(host_el, state_chip(SyncState::Behind), RUNNING_LABEL);
            assert_eq!(button.text_content().as_deref(), Some("syncing…"));
            assert_eq!(button.class_name(), "workspace__sync is-syncing");

            // NoUpstream → "offline" (same pill as an unreachable remote).
            paint(host_el, state_chip(SyncState::NoUpstream), OFFLINE_LABEL);
            assert_eq!(button.text_content().as_deref(), Some("offline"));
            assert_eq!(button.class_name(), "workspace__sync is-offline");

            host.remove();
        }

        #[dialog_common::test]
        async fn it_paints_a_read_only_badge_that_does_not_toggle_on_click() {
            register();
            let document = window().unwrap().document().unwrap();
            let body = document.body().unwrap();
            let storage = window().unwrap().local_storage().unwrap().unwrap();
            let key = "tonk:auto-sync:badgetest";
            // Start paused so the badge paints synchronously on connect —
            // the running branch would await a status fetch that never
            // resolves in this DOM-only test (no service worker).
            storage.set_item(key, "off").unwrap();

            let badge = document.create_element("tonk-sync-badge").unwrap();
            badge.set_attribute("with", "main@badgetest").unwrap();
            body.append_child(&badge).unwrap();

            let span = badge
                .query_selector(".sync-badge")
                .unwrap()
                .expect("badge injected on connect");
            assert_eq!(span.text_content().as_deref(), Some("paused"));
            assert_eq!(span.class_name(), "sync-badge is-paused");

            // Read-only: clicking the badge must NOT flip the preference
            // (the surrounding card link owns the click). The pill toggles
            // here; the badge must not.
            span.dyn_ref::<HtmlElement>().unwrap().click();
            badge.dyn_ref::<HtmlElement>().unwrap().click();
            assert_eq!(
                storage.get_item(key).unwrap().as_deref(),
                Some("off"),
                "the read-only badge must not toggle the auto-sync preference",
            );
            assert_eq!(span.text_content().as_deref(), Some("paused"));

            badge.remove();
            let _ = storage.remove_item(key);
        }

        #[dialog_common::test]
        async fn it_paints_each_state_on_the_read_only_badge() {
            let document = window().unwrap().document().unwrap();
            let body = document.body().unwrap();
            let host = document.create_element("tonk-sync-badge").unwrap();
            body.append_child(&host).unwrap();
            let host_el = host.dyn_ref::<HtmlElement>().unwrap();

            // Synced → a non-interactive "synced" span (read-only: a
            // `<span>`, never the pill's `<button>`).
            paint_badge(host_el, state_chip(SyncState::Synced));
            let span = host
                .query_selector(".sync-badge")
                .unwrap()
                .expect("badge painted for a concrete state");
            assert_eq!(
                span.tag_name().to_lowercase(),
                "span",
                "the read-only badge must be a span, not an interactive button",
            );
            assert_eq!(span.text_content().as_deref(), Some("synced"));
            assert_eq!(span.class_name(), "sync-badge is-synced");

            // Any drift collapses to the single "syncing…" state.
            paint_badge(host_el, state_chip(SyncState::Behind));
            assert_eq!(span.text_content().as_deref(), Some("syncing…"));
            assert_eq!(span.class_name(), "sync-badge is-syncing");

            // The paused preference paints the muted "paused" badge.
            paint_badge(host_el, PAUSED_CHIP);
            assert_eq!(span.text_content().as_deref(), Some("paused"));
            assert_eq!(span.class_name(), "sync-badge is-paused");

            // No upstream → "offline" (same badge as an unreachable
            // remote); the read-only badge never offers an enable-sync
            // trigger.
            paint_badge(host_el, state_chip(SyncState::NoUpstream));
            assert_eq!(span.text_content().as_deref(), Some("offline"));
            assert_eq!(span.class_name(), "sync-badge is-offline");

            host.remove();
        }

        #[dialog_common::test]
        async fn it_reveals_the_enable_sync_trigger_and_swaps_with_the_pill() {
            let document = window().unwrap().document().unwrap();
            let body = document.body().unwrap();
            let host = document.create_element("tonk-sync-state").unwrap();
            body.append_child(&host).unwrap();
            let host_el = host.dyn_ref::<HtmlElement>().unwrap();

            // No upstream → an "Enable sync" trigger that opens the
            // notation dialog (`data-dialog`), not a pill.
            paint_enable_sync(host_el);
            let button = host
                .query_selector(".workspace__enable-sync")
                .unwrap()
                .expect("enable-sync trigger painted for no-upstream");
            assert_eq!(button.text_content().as_deref(), Some("Enable sync"));
            assert_eq!(
                button.get_attribute("data-dialog").as_deref(),
                Some("open enable-sync"),
            );

            // A concrete state replaces the trigger with the pill — no
            // stale button left behind.
            paint(host_el, state_chip(SyncState::Synced), RUNNING_LABEL);
            assert!(
                host.query_selector(".workspace__enable-sync")
                    .unwrap()
                    .is_none(),
                "painting the pill must remove the enable-sync trigger",
            );
            assert!(host.query_selector(".workspace__sync").unwrap().is_some());

            host.remove();
        }

        #[dialog_common::test]
        async fn it_stamps_the_resolved_repo_into_the_enable_sync_form_on_click() {
            register();
            let document = window().unwrap().document().unwrap();
            let body = document.body().unwrap();

            // The notation form the element stamps: a hidden `name`
            // input inside `#enable-sync-form`.
            let form = document.create_element("form").unwrap();
            form.set_id("enable-sync-form");
            let hidden = document.create_element("input").unwrap();
            hidden.set_attribute("type", "hidden").unwrap();
            hidden.set_attribute("name", "name").unwrap();
            form.append_child(&hidden).unwrap();
            body.append_child(&form).unwrap();

            // <tonk-sync-state with="main@pictures"> — the consumer carries
            // its own routing context.
            let state = document.create_element("tonk-sync-state").unwrap();
            state.set_attribute("with", "main@pictures").unwrap();
            body.append_child(&state).unwrap();

            // Clicking the element resolves the ancestor repo and stamps
            // it into the form's hidden `name` input.
            state.dyn_ref::<HtmlElement>().unwrap().click();

            let value = js_sys::Reflect::get(hidden.as_ref(), &JsValue::from_str("value"))
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_default();
            assert_eq!(
                value, "pictures",
                "click should stamp the ancestor repo into the enable-sync form"
            );

            form.remove();
            state.remove();
        }
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) use dom::register;

#[cfg(test)]
mod tests {
    use super::logic::*;
    use tonk_schema::SyncState;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn it_maps_synced_to_the_synced_chip() {
        assert_eq!(state_chip(SyncState::Synced), ("synced", "is-synced"));
    }

    #[dialog_common::test]
    fn it_collapses_ahead_to_the_syncing_chip() {
        assert_eq!(state_chip(SyncState::Ahead), ("syncing…", "is-syncing"));
    }

    #[dialog_common::test]
    fn it_collapses_behind_to_the_syncing_chip() {
        assert_eq!(state_chip(SyncState::Behind), ("syncing…", "is-syncing"));
    }

    #[dialog_common::test]
    fn it_collapses_diverged_to_the_syncing_chip() {
        assert_eq!(state_chip(SyncState::Diverged), ("syncing…", "is-syncing"));
    }

    #[dialog_common::test]
    fn it_maps_no_upstream_to_the_offline_chip() {
        assert_eq!(state_chip(SyncState::NoUpstream), OFFLINE_CHIP);
        assert_eq!(OFFLINE_CHIP, ("offline", "is-offline"));
    }

    #[dialog_common::test]
    fn it_labels_the_paused_chip() {
        assert_eq!(PAUSED_CHIP, ("paused", "is-paused"));
    }

    #[dialog_common::test]
    fn it_defaults_the_preference_to_enabled() {
        assert!(pref_is_enabled(None));
    }

    #[dialog_common::test]
    fn it_disables_the_preference_only_for_an_explicit_off() {
        assert!(!pref_is_enabled(Some("off")));
    }

    #[dialog_common::test]
    fn it_keeps_the_preference_enabled_for_on_or_unrecognized() {
        assert!(pref_is_enabled(Some("on")));
        assert!(pref_is_enabled(Some("anything-else")));
    }

    #[dialog_common::test]
    fn it_builds_the_per_repository_preference_key() {
        assert_eq!(pref_key("pictures"), "tonk:auto-sync:pictures");
    }

    #[dialog_common::test]
    fn it_defaults_a_missing_branch_to_main() {
        assert_eq!(branch_or_default(None), "main");
    }

    #[dialog_common::test]
    fn it_defaults_an_empty_branch_to_main() {
        assert_eq!(branch_or_default(Some(String::new())), "main");
    }

    #[dialog_common::test]
    fn it_keeps_a_named_branch() {
        assert_eq!(branch_or_default(Some("draft".to_string())), "draft");
    }
}
