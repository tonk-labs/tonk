//! Browser presenter for one task contained by the mounted FABB.
//!
//! The normal bar stays alive behind the task. Its DOM, open panel and
//! subscribers are never rebuilt, so resolving the task can put the exact
//! prior surface and focus target back rather than reconstructing an
//! approximation of them.

use std::cell::RefCell;
use std::rc::Rc;

use js_sys::{Error, Function, Object, Promise, Reflect, TypeError};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{Element, HtmlDialogElement, HtmlElement, Node, window};

use crate::shadow::{self, Bound};
use crate::task::{Controller, Dismissal, Outcome, RequestId};

#[derive(Clone)]
struct Origin {
    parent: Option<Node>,
    next: Option<Node>,
    slot: Option<String>,
    hidden: bool,
}

#[derive(Clone)]
struct Anchor {
    right: bool,
    bottom: bool,
    x: f64,
    y: f64,
    width: f64,
}

#[derive(Clone)]
struct ScrollPosition {
    element: Element,
    top: f64,
    left: f64,
}

#[derive(Clone)]
struct Snapshot {
    content: HtmlElement,
    origin: Origin,
    focus: Option<HtmlElement>,
    scroll: Vec<ScrollPosition>,
    width: String,
    height: String,
    anchor: Anchor,
    resolve: Function,
}

#[derive(Default)]
pub(crate) struct State {
    controller: Controller<Snapshot>,
}

pub(crate) type Shared = Rc<RefCell<State>>;

pub(crate) fn install(this: &HtmlElement, state: &Shared) -> Vec<Bound> {
    install_api(this, state);
    let mut listeners = Vec::new();

    if let Some(dialog) = native_dialog(this) {
        let host = this.clone();
        let shared = state.clone();
        listeners.push(shadow::bind(&dialog, "cancel", move |event| {
            event.prevent_default();
            if shared.borrow().controller.dismissal() == Some(Dismissal::Optional) {
                resolve_current(&host, &shared, Outcome::Cancelled);
            }
        }));
    }

    if let Some(ack) = query_shadow(this, ".task-ack") {
        let host = this.clone();
        let shared = state.clone();
        listeners.push(shadow::on_click(&ack, move || {
            resolve_current(&host, &shared, Outcome::Acknowledged)
        }));
    }

    {
        let host = this.clone();
        let shared = state.clone();
        listeners.push(shadow::bind(this, "click", move |event| {
            let Some(result) = event
                .target()
                .and_then(|target| target.dyn_into::<Element>().ok())
                .and_then(|target| target.closest("[data-fabb-result]").ok().flatten())
                .and_then(|target| target.get_attribute("data-fabb-result"))
            else {
                return;
            };
            let outcome = match result.as_str() {
                "cancel" | "cancelled" => Outcome::Cancelled,
                "acknowledge" | "acknowledged" => Outcome::Acknowledged,
                _ => Outcome::Completed(result),
            };
            resolve_current(&host, &shared, outcome);
        }));
    }

    {
        let host = this.clone();
        let shared = state.clone();
        listeners.push(shadow::bind(this, "fabb-bail", move |_| {
            resolve_current(&host, &shared, Outcome::Cancelled);
        }));
    }

    if let Some(win) = window() {
        let host = this.clone();
        let shared = state.clone();
        listeners.push(shadow::bind(&win, "resize", move |_| {
            layout(&host, &shared)
        }));
        if let Some(viewport) = win.visual_viewport() {
            for event in ["resize", "scroll"] {
                let host = this.clone();
                let shared = state.clone();
                listeners.push(shadow::bind(&viewport, event, move |_| {
                    layout(&host, &shared)
                }));
            }
        }
    }
    listeners
}

/// Present existing guest-owned content through the FABB's installed task API.
///
/// This keeps feature modules out of the controller internals while ensuring
/// every in-space prompt uses the same ownership, focus, and teardown rules.
pub(crate) fn present_element(
    this: &HtmlElement,
    content: &HtmlElement,
    heading: &str,
    dismissible: bool,
) -> Result<Promise, JsValue> {
    let present = Reflect::get(this, &"present".into())?.dyn_into::<Function>()?;
    let options = Object::new();
    Reflect::set(&options, &"heading".into(), &JsValue::from_str(heading))?;
    Reflect::set(
        &options,
        &"dismissible".into(),
        &JsValue::from_bool(dismissible),
    )?;
    present
        .call2(this, content.as_ref(), options.as_ref())?
        .dyn_into::<Promise>()
}

pub(crate) fn disconnect(this: &HtmlElement, state: &Shared) {
    let completion = state.borrow_mut().controller.disconnect();
    if let Some(completion) = completion {
        restore(this, completion.snapshot, completion.outcome);
    }
}

fn install_api(this: &HtmlElement, state: &Shared) {
    let host = this.clone();
    let shared = state.clone();
    let present = Closure::<dyn FnMut(JsValue, JsValue) -> Promise>::new(
        move |content: JsValue, options: JsValue| present(&host, &shared, content, options),
    );
    let _ = Reflect::set(this, &"present".into(), present.as_ref());
    present.forget();

    let host = this.clone();
    let shared = state.clone();
    let notify = Closure::<dyn FnMut(String, JsValue) -> Promise>::new(
        move |message: String, options: JsValue| notify(&host, &shared, &message, options),
    );
    let _ = Reflect::set(this, &"notify".into(), notify.as_ref());
    notify.forget();

    let host = this.clone();
    let shared = state.clone();
    let resolve = Closure::<dyn FnMut(JsValue)>::new(move |result: JsValue| {
        resolve_current(&host, &shared, outcome_from_js(result));
    });
    let _ = Reflect::set(this, &"resolve".into(), resolve.as_ref());
    resolve.forget();

    let host = this.clone();
    let shared = state.clone();
    let resolve_request =
        Closure::<dyn FnMut(f64, JsValue)>::new(move |id: f64, result: JsValue| {
            if id.is_finite() && id >= 1.0 {
                resolve_id(
                    &host,
                    &shared,
                    RequestId::from_raw(id as u64),
                    outcome_from_js(result),
                );
            }
        });
    let _ = Reflect::set(this, &"resolveRequest".into(), resolve_request.as_ref());
    resolve_request.forget();

    define_getter(this, "requesting", {
        let shared = state.clone();
        move || JsValue::from_bool(shared.borrow().controller.active_id().is_some())
    });
    define_getter(this, "requestId", {
        let shared = state.clone();
        move || {
            shared
                .borrow()
                .controller
                .active_id()
                .map(|id| JsValue::from_f64(id.get() as f64))
                .unwrap_or(JsValue::UNDEFINED)
        }
    });
}

fn define_getter(this: &HtmlElement, name: &str, getter: impl FnMut() -> JsValue + 'static) {
    let descriptor = Object::new();
    let getter = Closure::<dyn FnMut() -> JsValue>::new(getter);
    let _ = Reflect::set(&descriptor, &"get".into(), getter.as_ref());
    getter.forget();
    let _ = Reflect::set(&descriptor, &"configurable".into(), &JsValue::TRUE);
    let _ = Object::define_property(this, &JsValue::from_str(name), &descriptor);
}

fn present(this: &HtmlElement, state: &Shared, value: JsValue, options: JsValue) -> Promise {
    let Ok(content) = value.dyn_into::<HtmlElement>() else {
        return rejected(TypeError::new("Provide a content element.").into());
    };
    if !this.is_connected() {
        return rejected(Error::new("Connect the FABB before presenting a request.").into());
    }
    let this_node: &Node = this.unchecked_ref();
    if content.is_same_node(Some(this_node)) || content.contains(Some(this_node)) {
        return rejected(
            TypeError::new("Provide a content element, not the FABB or its ancestor.").into(),
        );
    }
    if state.borrow().controller.active_id().is_some() {
        return rejected(
            Error::new("Resolve the current FABB request before presenting another.").into(),
        );
    }

    let heading = option_string(&options, "heading").unwrap_or_else(|| "attention needed".into());
    let dismissal = if option_bool(&options, "dismissible") {
        Dismissal::Optional
    } else {
        Dismissal::Required
    };
    let promise_resolver = Rc::new(RefCell::new(None::<Function>));
    let sink = promise_resolver.clone();
    let promise = Promise::new(&mut move |resolve, _reject| {
        *sink.borrow_mut() = Some(resolve);
    });
    let Some(resolve) = promise_resolver.borrow_mut().take() else {
        return rejected(Error::new("Unable to create the request promise.").into());
    };

    let rect = this.get_bounding_client_rect();
    let snapshot = Snapshot {
        origin: Origin {
            parent: content.parent_node(),
            next: content.next_sibling(),
            slot: content.get_attribute("slot"),
            hidden: content.hidden(),
        },
        focus: deepest_focus(),
        scroll: scroll_positions(this),
        width: this.style().get_property_value("width").unwrap_or_default(),
        height: this
            .style()
            .get_property_value("height")
            .unwrap_or_default(),
        anchor: Anchor {
            right: this.has_attribute("flip"),
            bottom: this.has_attribute("up"),
            x: if this.has_attribute("flip") {
                rect.right()
            } else {
                rect.left()
            },
            y: if this.has_attribute("up") {
                rect.bottom()
            } else {
                rect.top()
            },
            width: rect.width().clamp(360.0, 480.0),
        },
        content: content.clone(),
        resolve,
    };
    let id = match state.borrow_mut().controller.begin(snapshot, dismissal) {
        Ok(id) => id,
        Err(_) => {
            return rejected(
                Error::new("Resolve the current FABB request before presenting another.").into(),
            );
        }
    };

    if let Err(error) = mount(this, state, &content, &heading, id) {
        let completion = state
            .borrow_mut()
            .controller
            .resolve(id, Outcome::Disconnected);
        if let Some(completion) = completion {
            restore(this, completion.snapshot, completion.outcome);
        }
        return rejected(error);
    }
    promise
}

fn notify(this: &HtmlElement, state: &Shared, message: &str, options: JsValue) -> Promise {
    let Some(document) = window().and_then(|win| win.document()) else {
        return rejected(Error::new("No document is available.").into());
    };
    let Ok(content) = document.create_element("section") else {
        return rejected(Error::new("Unable to create the notification.").into());
    };
    let Ok(content) = content.dyn_into::<HtmlElement>() else {
        return rejected(Error::new("Unable to create the notification.").into());
    };
    content.set_text_content(Some(message));
    let _ = content.set_attribute("role", "status");
    let _ = content.set_attribute("data-fabb-notification", "");
    let normalized = if options.is_null() || options.is_undefined() {
        Object::new().into()
    } else {
        options
    };
    let _ = Reflect::set(&normalized, &"dismissible".into(), &JsValue::FALSE);
    present(this, state, content.into(), normalized)
}

fn mount(
    this: &HtmlElement,
    state: &Shared,
    content: &HtmlElement,
    heading: &str,
    id: RequestId,
) -> Result<(), JsValue> {
    let wrapper = wrapper(this).ok_or_else(|| Error::new("The FABB surface is unavailable."))?;
    let dialog = native_dialog(this).ok_or_else(|| Error::new("The task host is unavailable."))?;
    let task = query_shadow(this, ".task").ok_or_else(|| Error::new("The task is unavailable."))?;
    let title = query_shadow(this, ".task-title")
        .ok_or_else(|| Error::new("The task title is unavailable."))?;
    title.set_text_content(Some(heading));
    task.remove_attribute("hidden")?;
    wrapper.class_list().add_1("requesting")?;
    this.set_attribute("data-fabb-request", &id.get().to_string())?;
    content.set_attribute("slot", "request")?;
    content.set_hidden(false);
    this.append_child(content)?;

    let notification = content.has_attribute("data-fabb-notification");
    if let Some(ack) = query_shadow(this, ".task-ack") {
        ack.toggle_attribute_with_force("hidden", !notification)?;
    }
    if let Some(actions) = query_shadow(this, ".task-actions") {
        actions.toggle_attribute_with_force("hidden", !notification)?;
    }

    let rect = this.get_bounding_client_rect();
    this.style()
        .set_property("width", &format!("{}px", rect.width()))?;
    this.style()
        .set_property("height", &format!("{}px", rect.height()))?;
    dialog.append_child(&wrapper)?;
    layout(this, state);
    dialog.show_modal()?;
    layout(this, state);
    focus_task(this, content);
    shadow::emit(
        this,
        "fabb-request-open",
        &JsValue::from_f64(id.get() as f64),
    );
    Ok(())
}

fn resolve_current(this: &HtmlElement, state: &Shared, outcome: Outcome) {
    let Some(id) = state.borrow().controller.active_id() else {
        return;
    };
    resolve_id(this, state, id, outcome);
}

fn resolve_id(this: &HtmlElement, state: &Shared, id: RequestId, outcome: Outcome) {
    let completion = state.borrow_mut().controller.resolve(id, outcome);
    if let Some(completion) = completion {
        restore(this, completion.snapshot, completion.outcome);
    }
}

fn restore(this: &HtmlElement, snapshot: Snapshot, outcome: Outcome) {
    if let Some(dialog) = native_dialog(this) {
        if dialog.open() {
            dialog.close();
        }
        if let Some(parent) = dialog.parent_node()
            && let Some(wrapper) = wrapper(this)
        {
            let _ = parent.insert_before(&wrapper, Some(dialog.unchecked_ref()));
        }
    }
    if let Some(wrapper) = wrapper(this) {
        let _ = wrapper.class_list().remove_1("requesting");
        let _ = wrapper.style().remove_property("--_task-width");
        let _ = wrapper.style().remove_property("--_task-height");
    }
    if let Some(task) = query_shadow(this, ".task") {
        let _ = task.set_attribute("hidden", "");
    }
    let _ = this.remove_attribute("data-fabb-request");
    restore_style(&this.style(), "width", &snapshot.width);
    restore_style(&this.style(), "height", &snapshot.height);

    match snapshot.origin.slot {
        Some(slot) => {
            let _ = snapshot.content.set_attribute("slot", &slot);
        }
        None => {
            let _ = snapshot.content.remove_attribute("slot");
        }
    }
    snapshot.content.set_hidden(snapshot.origin.hidden);
    if let Some(parent) = snapshot.origin.parent {
        let next = snapshot
            .origin
            .next
            .filter(|next| next.parent_node().is_some_and(|owner| owner == parent));
        let _ = parent.insert_before(&snapshot.content, next.as_ref());
    } else {
        snapshot.content.remove();
    }
    for position in snapshot.scroll {
        position.element.set_scroll_top(position.top);
        position.element.set_scroll_left(position.left);
    }
    if this.is_connected() {
        if let Some(focus) = snapshot.focus.filter(focusable_now) {
            let _ = focus.focus();
        } else if let Some(circle) =
            query_shadow(this, ".fab").and_then(|el| el.dyn_into::<HtmlElement>().ok())
        {
            let _ = circle.focus();
        }
    }
    let result = JsValue::from_str(outcome.as_str());
    let _ = snapshot.resolve.call1(&JsValue::UNDEFINED, &result);
    shadow::emit(this, "fabb-request-close", &result);
}

fn layout(this: &HtmlElement, state: &Shared) {
    let Some(dialog) = native_dialog(this) else {
        return;
    };
    if !dialog.open() && state.borrow().controller.active_id().is_none() {
        return;
    }
    let anchor = {
        let state = state.borrow();
        let Some(snapshot) = state.controller.active_snapshot() else {
            return;
        };
        snapshot.anchor.clone()
    };
    let (x, y, width, height) = viewport();
    let task_width = anchor.width.min((width - 32.0).max(0.0));
    let available_height = (height - 32.0).max(0.0);
    if let Some(wrapper) = wrapper(this) {
        let _ = wrapper
            .style()
            .set_property("--_task-width", &format!("{task_width}px"));
        let _ = wrapper
            .style()
            .set_property("--_task-height", &format!("{available_height}px"));
    }
    let rect = dialog.get_bounding_client_rect();
    let min_left = x + 16.0;
    let max_left = (x + width - task_width - 16.0).max(min_left);
    let left = (if anchor.right {
        anchor.x - task_width
    } else {
        anchor.x
    })
    .clamp(min_left, max_left);
    let min_top = y + 16.0;
    let max_top = (y + height - rect.height() - 16.0).max(min_top);
    let top = (if anchor.bottom {
        anchor.y - rect.height()
    } else {
        anchor.y
    })
    .clamp(min_top, max_top);
    let _ = dialog.style().set_property("left", &format!("{left}px"));
    let _ = dialog.style().set_property("top", &format!("{top}px"));
}

fn viewport() -> (f64, f64, f64, f64) {
    let Some(win) = window() else {
        return (0.0, 0.0, 1024.0, 768.0);
    };
    if let Some(viewport) = win.visual_viewport() {
        return (
            viewport.offset_left(),
            viewport.offset_top(),
            viewport.width(),
            viewport.height(),
        );
    }
    (
        0.0,
        0.0,
        win.inner_width()
            .ok()
            .and_then(|value| value.as_f64())
            .unwrap_or(1024.0),
        win.inner_height()
            .ok()
            .and_then(|value| value.as_f64())
            .unwrap_or(768.0),
    )
}

fn scroll_positions(this: &HtmlElement) -> Vec<ScrollPosition> {
    let Some(wrapper) = wrapper(this) else {
        return Vec::new();
    };
    let mut positions = Vec::new();
    push_scroll(&mut positions, wrapper.clone().into());
    if let Ok(elements) = wrapper.query_selector_all("*") {
        for index in 0..elements.length() {
            if let Some(element) = elements.item(index).and_then(|node| node.dyn_into().ok()) {
                push_scroll(&mut positions, element);
            }
        }
    }
    positions
}

fn push_scroll(positions: &mut Vec<ScrollPosition>, element: Element) {
    let top = element.scroll_top();
    let left = element.scroll_left();
    if top != 0.0 || left != 0.0 {
        positions.push(ScrollPosition { element, top, left });
    }
}

fn deepest_focus() -> Option<HtmlElement> {
    let mut focused = window()?.document()?.active_element()?;
    loop {
        let Some(next) = focused.shadow_root().and_then(|root| root.active_element()) else {
            break;
        };
        focused = next;
    }
    focused.dyn_into().ok()
}

fn focus_task(this: &HtmlElement, content: &HtmlElement) {
    let target = content
        .query_selector("[autofocus],input:not([type=hidden]),button,select,textarea,[contenteditable=true],[tabindex]")
        .ok()
        .flatten()
        .and_then(|element| element.dyn_into::<HtmlElement>().ok())
        .or_else(|| query_shadow(this, ".task-title").and_then(|element| element.dyn_into().ok()));
    if let Some(target) = target {
        let _ = target.focus();
    }
}

fn focusable_now(element: &HtmlElement) -> bool {
    element.is_connected()
        && !element.has_attribute("disabled")
        && !element.has_attribute("hidden")
        && {
            let rect = element.get_bounding_client_rect();
            rect.width() > 0.0 && rect.height() > 0.0
        }
}

fn restore_style(style: &web_sys::CssStyleDeclaration, property: &str, value: &str) {
    if value.is_empty() {
        let _ = style.remove_property(property);
    } else {
        let _ = style.set_property(property, value);
    }
}

fn option_string(options: &JsValue, name: &str) -> Option<String> {
    Reflect::get(options, &JsValue::from_str(name))
        .ok()
        .and_then(|value| value.as_string())
        .filter(|value| !value.is_empty())
}

fn option_bool(options: &JsValue, name: &str) -> bool {
    Reflect::get(options, &JsValue::from_str(name))
        .ok()
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn outcome_from_js(value: JsValue) -> Outcome {
    match value.as_string().as_deref() {
        Some("cancel") | Some("cancelled") => Outcome::Cancelled,
        Some("acknowledge") | Some("acknowledged") => Outcome::Acknowledged,
        Some(value) => Outcome::Completed(value.to_owned()),
        None => Outcome::Acknowledged,
    }
}

fn rejected(error: JsValue) -> Promise {
    Promise::reject(&error)
}

fn wrapper(this: &HtmlElement) -> Option<HtmlElement> {
    query_shadow(this, ".w")?.dyn_into().ok()
}

fn native_dialog(this: &HtmlElement) -> Option<HtmlDialogElement> {
    query_shadow(this, ".request-layer")?.dyn_into().ok()
}

fn query_shadow(this: &HtmlElement, selector: &str) -> Option<Element> {
    this.shadow_root()?.query_selector(selector).ok().flatten()
}
