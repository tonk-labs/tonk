//! Guest-side lifecycle for trusted account tasks that replace this FABB.

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::JsCast;
use web_sys::{CustomEvent, HtmlElement, window};

use crate::bar;
use crate::shadow::{self, Bound};
use tonk_portal::task::{
    AccountContext, Action, Anchor, Dismissal, Horizontal, Presentation, Purpose, Request, VERSION,
    Vertical,
};

#[derive(Clone, Debug)]
struct Active {
    request_id: String,
    resume: Resume,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Resume {
    #[default]
    None,
    Share,
    Agent,
}

#[derive(Default)]
pub(crate) struct State {
    next_id: u64,
    active: Option<Active>,
}

pub(crate) type Shared = Rc<RefCell<State>>;

pub(crate) fn install(this: &HtmlElement, state: &Shared, bar_state: &bar::Shared) -> Vec<Bound> {
    let mut listeners = Vec::new();
    let Some(win) = window() else {
        return listeners;
    };

    {
        let host = this.clone();
        let shared = state.clone();
        let bar = bar_state.clone();
        listeners.push(shadow::bind(&win, "tonk:task-closed", move |event| {
            let Some(event) = event.dyn_ref::<CustomEvent>() else {
                return;
            };
            let result = js_sys::Reflect::get(&event.detail(), &"result".into())
                .ok()
                .and_then(|value| value.as_string())
                .unwrap_or_else(|| "disconnected".into());
            finish(&host, &shared, &bar, &result);
        }));
    }

    for event_name in ["resize", "orientationchange", "pointerup"] {
        let host = this.clone();
        let shared = state.clone();
        listeners.push(shadow::bind(&win, event_name, move |_| {
            reseat(&host, &shared);
        }));
    }
    if let Some(viewport) = win.visual_viewport() {
        for event_name in ["resize", "scroll"] {
            let host = this.clone();
            let shared = state.clone();
            listeners.push(shadow::bind(&viewport, event_name, move |_| {
                reseat(&host, &shared);
            }));
        }
    }
    listeners
}

pub(crate) fn open_account(
    this: &HtmlElement,
    state: &Shared,
    reason: &str,
    resume: Resume,
) -> bool {
    let mut cell = state.borrow_mut();
    if cell.active.is_some() {
        return false;
    }
    cell.next_id = cell.next_id.wrapping_add(1).max(1);
    let request_id = format!("account-{}-{}", js_sys::Date::now() as u64, cell.next_id);
    let active = Active {
        request_id: request_id.clone(),
        resume,
    };
    cell.active = Some(active.clone());
    drop(cell);

    let request = request(this, &active, Action::Open, Some(reason));
    let Ok(payload) = request.to_json() else {
        state.borrow_mut().active = None;
        return false;
    };
    let _ = this.set_attribute("data-task-hosted", "");
    let _ = this.set_attribute("aria-busy", "true");
    tonk_host::request_contained_task(&payload);
    true
}

pub(crate) fn disconnect(this: &HtmlElement, state: &Shared) {
    let Some(active) = state.borrow_mut().active.take() else {
        return;
    };
    if let Ok(payload) = request(this, &active, Action::Dismiss, None).to_json() {
        tonk_host::request_contained_task(&payload);
    }
    restore_surface(this);
}

fn reseat(this: &HtmlElement, state: &Shared) {
    let Some(active) = state.borrow().active.clone() else {
        return;
    };
    if let Ok(payload) = request(this, &active, Action::Reseat, None).to_json() {
        tonk_host::request_contained_task(&payload);
    }
}

fn finish(this: &HtmlElement, state: &Shared, bar_state: &bar::Shared, result: &str) {
    let Some(active) = state.borrow_mut().active.take() else {
        return;
    };
    if result == "completed" {
        crate::element::apply_account_ready(this, true);
    }
    restore_surface(this);
    match active.resume {
        Resume::None => {}
        // The account gate was the share drawer's only purpose. Once the
        // task returns, the share action copies in place on the next click.
        Resume::Share => bar::show_actions(this, bar_state),
        Resume::Agent => {
            restore_panel(this, bar_state, "agent", "#agent-panel");
            if result == "completed"
                && let Ok(Some(agent)) = this.query_selector("tonk-agent-panel")
                && let Ok(agent) = agent.dyn_into::<HtmlElement>()
            {
                shadow::emit(&agent, "fabb-agent-open", &wasm_bindgen::JsValue::NULL);
            }
        }
    }
}

fn restore_panel(this: &HtmlElement, bar_state: &bar::Shared, name: &str, selector: &str) {
    let hidden = this
        .shadow_root()
        .and_then(|root| root.query_selector(selector).ok().flatten())
        .is_none_or(|panel| panel.has_attribute("hidden"));
    if hidden {
        bar::open(this, bar_state, name);
    }
}

fn restore_surface(this: &HtmlElement) {
    let _ = this.remove_attribute("data-task-hosted");
    let _ = this.remove_attribute("aria-busy");
}

fn request(this: &HtmlElement, active: &Active, action: Action, reason: Option<&str>) -> Request {
    let rect = this.get_bounding_client_rect();
    Request {
        version: VERSION,
        request_id: active.request_id.clone(),
        purpose: Purpose::Account,
        action,
        account: reason.map(|reason| AccountContext {
            reason: reason.to_owned(),
            space: this.get_attribute("space").unwrap_or_default(),
        }),
        presentation: matches!(action, Action::Open | Action::Reseat).then_some(Presentation {
            anchor: Anchor {
                left: rect.left(),
                top: rect.top(),
                right: rect.right(),
                bottom: rect.bottom(),
                width: rect.width(),
                height: rect.height(),
            },
            horizontal: if this.has_attribute("flip") {
                Horizontal::Right
            } else {
                Horizontal::Left
            },
            vertical: if this.has_attribute("up") {
                Vertical::Bottom
            } else {
                Vertical::Top
            },
            dismissal: Dismissal::Optional,
        }),
    }
}
