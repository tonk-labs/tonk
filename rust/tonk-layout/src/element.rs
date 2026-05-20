//! `<tonk-layout>` custom-element implementation.
//!
//! A niri-style tiling window manager: an infinite horizontal
//! scrollable strip of columns, each column a vertical stack of
//! tiles. The layout itself is persisted to the branch as
//! normalized entities (see `/plan/tonk-layout.md`).
//!
//! This module owns the element lifecycle. Step 1 only observes
//! attributes and reflects `data-state="loading"`; subscription
//! wiring and rendering land in later steps.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use web_sys::{AbortController, Element, HtmlElement, window};

use crate::state::{self, State};

/// Default workspace name when the `workspace` attribute is absent.
const DEFAULT_WORKSPACE: &str = "default";

/// Internal lifecycle state shared across async closures.
///
/// `custom-elements` requires the element struct itself to be
/// `Default`, and there is no host element until
/// `connected_callback`. So the real state lives here and is only
/// allocated once we are connected.
struct Inner {
    /// Aborts in-flight SSE subscriptions when the element
    /// disconnects or an observed attribute changes. Populated
    /// once the read path lands (step 4).
    abort: Option<AbortController>,
}

impl Inner {
    fn new() -> Self {
        Self { abort: None }
    }

    /// Cancel any in-flight subscription. Safe to call when none
    /// is open.
    fn abort(&mut self) {
        if let Some(controller) = self.abort.take() {
            controller.abort();
        }
    }
}

/// The custom element. Holds no fields directly — see [`Inner`].
#[derive(Default)]
pub struct TonkLayout {
    inner: RefCell<Option<Rc<RefCell<Inner>>>>,
}

impl CustomElement for TonkLayout {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["workspace", "space", "branch"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        let host: Element = this.clone().into();
        state::set(&host, State::Loading);

        let inner = Rc::new(RefCell::new(Inner::new()));
        *self.inner.borrow_mut() = Some(inner.clone());

        start(&host, inner);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        if let Some(inner) = self.inner.borrow_mut().take() {
            inner.borrow_mut().abort();
        }
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        _name: String,
        _old: Option<String>,
        _new: Option<String>,
    ) {
        let host: Element = this.clone().into();
        let Some(inner) = self.inner.borrow().clone() else {
            return;
        };
        // Cancel the current subscription and restart against the
        // new attributes.
        inner.borrow_mut().abort();
        state::set(&host, State::Loading);
        start(&host, inner);
    }
}

/// Resolve the workspace name from the host's `workspace`
/// attribute, falling back to [`DEFAULT_WORKSPACE`].
fn workspace_name(host: &Element) -> String {
    match host.get_attribute("workspace") {
        Some(name) if !name.is_empty() => name,
        _ => DEFAULT_WORKSPACE.to_string(),
    }
}

/// Begin (or restart) the element's data flow.
///
/// Step 1 stub: subscriptions are not opened yet, so an empty
/// strip is the steady state. Steps 3-5 replace this body with
/// the real read path.
fn start(host: &Element, _inner: Rc<RefCell<Inner>>) {
    let _ = workspace_name(host);
    // No columns yet — reflect the empty state so authors can
    // style the placeholder.
    state::set(host, State::Empty);
}

/// Register `<tonk-layout>` with the custom-element registry.
/// Idempotent — repeated calls after the first are no-ops.
pub fn register() {
    if already_registered() {
        return;
    }
    TonkLayout::define("tonk-layout");
}

/// True once `<tonk-layout>` is in the registry.
fn already_registered() -> bool {
    window()
        .map(|w| !w.custom_elements().get("tonk-layout").is_undefined())
        .unwrap_or(false)
}
