//! App-owned `connect a tool` surface.
//!
//! The worker still owns scoped invitation issuance and refusal semantics. This
//! element only dispatches the existing handoff command, reads its session
//! response, and lets the person copy either the raw link or a prompt carrying
//! that same link. No second copy action mints another identity.

use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::{Array, Function, JSON, Object, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{Element, HtmlDialogElement, HtmlElement, window};

use crate::logic::{
    tool_connection_claim_json, tool_connection_prompt, tool_connection_state_query_body,
};
use crate::subscribing;

const TAG: &str = "tonk-tool-connection";
const CLUSTER_ID: &str = "fabb-tool-connection-cluster";
const READY_STATUS: &str = "link ready. anyone with it can read and change this space until it expires or you revoke it. keep it private.";
const COPY_LINK_LABEL: &str = "copy link";
const COPY_PROMPT_LABEL: &str = "copy agent prompt";

#[derive(Default)]
pub(crate) struct TonkToolConnection {
    scaffold: subscribing::Scaffold,
}

struct StateBehaviour;

struct ConnectionState {
    status: String,
    link: String,
    mode: Option<String>,
}

impl subscribing::Subscribing for StateBehaviour {
    fn query_body(&self, this: &HtmlElement) -> Result<String, String> {
        tool_connection_state_query_body(&this.get_attribute("space").unwrap_or_default())
    }

    fn render_reset(&self, host: &HtmlElement, payload: &JsValue) {
        let rows = Array::from(payload);
        if let Some(state) = read_state(&rows.get(rows.length().saturating_sub(1))) {
            apply_state(host, state);
        }
    }

    fn render_update(&self, host: &HtmlElement, payload: &JsValue) {
        let asserted =
            Reflect::get(payload, &JsValue::from_str("asserted")).unwrap_or(JsValue::UNDEFINED);
        let rows = Array::from(&asserted);
        if let Some(state) = read_state(&rows.get(rows.length().saturating_sub(1))) {
            apply_state(host, state);
        }
    }

    fn tag(&self) -> &'static str {
        TAG
    }
}

impl CustomElement for TonkToolConnection {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["space"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        self.connect(this);
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        name: String,
        old: Option<String>,
        new: Option<String>,
    ) {
        if name != "space" || old == new {
            return;
        }
        if cluster().is_some_and(|cluster| {
            cluster.get_attribute("data-tool-space") == old && !cluster.has_attribute("hidden")
        }) {
            clear_surface(true);
        }
        self.scaffold.disconnect();
        self.connect(this);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        self.scaffold.disconnect();
    }
}

impl TonkToolConnection {
    fn connect(&self, this: &HtmlElement) {
        let state: Rc<dyn subscribing::Subscribing> = Rc::new(StateBehaviour);
        self.scaffold.connect(this, state);
    }
}

fn read_state(row: &JsValue) -> Option<ConnectionState> {
    if row.is_null() || row.is_undefined() {
        return None;
    }
    let fields = Reflect::get(row, &JsValue::from_str("fields")).ok()?;
    let status = Reflect::get(&fields, &JsValue::from_str("status"))
        .ok()?
        .as_string()?;
    let link = Reflect::get(&fields, &JsValue::from_str("link"))
        .ok()?
        .as_string()?;
    let mode = Reflect::get(&fields, &JsValue::from_str("mode"))
        .ok()
        .and_then(|value| value.as_string())
        .filter(|value| !value.is_empty());
    Some(ConnectionState { status, link, mode })
}

fn apply_state(host: &HtmlElement, state: ConnectionState) {
    let Some(space) = host.get_attribute("space") else {
        return;
    };
    let Some(cluster) = cluster() else { return };
    if cluster.get_attribute("data-tool-space").as_deref() != Some(space.as_str()) {
        return;
    }
    let ready = state.status == "ready"
        && state.mode.as_deref() == Some("scoped")
        && !state.link.is_empty();
    if !ready || cluster.get_attribute("data-tool-link").as_deref() != Some(state.link.as_str()) {
        reset_copy_labels(&cluster);
    }
    if ready {
        let _ = cluster.set_attribute("data-tool-link", &state.link);
        set_status(&cluster, READY_STATUS);
    } else {
        let _ = cluster.remove_attribute("data-tool-link");
        set_status(&cluster, &state.status);
    }
    if let Some(mode) = state.mode {
        let _ = cluster.set_attribute("data-tool-mode", &mode);
    } else {
        let _ = cluster.remove_attribute("data-tool-mode");
    }
    set_enabled(&cluster, "[data-tool-copy-link]", ready);
    set_enabled(&cluster, "[data-tool-copy-prompt]", ready);
    let retry = matches!(
        cluster.get_attribute("data-tool-mode").as_deref(),
        Some("account" | "activation" | "sync" | "retry" | "new")
    );
    for selector in ["[data-tool-copy-link]", "[data-tool-copy-prompt]"] {
        set_visible(&cluster, selector, !retry);
    }
    if let Ok(Some(button)) = cluster.query_selector("[data-tool-retry]") {
        if retry {
            let _ = button.remove_attribute("hidden");
            button.set_text_content(Some(
                match cluster.get_attribute("data-tool-mode").as_deref() {
                    Some("account") => "create an account or sign in",
                    Some("activation") => "verify account",
                    Some("sync") => "turn on sync",
                    Some("new") => "create a new link",
                    _ => "try again",
                },
            ));
        } else {
            let _ = button.set_attribute("hidden", "");
        }
    }
}

fn set_enabled(cluster: &Element, selector: &str, enabled: bool) {
    if let Ok(Some(button)) = cluster.query_selector(selector) {
        if enabled {
            let _ = button.remove_attribute("disabled");
        } else {
            let _ = button.set_attribute("disabled", "");
        }
    }
}

fn set_visible(cluster: &Element, selector: &str, visible: bool) {
    if let Ok(Some(button)) = cluster.query_selector(selector) {
        if visible {
            let _ = button.remove_attribute("hidden");
        } else {
            let _ = button.set_attribute("hidden", "");
        }
    }
}

fn set_status(cluster: &Element, status: &str) {
    if let Ok(Some(element)) = cluster.query_selector("[data-tool-connection-status]") {
        element.set_text_content(Some(status));
    }
}

fn set_copy_label(cluster: &Element, selector: &str, label: &str) {
    if let Ok(Some(button)) = cluster.query_selector(selector) {
        button.set_text_content(Some(label));
    }
}

fn reset_copy_labels(cluster: &Element) {
    set_copy_label(cluster, "[data-tool-copy-link]", COPY_LINK_LABEL);
    set_copy_label(cluster, "[data-tool-copy-prompt]", COPY_PROMPT_LABEL);
}

fn begin_copy(cluster: &Element, selector: &str) -> u64 {
    let sequence = cluster
        .get_attribute("data-tool-copy-sequence")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0)
        .wrapping_add(1);
    let _ = cluster.set_attribute("data-tool-copy-sequence", &sequence.to_string());
    reset_copy_labels(cluster);
    set_copy_label(cluster, selector, "copying…");
    set_status(cluster, READY_STATUS);
    sequence
}

fn finish_copy(cluster: &Element, selector: &str, link: &str, sequence: u64, copied: bool) {
    let current_sequence = sequence.to_string();
    if cluster.get_attribute("data-tool-link").as_deref() != Some(link)
        || cluster.get_attribute("data-tool-copy-sequence").as_deref()
            != Some(current_sequence.as_str())
        || !cluster
            .query_selector(selector)
            .ok()
            .flatten()
            .is_some_and(|button| button.text_content().as_deref() == Some("copying…"))
    {
        return;
    }
    if copied {
        set_copy_label(cluster, selector, "copied");
        set_status(cluster, READY_STATUS);
    } else {
        let label = if selector == "[data-tool-copy-prompt]" {
            COPY_PROMPT_LABEL
        } else {
            COPY_LINK_LABEL
        };
        set_copy_label(cluster, selector, label);
        set_status(
            cluster,
            "couldn’t copy. keep the link private and try again.",
        );
    }
}

fn cluster() -> Option<Element> {
    window()?.document()?.get_element_by_id(CLUSTER_ID)
}

fn native_dialog(cluster: &Element) -> Option<HtmlDialogElement> {
    cluster
        .shadow_root()?
        .query_selector("dialog")
        .ok()
        .flatten()?
        .dyn_into()
        .ok()
}

fn ensure_cluster() -> Option<Element> {
    if let Some(cluster) = cluster() {
        return Some(cluster);
    }
    let document = window()?.document()?;
    let holder = document.create_element("div").ok()?;
    holder.set_inner_html(crate::markup::REFUSAL_DIALOGS_HTML);
    let cluster = holder.query_selector(&format!("#{CLUSTER_ID}")).ok()??;
    document.body()?.append_child(&cluster).ok()?;
    Some(cluster)
}

fn clear_surface(hide: bool) {
    let Some(cluster) = cluster() else { return };
    let _ = cluster.remove_attribute("data-tool-link");
    let _ = cluster.remove_attribute("data-tool-mode");
    let _ = cluster.set_attribute("data-tool-space", "");
    set_enabled(&cluster, "[data-tool-copy-link]", false);
    set_enabled(&cluster, "[data-tool-copy-prompt]", false);
    reset_copy_labels(&cluster);
    set_visible(&cluster, "[data-tool-copy-link]", true);
    set_visible(&cluster, "[data-tool-copy-prompt]", true);
    if let Ok(Some(retry)) = cluster.query_selector("[data-tool-retry]") {
        let _ = retry.set_attribute("hidden", "");
    }
    set_status(&cluster, "creating a private link…");
    if hide {
        if let Some(dialog) = native_dialog(&cluster) {
            dialog.close();
        }
        let _ = cluster.set_attribute("hidden", "");
    }
}

fn dispatch(space: &str) {
    let claim = tool_connection_claim_json(space, js_sys::Date::now());
    let Ok(json) = serde_json::to_string(&claim) else {
        return;
    };
    let Some(win) = window() else { return };
    let Some(tonk) = Reflect::get(&win, &"tonk".into())
        .ok()
        .and_then(|value| value.dyn_into::<Object>().ok())
    else {
        return;
    };
    let Some(transact) = Reflect::get(&tonk, &"transact".into())
        .ok()
        .and_then(|value| value.dyn_into::<Function>().ok())
    else {
        return;
    };
    if let Ok(claim) = JSON::parse(&json) {
        let _ = transact.call1(&tonk, &claim);
    }
}

/// Open the app-owned surface and request a fresh link for this exact space.
pub(crate) fn open(bar: &HtmlElement) {
    let Some(space) = bar.get_attribute("space").filter(|space| !space.is_empty()) else {
        return;
    };
    let Some(cluster) = ensure_cluster() else {
        return;
    };
    clear_surface(false);
    let _ = cluster.set_attribute("data-tool-space", &space);
    let _ = cluster.remove_attribute("hidden");
    if let Ok(host) = cluster.dyn_into::<HtmlElement>() {
        crate::dialog::show_dialog(&host);
    }
    dispatch(&space);
}

fn copy(kind: &str) {
    let Some(cluster) = cluster() else { return };
    let Some(link) = cluster.get_attribute("data-tool-link") else {
        return;
    };
    let is_prompt = kind == "prompt";
    let selector = if is_prompt {
        "[data-tool-copy-prompt]"
    } else {
        "[data-tool-copy-link]"
    };
    let text = if is_prompt {
        tool_connection_prompt(
            &link,
            &tonk_host::bridge::context_origin().unwrap_or_default(),
        )
    } else {
        link.clone()
    };
    let sequence = begin_copy(&cluster, selector);
    let Some(clipboard) = window().map(|window| window.navigator().clipboard()) else {
        reset_copy_labels(&cluster);
        set_status(
            &cluster,
            "clipboard unavailable. keep the link private and try again.",
        );
        return;
    };
    spawn_local(async move {
        let copied = JsFuture::from(clipboard.write_text(&text)).await.is_ok();
        finish_copy(&cluster, selector, &link, sequence, copied);
    });
}

fn install_surface_listeners() {
    let Some(document) = window().and_then(|window| window.document()) else {
        return;
    };
    let click = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        let Some(target) = event
            .target()
            .and_then(|target| target.dyn_into::<Element>().ok())
        else {
            return;
        };
        if target
            .closest("[data-tool-copy-link]")
            .ok()
            .flatten()
            .is_some()
        {
            copy("link");
        } else if target
            .closest("[data-tool-copy-prompt]")
            .ok()
            .flatten()
            .is_some()
        {
            copy("prompt");
        } else if target.closest("[data-tool-retry]").ok().flatten().is_some() {
            let Some(surface) = cluster() else { return };
            let Some(space) = surface
                .get_attribute("data-tool-space")
                .filter(|space| !space.is_empty())
            else {
                return;
            };
            match surface.get_attribute("data-tool-mode").as_deref() {
                Some("account") | Some("activation") => {
                    let reason = if surface.get_attribute("data-tool-mode").as_deref()
                        == Some("activation")
                    {
                        "agent-invite-activation"
                    } else {
                        "agent-invite-account"
                    };
                    let _ = surface.remove_attribute("data-tool-link");
                    tonk_host::request_registration(
                        &serde_json::json!({ "reason": reason }).to_string(),
                    );
                }
                _ => {
                    clear_surface(false);
                    let Some(cluster) = cluster() else { return };
                    let _ = cluster.set_attribute("data-tool-space", &space);
                    dispatch(&space);
                }
            }
        }
    });
    let _ = document.add_event_listener_with_callback("fabb-press", click.as_ref().unchecked_ref());
    click.forget();

    let close = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        if event
            .target()
            .and_then(|target| target.dyn_into::<Element>().ok())
            .is_some_and(|target| target.id() == CLUSTER_ID)
        {
            clear_surface(true);
        }
    });
    let _ = document.add_event_listener_with_callback("fabb-close", close.as_ref().unchecked_ref());
    close.forget();

    let registration = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        let Some(surface) = cluster().filter(|cluster| !cluster.has_attribute("hidden")) else {
            return;
        };
        let Some(space) = surface
            .get_attribute("data-tool-space")
            .filter(|space| !space.is_empty())
        else {
            return;
        };
        clear_surface(false);
        let Some(cluster) = cluster() else { return };
        let _ = cluster.set_attribute("data-tool-space", &space);
        dispatch(&space);
    });
    let _ = window().unwrap().add_event_listener_with_callback(
        "tonk:registration-closed",
        registration.as_ref().unchecked_ref(),
    );
    registration.forget();
}

pub(crate) fn register() {
    if subscribing::already_registered(TAG) {
        return;
    }
    TonkToolConnection::define(TAG);
    subscribing::install_frame_shims(TAG);
    install_surface_listeners();
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    fn tool_connection_uses_the_shared_dialog_and_clears_on_close() {
        crate::register();
        let document = window().unwrap().document().unwrap();
        let bar: HtmlElement = document.create_element("div").unwrap().dyn_into().unwrap();
        bar.set_attribute("space", "did:key:zToolDialog").unwrap();

        open(&bar);
        let surface = cluster().unwrap();
        let native = native_dialog(&surface).unwrap();
        assert!(native.open());
        assert!(!surface.has_attribute("hidden"));
        assert_eq!(
            surface.get_attribute("heading").as_deref(),
            Some("connect a tool")
        );
        assert!(
            surface
                .query_selector("[data-tool-copy-link][slot=actions]")
                .unwrap()
                .is_some()
        );
        let root = surface.shadow_root().unwrap();
        let heading = root.query_selector(".t").unwrap().unwrap();
        let body = root.query_selector(".body").unwrap().unwrap();
        let actions = root.query_selector(".frow").unwrap().unwrap();
        let description = surface.query_selector("p").unwrap().unwrap();
        let status = surface
            .query_selector("[data-tool-connection-status]")
            .unwrap()
            .unwrap();
        let heading_rect = heading.get_bounding_client_rect();
        let body_rect = body.get_bounding_client_rect();
        let actions_rect = actions.get_bounding_client_rect();
        let description_rect = description.get_bounding_client_rect();
        let status_rect = status.get_bounding_client_rect();
        assert!(body_rect.top() > heading_rect.bottom());
        assert!(description_rect.top() >= body_rect.top());
        assert!(description_rect.bottom() <= status_rect.top());
        assert!(status_rect.bottom() <= body_rect.bottom());
        assert!(actions_rect.top() >= body_rect.bottom());
        assert!(
            !surface
                .text_content()
                .unwrap_or_default()
                .contains("back to share")
        );

        let connection: HtmlElement = document.create_element(TAG).unwrap().dyn_into().unwrap();
        connection
            .set_attribute("space", "did:key:zToolDialog")
            .unwrap();
        let link = "https://tonk.test/join#tonk-agent-v2=first";
        apply_state(
            &connection,
            ConnectionState {
                status: "ready".into(),
                link: link.into(),
                mode: Some("scoped".into()),
            },
        );
        let first = begin_copy(&surface, "[data-tool-copy-link]");
        finish_copy(&surface, "[data-tool-copy-link]", link, first, true);
        assert_eq!(
            surface
                .query_selector("[data-tool-copy-link]")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("copied")
        );
        assert_eq!(
            surface
                .query_selector("[data-tool-copy-prompt]")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some(COPY_PROMPT_LABEL)
        );
        assert_eq!(status.text_content().as_deref(), Some(READY_STATUS));

        let failed = begin_copy(&surface, "[data-tool-copy-prompt]");
        finish_copy(&surface, "[data-tool-copy-prompt]", link, failed, false);
        assert!(
            status
                .text_content()
                .unwrap_or_default()
                .contains("couldn’t copy")
        );
        assert_eq!(
            surface
                .query_selector("[data-tool-copy-prompt]")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some(COPY_PROMPT_LABEL)
        );
        let prompt = begin_copy(&surface, "[data-tool-copy-prompt]");
        assert_eq!(status.text_content().as_deref(), Some(READY_STATUS));
        finish_copy(&surface, "[data-tool-copy-prompt]", link, prompt, true);
        assert_eq!(
            surface
                .query_selector("[data-tool-copy-prompt]")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("copied")
        );

        let stale = begin_copy(&surface, "[data-tool-copy-link]");
        let current = begin_copy(&surface, "[data-tool-copy-link]");
        finish_copy(&surface, "[data-tool-copy-link]", link, stale, true);
        assert_eq!(
            surface
                .query_selector("[data-tool-copy-link]")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("copying…")
        );
        apply_state(
            &connection,
            ConnectionState {
                status: "ready".into(),
                link: "https://tonk.test/join#tonk-agent-v2=second".into(),
                mode: Some("scoped".into()),
            },
        );
        finish_copy(&surface, "[data-tool-copy-link]", link, current, true);
        assert_eq!(
            surface
                .query_selector("[data-tool-copy-link]")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some(COPY_LINK_LABEL)
        );

        apply_state(
            &connection,
            ConnectionState {
                status: "sign in to connect a tool".into(),
                link: String::new(),
                mode: Some("account".into()),
            },
        );
        assert!(
            surface
                .query_selector("[data-tool-retry]:not([hidden])")
                .unwrap()
                .is_some()
        );
        assert!(
            surface
                .query_selector("[data-tool-copy-link][hidden]")
                .unwrap()
                .is_some()
        );
        assert!(
            surface
                .query_selector("[data-tool-copy-prompt][hidden]")
                .unwrap()
                .is_some()
        );

        crate::dialog::close_dialog(&surface.dyn_into::<HtmlElement>().unwrap());
        assert!(!native.open());
        let surface = cluster().unwrap();
        assert!(surface.has_attribute("hidden"));
        assert!(!surface.has_attribute("data-tool-link"));

        document
            .get_element_by_id("fabb-connect-cluster")
            .unwrap()
            .remove();
        surface.remove();
    }
}
