//! Shared scaffolding for the FABB component family.
//!
//! `fabb.js` gets this from a `Fabb` base class every component extends:
//! shadow attach, the `fabb-*` event emitter, and the block-cursor editable.
//! Rust has no class inheritance, so the same surface is free functions over
//! the host element, called from each component's `connected_callback`.
//!
//! One scheme — the chrome is light (law 8). The mode plumbing left with the
//! dark twin; it was the only thing that walked the light-DOM children.

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{CustomEvent, CustomEventInit, Element, HtmlElement, Range, ShadowRoot, window};

use crate::skin::SKIN;

/// An event listener that detaches itself when dropped.
///
/// Dropping a bare `Closure` does NOT remove the listener it was registered
/// with — it invalidates the function, leaving a registration that throws
/// "closure invoked after being dropped" the next time it fires. That is not
/// theoretical here: `mi` and `menu` reset their wiring on disconnect and
/// re-wire on reconnect, and the bar's in-place sub-stack disclosure moves a
/// stack between parents, so those elements disconnect and come back during
/// ordinary use. Holding the target and the event name alongside the closure
/// is what lets `Drop` undo the registration properly.
pub struct Bound {
    target: web_sys::EventTarget,
    event: &'static str,
    closure: Closure<dyn FnMut(web_sys::Event)>,
}

impl Drop for Bound {
    fn drop(&mut self) {
        let _ = self
            .target
            .remove_event_listener_with_callback(self.event, self.closure.as_ref().unchecked_ref());
    }
}

/// Register `handler` for `event` on `target`, detaching when the returned
/// [`Bound`] is dropped.
pub fn bind(
    target: &web_sys::EventTarget,
    event: &'static str,
    handler: impl FnMut(web_sys::Event) + 'static,
) -> Bound {
    let closure: Closure<dyn FnMut(web_sys::Event)> = Closure::wrap(Box::new(handler));
    let _ = target.add_event_listener_with_callback(event, closure.as_ref().unchecked_ref());
    Bound {
        target: target.clone(),
        event,
        closure,
    }
}

fn document() -> web_sys::Document {
    window()
        .expect("a window")
        .document()
        .expect("a document on the window")
}

/// Create an element, panicking only on a genuinely impossible failure
/// (`createElement` rejects nothing we pass it).
pub fn el(tag: &str) -> Element {
    document().create_element(tag).expect("createElement")
}

/// Attach the shadow root, or return the one already attached.
///
/// Attach is done here rather than through `CustomElement::shadow()` so the
/// component controls build timing — the trait's own attach happens before
/// `connected_callback`, which is too early for components that need to read
/// their attributes first.
pub fn ensure_shadow(this: &HtmlElement) -> ShadowRoot {
    if let Some(root) = this.shadow_root() {
        return root;
    }
    let init = web_sys::ShadowRootInit::new(web_sys::ShadowRootMode::Open);
    this.attach_shadow(&init).expect("attach_shadow")
}

/// Build a component's shadow root: [`SKIN`] plus `css`, then `html`.
///
/// `html` must contain the `.w` wrapper the tokens hang on — every component
/// in the family opens with `<div class="w">`.
pub fn build(this: &HtmlElement, css: &str, html: &str) -> ShadowRoot {
    let root = ensure_shadow(this);
    root.set_inner_html(&format!("<style>{SKIN}{css}</style>{html}"));
    root
}

/// Dispatch a composed, bubbling `fabb-*` event.
///
/// Composed because the whole point is that the host page hears it: an event
/// that does not cross the shadow boundary is invisible to the listener the
/// application actually installed.
pub fn emit(this: &HtmlElement, kind: &str, detail: &JsValue) {
    let init = CustomEventInit::new();
    init.set_bubbles(true);
    init.set_composed(true);
    init.set_detail(detail);
    if let Ok(event) = CustomEvent::new_with_event_init_dict(kind, &init) {
        let _ = this.dispatch_event(&event);
    }
}

/// Install a `click` listener, returning the binding to keep alive.
pub fn on_click(target: &Element, mut handler: impl FnMut() + 'static) -> Bound {
    bind(target, "click", move |_| handler())
}

/// Keep an element's animations frozen while the tab is hidden.
///
/// A blinking disc in a background tab is pure spend; the `vispause` class in
/// [`SKIN`] holds every animation's frame.
pub fn install_visibility_pause(this: &HtmlElement) -> Bound {
    let host = this.clone();
    bind(&document(), "visibilitychange", move |_| {
        let hidden = document().hidden();
        if let Some(root) = host.shadow_root()
            && let Ok(Some(w)) = root.query_selector(".w")
        {
            let _ = w.class_list().toggle_with_force("vispause", hidden);
        }
    })
}

/// A live in-place edit: the contenteditable span plus its block cursor.
///
/// Held behind an `Rc` by the caller, and deliberately NOT dropped when the
/// edit settles: the commit runs from inside this struct's own blur listener,
/// and dropping it there would free the closure currently executing. It is
/// released when the next edit replaces it, or when the element disconnects.
pub struct Edit {
    /// The editable span itself.
    pub span: HtmlElement,
    committed: Rc<RefCell<bool>>,
    /// The commit callback, so [`Edit::commit`] can settle the edit directly
    /// rather than going through focus.
    on_commit: Rc<dyn Fn(bool, String)>,
    /// Held so the listeners outlive the call.
    _listeners: Vec<Closure<dyn FnMut(web_sys::Event)>>,
}

impl Edit {
    /// Settle the edit without depending on focus.
    ///
    /// `blur()` is a no-op when the document itself is not focused, so a
    /// commit driven by anything other than the user — the bar folding, a
    /// drag starting — has to run the callback itself. The `committed` guard
    /// is what keeps the follow-up blur from committing a second time.
    pub fn commit(&self) {
        if *self.committed.borrow() {
            return;
        }
        *self.committed.borrow_mut() = true;
        let value = self
            .span
            .text_content()
            .unwrap_or_default()
            .trim()
            .to_string();
        (self.on_commit)(true, value);
        let _ = self.span.blur();
    }

    /// Focus the span and put the caret after the last character — where the
    /// block cursor is already drawn.
    pub fn focus_end(&self) {
        let _ = self.span.focus();
        let Some(win) = window() else { return };
        let Ok(range) = Range::new() else { return };
        let _ = range.select_node_contents(&self.span);
        range.collapse_with_to_start(false);
        if let Some(sel) = win.get_selection().ok().flatten() {
            let _ = sel.remove_all_ranges();
            let _ = sel.add_range(&range);
        }
    }
}

/// Mount an editable value with the terminal block cursor into `cell`.
///
/// The cursor is the affordance: it blinks on the last character at rest, so
/// the value reads as editable without a box, a pencil or a hover reveal.
/// `on_commit(accepted, value)` fires on Enter and blur with `true`, and on
/// Escape with `false` and the original text.
pub fn mount_edit(
    cell: &Element,
    initial: &str,
    on_commit: impl Fn(bool, String) + 'static,
) -> Edit {
    cell.set_text_content(Some(""));

    let span: HtmlElement = el("span").unchecked_into();
    span.set_class_name("edit");
    // `plaintext-only` keeps pasted markup out of a chrome label; Firefox
    // rejects the value, so fall back to the permissive mode there.
    if span
        .set_attribute("contenteditable", "plaintext-only")
        .is_err()
    {
        let _ = span.set_attribute("contenteditable", "true");
    }
    span.set_text_content(Some(initial));

    let cursor = el("i");
    cursor.set_class_name("cur");
    let _ = cursor.set_attribute("aria-hidden", "true");

    let _ = cell.append_child(&span);
    let _ = cell.append_child(&cursor);

    let committed = Rc::new(RefCell::new(false));
    let on_commit = Rc::new(on_commit);
    let original = initial.to_string();
    let mut listeners: Vec<Closure<dyn FnMut(web_sys::Event)>> = Vec::new();

    {
        let span_for_keys = span.clone();
        let commit = on_commit.clone();
        let committed = committed.clone();
        let original = original.clone();
        let cb: Closure<dyn FnMut(web_sys::Event)> =
            Closure::wrap(Box::new(move |ev: web_sys::Event| {
                let Some(ev) = ev.dyn_ref::<web_sys::KeyboardEvent>() else {
                    return;
                };
                match ev.key().as_str() {
                    "Enter" => {
                        ev.prevent_default();
                        if !*committed.borrow() {
                            *committed.borrow_mut() = true;
                            commit(
                                true,
                                span_for_keys
                                    .text_content()
                                    .unwrap_or_default()
                                    .trim()
                                    .to_string(),
                            );
                        }
                        let _ = span_for_keys.blur();
                    }
                    "Escape" => {
                        // Stop here: an Escape aimed at the text must not also
                        // close the stack the edit is sitting in.
                        ev.stop_propagation();
                        if !*committed.borrow() {
                            *committed.borrow_mut() = true;
                            commit(false, original.clone());
                        }
                        let _ = span_for_keys.blur();
                    }
                    _ => {}
                }
            }));
        let _ = span.add_event_listener_with_callback("keydown", cb.as_ref().unchecked_ref());
        listeners.push(cb);
    }

    {
        let span_for_blur = span.clone();
        let commit = on_commit.clone();
        let committed = committed.clone();
        let cb: Closure<dyn FnMut(web_sys::Event)> =
            Closure::wrap(Box::new(move |_: web_sys::Event| {
                if *committed.borrow() {
                    return;
                }
                *committed.borrow_mut() = true;
                commit(
                    true,
                    span_for_blur
                        .text_content()
                        .unwrap_or_default()
                        .trim()
                        .to_string(),
                );
            }));
        let _ = span.add_event_listener_with_callback("blur", cb.as_ref().unchecked_ref());
        listeners.push(cb);
    }

    Edit {
        span,
        committed,
        on_commit,
        _listeners: listeners,
    }
}
