//! Worker-backed connect-agent state rendered in the attached FABB panel.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::{Function, JSON, Object, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{Element, HtmlElement, Url, window};

use crate::logic::{agent_handoff_claim_json, agent_handoff_query_body, agent_prompt};
use crate::shadow::{self, Bound};
use crate::subscribing;

const SUB_TAG: &str = "tonk-agent-panel";

#[derive(Clone, Default)]
struct AgentState {
    status: String,
    link: String,
    pending: bool,
}

#[derive(Clone)]
struct Target {
    status: Element,
    prompt: Element,
    copy: Element,
    retry: Element,
    bar: HtmlElement,
}

#[derive(Default)]
pub struct TonkAgentPanel {
    scaffold: subscribing::Scaffold,
    state: Rc<RefCell<AgentState>>,
    listeners: Vec<Bound>,
}

impl CustomElement for TonkAgentPanel {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["space"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        let behaviour: Rc<dyn subscribing::Subscribing> = Rc::new(AgentBehaviour {
            state: self.state.clone(),
            host: this.clone(),
        });
        self.scaffold.connect(this, behaviour);
        if let Some(view) = target(this) {
            render(&view, &self.state.borrow());
        }

        let host = this.clone();
        let state = self.state.clone();
        self.listeners
            .push(shadow::bind(this, "fabb-agent-open", move |_| {
                let should_request = {
                    let mut state = state.borrow_mut();
                    if !state.link.is_empty() || !state.status.is_empty() || state.pending {
                        false
                    } else {
                        state.pending = true;
                        true
                    }
                };
                if let Some(view) = target(&host) {
                    render(&view, &state.borrow());
                }
                if should_request {
                    dispatch_handoff(false);
                }
            }));

        let host = this.clone();
        let state = self.state.clone();
        self.listeners
            .push(shadow::bind(this, "fabb-agent-copy", move |_| {
                let link = state.borrow().link.clone();
                if link.is_empty() {
                    return;
                }
                if let Some(view) = target(&host) {
                    copy_prompt(&view, &link);
                }
            }));

        let state = self.state.clone();
        let host = this.clone();
        self.listeners
            .push(shadow::bind(this, "fabb-agent-retry", move |_| {
                if needs_account(&state.borrow().status) {
                    shadow::emit(&host, "fabb-account-needed", &wasm_bindgen::JsValue::NULL);
                    return;
                }
                state.borrow_mut().pending = true;
                if let Some(view) = target(&host) {
                    render(&view, &state.borrow());
                }
                dispatch_handoff(true);
            }));
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
        self.scaffold.disconnect();
        *self.state.borrow_mut() = AgentState::default();
        if let Some(target) = target(this) {
            let behaviour: Rc<dyn subscribing::Subscribing> = Rc::new(AgentBehaviour {
                state: self.state.clone(),
                host: this.clone(),
            });
            render(&target, &self.state.borrow());
            self.scaffold.connect(this, behaviour);
        } else {
            let behaviour: Rc<dyn subscribing::Subscribing> = Rc::new(AgentBehaviour {
                state: self.state.clone(),
                host: this.clone(),
            });
            self.scaffold.connect(this, behaviour);
        }
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        self.scaffold.disconnect();
        self.listeners.clear();
        *self.state.borrow_mut() = AgentState::default();
    }
}

struct AgentBehaviour {
    state: Rc<RefCell<AgentState>>,
    host: HtmlElement,
}

impl subscribing::Subscribing for AgentBehaviour {
    fn query_body(&self, this: &HtmlElement) -> Result<String, String> {
        agent_handoff_query_body(&this.get_attribute("space").unwrap_or_default())
    }

    fn render_reset(&self, _host: &HtmlElement, payload: &wasm_bindgen::JsValue) {
        let rows = js_sys::Array::from(payload);
        if let Some((status, link)) = read_row(&rows.get(rows.length().saturating_sub(1))) {
            apply(&self.state, &self.host, status, link);
        }
    }

    fn render_update(&self, _host: &HtmlElement, payload: &wasm_bindgen::JsValue) {
        let asserted = Reflect::get(payload, &"asserted".into()).unwrap_or_default();
        let rows = js_sys::Array::from(&asserted);
        if let Some((status, link)) = read_row(&rows.get(rows.length().saturating_sub(1))) {
            apply(&self.state, &self.host, status, link);
        }
    }

    fn tag(&self) -> &'static str {
        SUB_TAG
    }
}

fn target(this: &HtmlElement) -> Option<Target> {
    let bar: HtmlElement = this.closest("tonk-fab").ok().flatten()?.dyn_into().ok()?;
    let root = bar.shadow_root()?;
    Some(Target {
        status: root.query_selector(".agent-status").ok().flatten()?,
        prompt: root
            .query_selector("#agent-panel .panel-copytext")
            .ok()
            .flatten()?,
        copy: root
            .query_selector("#agent-panel .panel-copy")
            .ok()
            .flatten()?,
        retry: root
            .query_selector("#agent-panel .agent-retry")
            .ok()
            .flatten()?,
        bar,
    })
}

fn read_row(row: &wasm_bindgen::JsValue) -> Option<(String, String)> {
    let fields = Reflect::get(row, &"fields".into()).ok()?;
    let status = Reflect::get(&fields, &"status".into()).ok()?.as_string()?;
    let link = Reflect::get(&fields, &"link".into())
        .ok()
        .and_then(|value| value.as_string())
        .unwrap_or_default();
    Some((status, link))
}

fn apply(state: &Rc<RefCell<AgentState>>, host: &HtmlElement, status: String, link: String) {
    *state.borrow_mut() = AgentState {
        status,
        link,
        pending: false,
    };
    if let Some(target) = target(host) {
        render(&target, &state.borrow());
    }
}

fn render(target: &Target, state: &AgentState) {
    let ready = !state.link.is_empty();
    let _ = target.copy.toggle_attribute_with_force("hidden", !ready);
    let retryable = !ready && !state.pending && !state.status.is_empty();
    let _ = target
        .retry
        .toggle_attribute_with_force("hidden", !retryable);
    if retryable {
        target
            .retry
            .set_text_content(Some(if needs_account(&state.status) {
                "add an account"
            } else if state.status.contains("sync") {
                "turn on sync"
            } else {
                "try again"
            }));
    }
    let message = if ready {
        "copy the complete prompt and keep its bearer link private"
    } else if state.pending {
        "creating an agent invitation…"
    } else if state.status.is_empty() {
        "create an agent invitation when you open this panel"
    } else {
        &state.status
    };
    target.status.set_text_content(Some(message));
    if ready {
        let name = target
            .bar
            .get_attribute("label")
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "this".into());
        target
            .prompt
            .set_text_content(Some(&localized_prompt(&name, &state.link)));
        let _ = target.prompt.remove_attribute("hidden");
    } else {
        target.prompt.set_text_content(None);
        let _ = target.prompt.set_attribute("hidden", "");
    }
}

fn needs_account(status: &str) -> bool {
    status.contains("account") || status.contains("sign in") || status.contains("email")
}

fn localized_prompt(name: &str, link: &str) -> String {
    let prompt = agent_prompt(name, link);
    let Ok(url) = Url::new(link) else {
        return prompt;
    };
    if !matches!(url.hostname().as_str(), "localhost" | "127.0.0.1" | "[::1]") {
        return prompt;
    }
    let local = prompt.replace("npx --yes @tonk/cli", "tonk");
    local.replacen(
        "tonk join '",
        &format!(
            "TONK_CONNECTION_ORIGIN={} tonk join '",
            serde_json::to_string(&url.origin()).unwrap_or_else(|_| "\"\"".into())
        ),
        1,
    )
}

fn dispatch_handoff(fresh: bool) {
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
    let claim = agent_handoff_claim_json(js_sys::Date::now(), fresh);
    if let Ok(value) = JSON::parse(&claim.to_string()) {
        let _ = transact.call1(&tonk, &value);
    }
}

fn copy_prompt(target: &Target, link: &str) {
    let name = target
        .bar
        .get_attribute("label")
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "this".into());
    let prompt = localized_prompt(&name, link);
    let clipboard = window().map(|window| window.navigator().clipboard());
    let view = target.clone();
    match clipboard {
        Some(clipboard) => {
            let promise = clipboard.write_text(&prompt);
            spawn_local(async move {
                if JsFuture::from(promise).await.is_ok() {
                    view.status.set_text_content(Some("agent prompt copied"));
                } else {
                    view.status
                        .set_text_content(Some("could not copy the prompt; try again"));
                }
            });
        }
        None => target
            .status
            .set_text_content(Some("clipboard unavailable; select and copy the prompt")),
    }
}

pub fn register() {
    if subscribing::already_registered(SUB_TAG) {
        return;
    }
    TonkAgentPanel::define(SUB_TAG);
    subscribing::install_frame_shims(SUB_TAG);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn localhost_prompts_use_the_linked_cli_and_connection_origin() {
        let prompt = localized_prompt("Local", "http://localhost:8080/#tonk-agent-v2=secret");
        assert!(prompt.contains("TONK_CONNECTION_ORIGIN=\"http://localhost:8080\" tonk join"));
        assert!(!prompt.contains("npx --yes"));
    }
}
