//! Background-sync chrome for the workspace top bar.
//!
//! Two dumb light-DOM custom elements, cut from the same cloth as
//! [`super::share`]: they resolve their repo/branch from the nearest
//! `<tonk-repository>` / `<tonk-branch>` ancestors and otherwise hold
//! no app policy.
//!
//! - `<tonk-sync-toggle>` pauses/resumes background sync by flipping
//!   the per-repository `tonk:auto-sync:{repo}` `localStorage`
//!   preference — the exact key the Leptos `sync_controller` reads.
//! - `<tonk-sync-state>` paints a live `synced` / `ahead` / `behind` /
//!   `diverged` chip for the branch in scope relative to its remote,
//!   re-reading the read-only `sync/status` route whenever the
//!   controller dispatches `tonk:status-refresh` or a commit lands.
//!
//! The two coordinate with the controller only through that shared
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

    /// Label and modifier class for a sync state, or `None` for states
    /// with nothing to show (`NoUpstream`). A pure mirror of
    /// `tonk-ui`'s `upstream_state_chip`, with the workspace's own
    /// `is-*` modifier classes in place of the `<wa-badge>` variants.
    pub(crate) fn state_chip(state: SyncState) -> Option<(&'static str, &'static str)> {
        match state {
            SyncState::Synced => Some(("synced", "is-synced")),
            SyncState::Ahead => Some(("ahead", "is-ahead")),
            SyncState::Behind => Some(("behind", "is-behind")),
            SyncState::Diverged => Some(("diverged", "is-diverged")),
            SyncState::NoUpstream => None,
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
    use wasm_bindgen_futures::{JsFuture, spawn_local};
    use web_sys::{
        CustomEvent, CustomEventInit, Element, Event, HtmlElement, Request, RequestInit, Response,
        window,
    };

    use tonk_schema::SyncState;

    use super::logic::{branch_or_default, pref_is_enabled, pref_key, state_chip};
    use crate::ancestors::repo_from_ancestor;

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

    /// Read the `name` of the nearest `<tonk-branch>` ancestor,
    /// defaulting to `main` when absent or empty.
    fn branch_from_ancestor(this: &HtmlElement) -> String {
        let name = this
            .closest("tonk-branch")
            .ok()
            .flatten()
            .and_then(|el| el.get_attribute("name"));
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
        let win = window()?;
        let origin = win.location().origin().ok()?;
        let url = format!("{origin}/api/repository/{repo}/branch/{branch}/sync/status");

        let init = RequestInit::new();
        init.set_method("GET");
        let request = Request::new_with_str_and_init(&url, &init).ok()?;

        let resp_value = JsFuture::from(win.fetch_with_request(&request))
            .await
            .ok()?;
        let resp: Response = resp_value.dyn_into().ok()?;
        if !resp.ok() {
            tonk_common::log!("sync status: GET {url} -> {}", resp.status());
            return None;
        }
        let text = JsFuture::from(resp.text().ok()?).await.ok()?.as_string()?;
        // Read just the `state` field; the rest of the response (local /
        // remote revisions) is the inspector's concern.
        let value: serde_json::Value = match serde_json::from_str(&text) {
            Ok(value) => value,
            Err(err) => {
                tonk_common::log!("sync status: malformed JSON from {url}: {err}");
                return None;
            }
        };
        let Some(state) = value.get("state") else {
            tonk_common::log!("sync status: response from {url} missing `state` field");
            return None;
        };
        match serde_json::from_value::<SyncState>(state.clone()) {
            Ok(state) => Some(state),
            Err(err) => {
                tonk_common::log!("sync status: unrecognized `state` from {url}: {err}");
                None
            }
        }
    }

    /// Dispatch [`STATUS_REFRESH_EVENT`] on `window` with `repo` in the
    /// detail, so the chip re-reads status immediately after a toggle.
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

    // ---- `<tonk-sync-toggle>` -------------------------------------

    /// CSS class the consuming workspace view styles for the toggle.
    const TOGGLE_BUTTON: &str = "workspace__sync-toggle";

    /// Title/`aria-label` while auto-sync is running. Mirrors tonk-ui.
    const RUNNING_LABEL: &str = "Auto-sync on — click to pause";
    /// Title/`aria-label` while auto-sync is paused. Mirrors tonk-ui.
    const PAUSED_LABEL: &str = "Auto-sync paused — click to resume";

    /// Inline rotating-arrows glyph (running) — the icon *reflects the
    /// active mode*, mirroring tonk-ui's inspector toggle
    /// (`arrows-rotate`). Drawn with `currentColor`.
    const SYNC_GLYPH: &str = r#"<svg class="workspace__sync-toggle-glyph" viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="23 4 23 10 17 10"></polyline><polyline points="1 20 1 14 7 14"></polyline><path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"></path></svg>"#;
    /// Inline pause glyph (paused). Drawn with `currentColor`.
    const PAUSE_GLYPH: &str = r#"<svg class="workspace__sync-toggle-glyph" viewBox="0 0 24 24" aria-hidden="true" fill="currentColor"><rect x="6" y="5" width="4" height="14"></rect><rect x="14" y="5" width="4" height="14"></rect></svg>"#;

    /// Per-element state for `<tonk-sync-toggle>`.
    #[derive(Default)]
    pub(crate) struct TonkSyncToggle {
        click: Listener,
    }

    impl CustomElement for TonkSyncToggle {
        fn shadow() -> bool {
            false
        }

        fn observed_attributes() -> &'static [&'static str] {
            &[]
        }

        fn inject_children(&mut self, _this: &HtmlElement) {}

        fn connected_callback(&mut self, this: &HtmlElement) {
            if let Some(button) = ensure_toggle_button(this) {
                let repo = repo_from_ancestor(this).unwrap_or_default();
                paint_toggle(&button, is_enabled(&repo));
            }
            install_toggle_click(this, &self.click);
        }

        fn disconnected_callback(&mut self, _this: &HtmlElement) {
            self.click.borrow_mut().take();
        }
    }

    /// Find or create the toggle button as the element's only child.
    fn ensure_toggle_button(this: &HtmlElement) -> Option<Element> {
        if let Ok(Some(existing)) = this.query_selector(&format!(":scope > .{TOGGLE_BUTTON}")) {
            return Some(existing);
        }
        let document = window()?.document()?;
        let button = document.create_element("button").ok()?;
        let _ = button.set_attribute("class", TOGGLE_BUTTON);
        let _ = button.set_attribute("type", "button");
        let _ = button.set_attribute("part", "button");
        let _ = this.append_child(&button);
        Some(button)
    }

    /// Paint the toggle button's glyph + title/`aria-label` for the
    /// current state. The glyph reflects the active mode: rotating
    /// sync-arrows + "click to pause" when running; pause bars +
    /// "click to resume" when paused.
    fn paint_toggle(button: &Element, enabled: bool) {
        let (glyph, label) = if enabled {
            (SYNC_GLYPH, RUNNING_LABEL)
        } else {
            (PAUSE_GLYPH, PAUSED_LABEL)
        };
        button.set_inner_html(glyph);
        let _ = button.set_attribute("title", label);
        let _ = button.set_attribute("aria-label", label);
    }

    /// Install the toggle's click listener: flip the preference and, if
    /// the write lands, repaint and ask the chip to refresh. A rejected
    /// write leaves the icon as-is — the controller still reads the old
    /// preference, so UI and behaviour stay consistent.
    fn install_toggle_click(this: &HtmlElement, slot: &Listener) {
        let host = this.clone();
        let listener = Closure::wrap(Box::new(move |_event: Event| {
            let repo = repo_from_ancestor(&host).unwrap_or_default();
            let next = !is_enabled(&repo);
            if set_enabled(&repo, next) {
                if let Ok(Some(button)) = host.query_selector(&format!(":scope > .{TOGGLE_BUTTON}"))
                {
                    paint_toggle(&button, next);
                }
                request_status_refresh(&repo);
            }
        }) as Box<dyn FnMut(Event)>);

        let _ = this.add_event_listener_with_callback("click", listener.as_ref().unchecked_ref());
        *slot.borrow_mut() = Some(listener);
    }

    // ---- `<tonk-sync-state>` --------------------------------------

    /// CSS class for the chip span (with an `is-*` state modifier).
    const STATE_CHIP: &str = "workspace__sync";

    /// Per-element state for `<tonk-sync-state>`.
    #[derive(Default)]
    pub(crate) struct TonkSyncState {
        refresh: Listener,
        committed: Listener,
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
            install_state_listeners(this, &self.refresh, &self.committed);
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

    /// Resolve repo+branch and refresh the chip. With no repository
    /// ancestor there is nothing to show, so the chip is hidden. With a
    /// repo, fetch the status and paint; a fetch failure leaves the chip
    /// as-is rather than flashing error chrome — holding the last good
    /// value in steady state, or staying empty on a first-load failure
    /// until the next refresh event (the controller's ≤20s sweep fires
    /// one, so a blank chip self-heals).
    fn refresh_state(this: &HtmlElement) {
        let Some(repo) = repo_from_ancestor(this) else {
            paint_chip(this, None);
            return;
        };
        let branch = branch_from_ancestor(this);
        let host = this.clone();
        spawn_local(async move {
            if let Some(state) = fetch_sync_status(&repo, &branch).await {
                paint_chip(&host, state_chip(state));
            }
        });
    }

    /// Paint (or hide) the chip. `None` — no repo or `NoUpstream` —
    /// clears the host so it takes no space; `Some((label, class))`
    /// renders a single `<span class="workspace__sync {class}">label</span>`.
    fn paint_chip(host: &HtmlElement, chip: Option<(&'static str, &'static str)>) {
        match chip {
            None => host.set_inner_html(""),
            Some((label, class)) => {
                if let Some(span) = ensure_chip_span(host) {
                    let _ = span.set_attribute("class", &format!("{STATE_CHIP} {class}"));
                    span.set_text_content(Some(label));
                }
            }
        }
    }

    /// Find or create the chip span as the element's only child.
    fn ensure_chip_span(host: &HtmlElement) -> Option<Element> {
        if let Ok(Some(existing)) = host.query_selector(&format!(":scope > .{STATE_CHIP}")) {
            return Some(existing);
        }
        let document = window()?.document()?;
        let span = document.create_element("span").ok()?;
        let _ = span.set_attribute("class", STATE_CHIP);
        let _ = host.append_child(&span);
        Some(span)
    }

    /// Install the chip's window listeners: `tonk:status-refresh`
    /// (ignored unless its `detail` repo matches this element's) and
    /// `tonk:committed` (always). Both re-fetch and repaint.
    fn install_state_listeners(
        this: &HtmlElement,
        refresh_slot: &Listener,
        committed_slot: &Listener,
    ) {
        let Some(win) = window() else {
            return;
        };

        let host = this.clone();
        let refresh_listener = Closure::wrap(Box::new(move |event: Event| {
            let event_repo = event
                .dyn_ref::<CustomEvent>()
                .and_then(|e| e.detail().as_string());
            if let (Some(this_repo), Some(event_repo)) = (repo_from_ancestor(&host), event_repo)
                && this_repo == event_repo
            {
                refresh_state(&host);
            }
        }) as Box<dyn FnMut(Event)>);
        let _ = win.add_event_listener_with_callback(
            STATUS_REFRESH_EVENT,
            refresh_listener.as_ref().unchecked_ref(),
        );
        *refresh_slot.borrow_mut() = Some(refresh_listener);

        let host = this.clone();
        let committed_listener = Closure::wrap(Box::new(move |_event: Event| {
            refresh_state(&host);
        }) as Box<dyn FnMut(Event)>);
        let _ = win.add_event_listener_with_callback(
            COMMITTED_EVENT,
            committed_listener.as_ref().unchecked_ref(),
        );
        *committed_slot.borrow_mut() = Some(committed_listener);
    }

    /// Register both elements. Idempotent.
    pub(crate) fn register() {
        let Some(elements) = window().map(|w| w.custom_elements()) else {
            return;
        };
        if elements.get("tonk-sync-toggle").is_undefined() {
            TonkSyncToggle::define("tonk-sync-toggle");
        }
        if elements.get("tonk-sync-state").is_undefined() {
            TonkSyncState::define("tonk-sync-state");
        }
    }

    #[cfg(test)]
    mod dom_tests {
        use super::*;
        use wasm_bindgen_test::wasm_bindgen_test_configure;

        wasm_bindgen_test_configure!(run_in_browser);

        #[dialog_common::test]
        async fn it_toggles_the_preference_and_swaps_the_icon() {
            register();
            let document = window().unwrap().document().unwrap();
            let body = document.body().unwrap();
            let storage = window().unwrap().local_storage().unwrap().unwrap();
            let key = "tonk:auto-sync:toggletest";
            let _ = storage.remove_item(key);

            let repo = document.create_element("tonk-repository").unwrap();
            repo.set_attribute("name", "toggletest").unwrap();
            let toggle = document.create_element("tonk-sync-toggle").unwrap();
            repo.append_child(&toggle).unwrap();
            // Defined element → connectedCallback runs synchronously on
            // append, so the button is present by the time it returns.
            body.append_child(&repo).unwrap();

            let button = toggle
                .query_selector(".workspace__sync-toggle")
                .unwrap()
                .expect("toggle button injected on connect");

            // Default on: running label + the sync-arrows glyph
            // (distinguished by its `polyline` arrowheads).
            assert_eq!(
                button.get_attribute("title").as_deref(),
                Some(RUNNING_LABEL)
            );
            assert!(button.inner_html().contains("polyline"));

            // Click → paused: preference flips to off, label + glyph swap
            // to the pause bars (`rect`s).
            button.dyn_ref::<HtmlElement>().unwrap().click();
            assert_eq!(storage.get_item(key).unwrap().as_deref(), Some("off"));
            assert_eq!(button.get_attribute("title").as_deref(), Some(PAUSED_LABEL));
            assert!(button.inner_html().contains("rect"));

            // Click again → running.
            button.dyn_ref::<HtmlElement>().unwrap().click();
            assert_eq!(storage.get_item(key).unwrap().as_deref(), Some("on"));
            assert_eq!(
                button.get_attribute("title").as_deref(),
                Some(RUNNING_LABEL)
            );

            repo.remove();
            let _ = storage.remove_item(key);
        }

        #[dialog_common::test]
        async fn it_paints_the_chip_for_a_state_and_hides_on_no_upstream() {
            let document = window().unwrap().document().unwrap();
            let body = document.body().unwrap();
            let host = document.create_element("tonk-sync-state").unwrap();
            body.append_child(&host).unwrap();
            let host_el = host.dyn_ref::<HtmlElement>().unwrap();

            // A concrete state renders the label + single modifier class.
            paint_chip(host_el, state_chip(SyncState::Ahead));
            let span = host
                .query_selector(".workspace__sync")
                .unwrap()
                .expect("chip painted for a concrete state");
            assert_eq!(span.text_content().as_deref(), Some("ahead"));
            assert_eq!(span.class_name(), "workspace__sync is-ahead");

            // NoUpstream → nothing to show → host cleared.
            paint_chip(host_el, state_chip(SyncState::NoUpstream));
            assert!(host.query_selector(".workspace__sync").unwrap().is_none());

            // No repo (None) hides the same way.
            paint_chip(host_el, state_chip(SyncState::Behind));
            paint_chip(host_el, None);
            assert!(host.query_selector(".workspace__sync").unwrap().is_none());

            host.remove();
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
        assert_eq!(state_chip(SyncState::Synced), Some(("synced", "is-synced")));
    }

    #[dialog_common::test]
    fn it_maps_ahead_to_the_ahead_chip() {
        assert_eq!(state_chip(SyncState::Ahead), Some(("ahead", "is-ahead")));
    }

    #[dialog_common::test]
    fn it_maps_behind_to_the_behind_chip() {
        assert_eq!(state_chip(SyncState::Behind), Some(("behind", "is-behind")));
    }

    #[dialog_common::test]
    fn it_maps_diverged_to_the_diverged_chip() {
        assert_eq!(
            state_chip(SyncState::Diverged),
            Some(("diverged", "is-diverged"))
        );
    }

    #[dialog_common::test]
    fn it_hides_the_chip_for_no_upstream() {
        assert_eq!(state_chip(SyncState::NoUpstream), None);
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
