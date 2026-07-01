//! `<ui-dropdown>` — a light-DOM wrapper that hides an excluded row.
//!
//! Host chrome, NOT space content: it wraps a menu whose rows are produced
//! elsewhere (e.g. the FAB switcher's `<tonk-display model="space"
//! view="tonk:view/fab-menu">` roster) and drops the ONE row that names the
//! space given in its `exclude` attribute — so the current space doesn't list
//! itself in its own switcher. Defined in Rust; the `ui-` prefix marks it a
//! host UI primitive, like `<ui-sync-status>`, distinct from the `tonk-` data
//! elements it contains.
//!
//! Filtering stays out of `<tonk-display>` (which has no notion of a "current"
//! instance): the roster renders every space, and this wrapper hides the
//! excluded one. A row is a link to `/space/<did>`; the excluded space is
//! matched by that `href` suffix, so no per-row data attribute is required.
//!
//! Rows arrive asynchronously (the roster's subscription frames land after
//! mount), so a `MutationObserver` re-applies the filter as rows are added,
//! and an `exclude` change re-scans in place.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::*;
use web_sys::{Element, HtmlElement, MutationObserver, MutationObserverInit, window};

/// The class of a menu row (roster link or action). Only roster links carry a
/// `/space/<did>` href, so the suffix match below never hides an action row.
const MENU_ITEM_SELECTOR: &str = ".fab__menu-item";

/// The `MutationObserver` callback closure, kept alive for the element's life.
type ObserverClosure = Closure<dyn FnMut(JsValue, JsValue)>;

/// Per-element state: the observer watching for async-rendered rows (its `Drop`
/// disconnects it) and the callback closure, kept alive for the element's life.
#[derive(Default)]
pub(crate) struct UiDropdown {
    observer: Rc<RefCell<Option<MutationObserver>>>,
    callback: Rc<RefCell<Option<ObserverClosure>>>,
}

impl CustomElement for UiDropdown {
    fn shadow() -> bool {
        // Light DOM: the wrapped menu's rows and CSS live in the caller's tree,
        // and this element only toggles their visibility.
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        // `exclude` is the space to hide; a change re-scans the current rows.
        &["exclude"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        apply_filter(this);

        // Watch the subtree: the roster's rows are appended after its
        // subscription frame lands, so re-apply the filter as children change.
        let host = this.clone();
        let callback: ObserverClosure =
            Closure::wrap(Box::new(move |_records: JsValue, _observer: JsValue| {
                apply_filter(&host);
            }));
        if let Ok(observer) = MutationObserver::new(callback.as_ref().unchecked_ref()) {
            let init = MutationObserverInit::new();
            init.set_child_list(true);
            init.set_subtree(true);
            let _ = observer.observe_with_options(this, &init);
            *self.observer.borrow_mut() = Some(observer);
        }
        *self.callback.borrow_mut() = Some(callback);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        if let Some(observer) = self.observer.borrow_mut().take() {
            observer.disconnect();
        }
        self.callback.borrow_mut().take();
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        _name: String,
        _old: Option<String>,
        _new: Option<String>,
    ) {
        apply_filter(this);
    }
}

/// Hide the row that links to the excluded space; show every other row. Runs
/// on connect, on each observed mutation, and on an `exclude` change — always
/// reconciling from the full row set, so a previously-hidden row reappears if
/// `exclude` clears or changes.
fn apply_filter(host: &HtmlElement) {
    let excluded = host
        .get_attribute("exclude")
        .filter(|value| !value.is_empty());
    let Ok(rows) = host.query_selector_all(MENU_ITEM_SELECTOR) else {
        return;
    };
    for index in 0..rows.length() {
        let Some(node) = rows.item(index) else {
            continue;
        };
        let Ok(row) = node.dyn_into::<HtmlElement>() else {
            continue;
        };
        let hide = excluded
            .as_deref()
            .is_some_and(|did| links_to_space(&row, did));
        set_hidden(&row, hide);
    }
}

/// Whether `row` is a link to `/space/<did>` — the switcher roster's row shape.
/// Matches the `href`'s path suffix so a full or relative URL both resolve.
fn links_to_space(row: &HtmlElement, did: &str) -> bool {
    let element: &Element = row.as_ref();
    element
        .get_attribute("href")
        .is_some_and(|href| href.ends_with(&format!("/space/{did}")))
}

/// Toggle a row's visibility with an inline `display`, so no layout box is
/// left behind (the caller's row gap simply skips it). Inline style — not the
/// `hidden` attribute — because the menu rows set an explicit `display: block`
/// / `display: flex` in the app stylesheet, which OVERRIDES the UA `[hidden] {
/// display: none }` rule; an inline `display: none` outranks the class. Clearing
/// the property (not setting `display: block`) restores whatever the row's
/// class dictates, so an action row's `flex` layout is untouched when shown.
fn set_hidden(row: &HtmlElement, hidden: bool) {
    let style = row.style();
    if hidden {
        let _ = style.set_property("display", "none");
    } else {
        let _ = style.remove_property("display");
    }
}

/// Register `<ui-dropdown>`. Idempotent.
pub(crate) fn register() {
    let Some(win) = window() else {
        return;
    };
    if win.custom_elements().get("ui-dropdown").is_undefined() {
        UiDropdown::define("ui-dropdown");
    }
}
