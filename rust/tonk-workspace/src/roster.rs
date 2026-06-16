//! `<tonk-roster>` — a compact avatar cluster for the workspace top
//! bar. Resolves its repo from the nearest `<tonk-repository>`
//! ancestor (the same walk `<tonk-share>`/`<tonk-sync-state>` use),
//! fetches the repository info, and renders one `<tonk-sigil>` per
//! member. Dumb: it holds no state and emits no events.

#[cfg(any(target_arch = "wasm32", test))]
use serde::Deserialize;

/// A member as the roster renders it — the subset of the worker's
/// `MemberInfo` this element needs. Decoupled from `tonk-worker` so
/// the workspace crate need not depend on it.
#[cfg(any(target_arch = "wasm32", test))]
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct RosterMember {
    pub did: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub is_self: bool,
}

/// Parse the `members` array out of a `RepositoryInfo` JSON body.
/// Returns an empty vec on malformed input or a missing `members`
/// field (a repo that predates roster writes).
#[cfg(any(target_arch = "wasm32", test))]
pub fn members_from_repository_info(json: &str) -> Vec<RosterMember> {
    #[derive(Deserialize)]
    struct Info {
        #[serde(default)]
        members: Vec<RosterMember>,
    }
    serde_json::from_str::<Info>(json)
        .map(|i| i.members)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn it_parses_members_from_repository_info() {
        let json = r#"{
            "name":"k","label":"L","subject":"did:key:zSub",
            "operator":"did:key:zOp","profile":"did:key:zPr",
            "members":[
                {"did":"did:key:zA","name":"Alice","is_self":true},
                {"did":"did:key:zB","is_self":false}
            ]
        }"#;
        let members = members_from_repository_info(json);
        assert_eq!(members.len(), 2);
        assert_eq!(members[0].name.as_deref(), Some("Alice"));
        assert!(members[0].is_self);
        assert_eq!(members[1].name, None);
    }

    #[dialog_common::test]
    fn it_returns_empty_when_members_absent() {
        let json = r#"{"name":"k","label":"L"}"#;
        assert!(members_from_repository_info(json).is_empty());
    }
}

// The wasm-only custom element that consumes the parser above. Light
// DOM, in the `<tonk-share>` / `<tonk-sync-state>` mold: resolve the
// repo from the `<tonk-repository>` ancestor, hold no app policy.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod element {
    use std::cell::RefCell;
    use std::rc::Rc;

    use custom_elements::CustomElement;
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::{JsFuture, spawn_local};
    use web_sys::{CustomEvent, Event, HtmlElement, Request, RequestInit, Response, window};

    use super::{RosterMember, members_from_repository_info};
    use crate::ancestors::repo_from_ancestor;

    /// CSS class for the avatar-cluster container.
    const ROSTER_CLASS: &str = "workspace__roster";

    /// CSS class for an individual `<tonk-sigil>` avatar.
    const AVATAR_CLASS: &str = "workspace__roster-avatar";

    /// Window event the sync controller dispatches to ask the roster to
    /// re-read its members. Repo name rides in `detail`.
    const STATUS_REFRESH_EVENT: &str = "tonk:status-refresh";

    /// Window event a local commit dispatches; the roster re-reads on it
    /// so a fresh write reflects immediately.
    const COMMITTED_EVENT: &str = "tonk:committed";

    /// A retained listener closure, kept alive for the element's
    /// lifetime so the listener stays valid; dropped on disconnect.
    type Listener = Rc<RefCell<Option<Closure<dyn FnMut(Event)>>>>;

    /// Fetch the repository info for `repo` and return its members, or
    /// `None` on any failure. Gated on service-worker readiness so a
    /// cold-start call doesn't land on the asset server.
    ///
    /// The cluster degrades to empty on `None`, so the cold-start
    /// failures stay quiet; a non-200 likewise just leaves the cluster
    /// empty (a repo the caller can't read has no roster to show).
    async fn fetch_members(repo: &str) -> Option<Vec<RosterMember>> {
        tonk_host::ready::wait().await;
        let win = window()?;
        let origin = win.location().origin().ok()?;
        let url = format!("{origin}/api/repository/{repo}");

        let init = RequestInit::new();
        init.set_method("GET");
        let request = Request::new_with_str_and_init(&url, &init).ok()?;

        let resp_value = JsFuture::from(win.fetch_with_request(&request))
            .await
            .ok()?;
        let resp: Response = resp_value.dyn_into().ok()?;
        if !resp.ok() {
            return None;
        }
        let body = JsFuture::from(resp.text().ok()?).await.ok()?.as_string()?;
        Some(members_from_repository_info(&body))
    }

    /// Minimal HTML-attribute escaping for the user-controlled member
    /// name, which is interpolated into the `title` attribute below
    /// before the cluster is set via `set_inner_html`.
    fn escape_attr(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }

    /// Render a row of `<tonk-sigil value=…>` avatars into `host`. A
    /// member with no sigil derivation (an unparseable DID) is skipped
    /// rather than rendered as a blank avatar.
    fn render(host: &HtmlElement, members: &[RosterMember]) {
        let mut html = format!(r#"<div class="{ROSTER_CLASS}">"#);
        for member in members {
            let Some(value) = tonk_sigil::did_sigil_value(&member.did) else {
                continue;
            };
            let title = escape_attr(member.name.as_deref().unwrap_or(&member.did));
            html.push_str(&format!(
                r#"<tonk-sigil class="{AVATAR_CLASS}" value="{value}" title="{title}"></tonk-sigil>"#
            ));
        }
        html.push_str("</div>");
        host.set_inner_html(&html);
    }

    /// Resolve the repo from the `<tonk-repository>` ancestor, fetch the
    /// roster, and repaint. A no-op with no repository ancestor.
    fn refresh(this: &HtmlElement) {
        let host = this.clone();
        let Some(repo) = repo_from_ancestor(&host) else {
            return;
        };
        spawn_local(async move {
            if let Some(members) = fetch_members(&repo).await {
                render(&host, &members);
            }
        });
    }

    /// Install the roster's window listeners: `tonk:status-refresh`
    /// (ignored unless its `detail` repo matches this element's) and
    /// `tonk:committed` (always). Both re-fetch and repaint.
    fn install_listeners(this: &HtmlElement, refresh_slot: &Listener, committed_slot: &Listener) {
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

    /// `<tonk-roster>`. Resolves its repo on connect, fetches the
    /// roster, and paints — then re-paints whenever the sync controller
    /// fires `tonk:status-refresh` / `tonk:committed`, so a member who
    /// joins on another device appears without a manual reload.
    #[derive(Default)]
    pub(crate) struct TonkRoster {
        refresh: Listener,
        committed: Listener,
    }

    impl CustomElement for TonkRoster {
        fn shadow() -> bool {
            // Light DOM: the consuming workspace view styles the cluster
            // (`.workspace__roster`) and the element must see its
            // `<tonk-repository>` ancestor via `closest`.
            false
        }

        fn observed_attributes() -> &'static [&'static str] {
            &[]
        }

        fn inject_children(&mut self, _this: &HtmlElement) {}

        fn connected_callback(&mut self, this: &HtmlElement) {
            refresh(this);
            install_listeners(this, &self.refresh, &self.committed);
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

    /// Register `<tonk-roster>`. Idempotent.
    pub(crate) fn register() {
        if already_registered() {
            return;
        }
        TonkRoster::define("tonk-roster");
    }

    fn already_registered() -> bool {
        let Some(win) = window() else {
            return false;
        };
        !win.custom_elements().get("tonk-roster").is_undefined()
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use wasm_bindgen_test::wasm_bindgen_test_configure;

        wasm_bindgen_test_configure!(run_in_browser);

        #[dialog_common::test]
        fn it_escapes_attribute_specials_in_member_names() {
            assert_eq!(escape_attr(r#"a&b<c>"d"#), "a&amp;b&lt;c&gt;&quot;d",);
        }

        #[dialog_common::test]
        async fn it_renders_one_avatar_per_member_with_a_sigil() {
            let document = window().unwrap().document().unwrap();
            let body = document.body().unwrap();
            let host = document.create_element("tonk-roster").unwrap();
            body.append_child(&host).unwrap();
            let host_el = host.dyn_ref::<HtmlElement>().unwrap();

            let members = vec![
                RosterMember {
                    did: "did:key:zAlice".to_string(),
                    name: Some("Alice".to_string()),
                    is_self: true,
                },
                RosterMember {
                    did: "did:key:zBob".to_string(),
                    name: None,
                    is_self: false,
                },
            ];
            render(host_el, &members);

            let avatars = host.query_selector_all("tonk-sigil").unwrap();
            assert_eq!(avatars.length(), 2);

            // The named member's title is the name; the unnamed one's
            // title falls back to the DID.
            let first = host
                .query_selector("tonk-sigil")
                .unwrap()
                .expect("first avatar present");
            assert_eq!(first.get_attribute("title").as_deref(), Some("Alice"));

            host.remove();
        }
    }
}

/// Register the `<tonk-roster>` custom element. No-op off-wasm.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) use element::register;
