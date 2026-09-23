//! Worker-backed connect-agent state rendered in the attached FABB panel.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::{Function, JSON, Object, Promise, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
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
    attempt: u64,
    uncertain: bool,
}

impl AgentState {
    fn begin_retry(&mut self) -> bool {
        self.pending = true;
        self.attempt += 1;
        let fresh = !self.uncertain;
        self.uncertain = false;
        fresh
    }
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
                    if !state.link.is_empty()
                        || state.pending
                        || (!state.status.is_empty() && !needs_new_invite(&state.status))
                    {
                        false
                    } else {
                        state.pending = true;
                        state.attempt += 1;
                        state.status.clear();
                        true
                    }
                };
                if let Some(view) = target(&host) {
                    render(&view, &state.borrow());
                }
                if should_request {
                    // Opening this drawer is an explicit request. Replacing a
                    // lost prior bearer needs a new grant, without an extra
                    // recovery click.
                    dispatch_handoff(&host, &state, true);
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
                let fresh = {
                    let mut state = state.borrow_mut();
                    state.begin_retry()
                };
                if let Some(view) = target(&host) {
                    render(&view, &state.borrow());
                }
                dispatch_handoff(&host, &state, fresh);
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
    let attempt = state.borrow().attempt;
    *state.borrow_mut() = AgentState {
        status,
        link,
        pending: false,
        attempt,
        uncertain: false,
    };
    if let Some(target) = target(host) {
        render(&target, &state.borrow());
    }
}

fn render(target: &Target, state: &AgentState) {
    let ready = !state.link.is_empty();
    let _ = target.copy.toggle_attribute_with_force("hidden", !ready);
    clear_copy_feedback(&target.copy);
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
        "copy the prompt and only share the link with the agent"
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

fn needs_new_invite(status: &str) -> bool {
    status.starts_with("an invite was already issued for this space")
        || status.starts_with("invite link is no longer in this session")
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

fn dispatch_handoff(host: &HtmlElement, state: &Rc<RefCell<AgentState>>, fresh: bool) {
    let attempt = state.borrow().attempt;
    let fail = |message: &str| {
        let mut current = state.borrow_mut();
        if current.attempt != attempt || !current.pending {
            return;
        }
        current.pending = false;
        current.status = message.into();
        current.uncertain = false;
        drop(current);
        if let Some(view) = target(host) {
            render(&view, &state.borrow());
        }
    };
    let Some(win) = window() else {
        fail("Agent invitation is unavailable. Try again.");
        return;
    };
    let Some(tonk) = Reflect::get(&win, &"tonk".into())
        .ok()
        .and_then(|value| value.dyn_into::<Object>().ok())
    else {
        fail("Agent invitation is unavailable. Try again.");
        return;
    };
    let Some(transact) = Reflect::get(&tonk, &"transact".into())
        .ok()
        .and_then(|value| value.dyn_into::<Function>().ok())
    else {
        fail("Agent invitation is unavailable. Try again.");
        return;
    };
    // App chrome sends the command from the profile branch. The worker uses
    // the explicit space to target the handoff, while the response remains
    // subscribed on that space's content branch.
    let Some(space) = host
        .get_attribute("space")
        .filter(|space| !space.is_empty())
    else {
        fail("Agent invitation is unavailable. Try again.");
        return;
    };
    let claim = agent_handoff_claim_json(&space, js_sys::Date::now(), fresh);
    if let Ok(value) = JSON::parse(&claim.to_string()) {
        match transact.call1(&tonk, &value) {
            Ok(result) => {
                if let Ok(promise) = result.dyn_into::<Promise>() {
                    let host = host.clone();
                    let state = state.clone();
                    spawn_local(async move {
                        if JsFuture::from(promise).await.is_err() {
                            fail_handoff(
                                &host,
                                &state,
                                attempt,
                                "Agent invitation failed. Try again.",
                                false,
                            );
                        }
                    });
                }
            }
            Err(_) => fail("Agent invitation failed. Try again."),
        }
    } else {
        fail("Agent invitation is unavailable. Try again.");
    }
    if !state.borrow().pending {
        return;
    }
    // Command providers report their result through the subscription. If
    // delivery fails, leave a recoverable action instead of a permanent spinner.
    let host = host.clone();
    let state = state.clone();
    let timeout = Closure::<dyn FnMut()>::once(move || {
        fail_handoff(
            &host,
            &state,
            attempt,
            "Agent invitation is taking too long. Try again.",
            true,
        );
    });
    let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(
        timeout.as_ref().unchecked_ref(),
        30_000,
    );
    timeout.forget();
}

fn fail_handoff(
    host: &HtmlElement,
    state: &Rc<RefCell<AgentState>>,
    attempt: u64,
    message: &str,
    uncertain: bool,
) {
    let mut current = state.borrow_mut();
    if current.attempt != attempt || !current.pending || !host.is_connected() {
        return;
    }
    current.pending = false;
    current.status = message.into();
    current.uncertain = uncertain;
    drop(current);
    if let Some(view) = target(host) {
        render(&view, &state.borrow());
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
                    show_copy_feedback(&view.copy, 1_800);
                } else {
                    clear_copy_feedback(&view.copy);
                    view.status
                        .set_text_content(Some("could not copy the prompt; try again"));
                }
            });
        }
        None => {
            clear_copy_feedback(&target.copy);
            target
                .status
                .set_text_content(Some("clipboard unavailable; select and copy the prompt"));
        }
    }
}

fn clear_copy_feedback(button: &Element) {
    button.set_text_content(Some("copy prompt"));
}

fn show_copy_feedback(button: &Element, duration_ms: i32) {
    let sequence = button
        .get_attribute("data-copy-feedback")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0)
        .wrapping_add(1)
        .to_string();
    let _ = button.set_attribute("data-copy-feedback", &sequence);
    button.set_text_content(Some("copied"));
    let Some(win) = window() else {
        clear_copy_feedback(button);
        return;
    };
    let pending_button = button.clone();
    let timeout = Closure::<dyn FnMut()>::once(move || {
        if pending_button
            .get_attribute("data-copy-feedback")
            .as_deref()
            == Some(sequence.as_str())
        {
            clear_copy_feedback(&pending_button);
        }
    });
    if win
        .set_timeout_with_callback_and_timeout_and_arguments_0(
            timeout.as_ref().unchecked_ref(),
            duration_ms,
        )
        .is_ok()
    {
        timeout.forget();
    } else {
        clear_copy_feedback(&button);
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

    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn copied_button_feedback_resets_after_the_latest_copy() {
        let document = window().unwrap().document().unwrap();
        let button = document.create_element("button").unwrap();
        document.body().unwrap().append_child(&button).unwrap();
        show_copy_feedback(&button, 25);
        assert_eq!(button.text_content().as_deref(), Some("copied"));
        show_copy_feedback(&button, 200);
        let wait = |ms| {
            Promise::new(&mut |resolve, _| {
                window()
                    .unwrap()
                    .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
                    .unwrap();
            })
        };
        JsFuture::from(wait(60)).await.unwrap();
        assert_eq!(button.text_content().as_deref(), Some("copied"));
        JsFuture::from(wait(200)).await.unwrap();
        assert_eq!(button.text_content().as_deref(), Some("copy prompt"));
        button.remove();
    }

    #[wasm_bindgen_test::wasm_bindgen_test]
    fn uncertain_retry_reuses_the_invitation_request() {
        let mut state = AgentState {
            uncertain: true,
            ..AgentState::default()
        };
        assert!(!state.begin_retry(), "timeout must not mint a fresh bearer");
        assert!(state.pending);
        assert_eq!(state.attempt, 1);
    }

    #[test]
    fn localhost_prompts_use_the_linked_cli_and_connection_origin() {
        let prompt = localized_prompt("Local", "http://localhost:8080/#tonk-agent-v2=secret");
        assert!(prompt.contains("TONK_CONNECTION_ORIGIN=\"http://localhost:8080\" tonk join"));
        assert!(!prompt.contains("npx --yes"));
    }
}
