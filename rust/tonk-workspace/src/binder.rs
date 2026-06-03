//! `<tonk-sheet-binder active="…">` — a tab/sheet binder.
//!
//! Models `<wa-tab-group>`: it accepts `<tonk-sheet>` children (each a
//! sheet, like `<wa-tab>`) and **projects** the tab strip from them —
//! the consuming view declares only the sheets, never the tabs.
//!
//! Responsibilities:
//!
//! 1. **Project tabs** — for each `<tonk-sheet>` child it builds one
//!    tab button (bullet + title) in a strip it owns, keyed by the
//!    sheet's `sheet` attribute.
//! 2. **Active** — the `active` attribute names the live sheet. The
//!    matching `<tonk-sheet>` panel is shown (the rest hidden) and the
//!    matching tab gets `is-active`.
//! 3. **Ordering** — sheets and their tabs are ordered by each
//!    sheet's `order` attribute. The view's `{sheet}` iteration is
//!    CID-keyed (not author-controllable), so the binder is what makes
//!    the order deterministic.
//! 4. **Selection** — clicking a tab dispatches a bubbling `activate`
//!    `CustomEvent` whose `detail.sheet` is the sheet id. The binder
//!    does not mutate state; the view wires the event to a command
//!    (`onactivate=workspace/activate-sheet`), keeping selection in
//!    the data model. The resulting `active` attribute flows back and
//!    the binder reflects it.
//!
//! `<tonk-sheet>` children arrive asynchronously (the view mounts each
//! through a `<tonk-display>`), so a `MutationObserver` re-projects as
//! they land.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{
    CustomEvent, CustomEventInit, Document, Element, Event, HtmlElement, MutationObserver,
    MutationObserverInit, window,
};

/// A retained event-listener closure, kept alive for the element's
/// lifetime so the listener stays valid.
type ClickClosure = Rc<RefCell<Option<Closure<dyn FnMut(Event)>>>>;
/// A retained MutationObserver callback closure.
type MutationClosure = Rc<RefCell<Option<Closure<dyn FnMut()>>>>;
/// The retained MutationObserver itself.
type ObserverCell = Rc<RefCell<Option<MutationObserver>>>;

/// Per-element state. Holds the observer/closures so they live as
/// long as the element and drop on disconnect.
#[derive(Default)]
pub(crate) struct TonkSheetBinder {
    observer: ObserverCell,
    click: ClickClosure,
    mutation: MutationClosure,
}

impl CustomElement for TonkSheetBinder {
    fn shadow() -> bool {
        // Light DOM: the consuming view's stylesheet styles the
        // projected tab strip and the sheet panels.
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["active"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        install_click(this, &self.click);
        install_observer(this, &self.observer, &self.mutation);
        project(this);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        if let Some(observer) = self.observer.borrow_mut().take() {
            observer.disconnect();
        }
        self.click.borrow_mut().take();
        self.mutation.borrow_mut().take();
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        _name: String,
        old: Option<String>,
        new: Option<String>,
    ) {
        if old == new {
            return;
        }
        project(this);
    }
}

/// CSS class names the consuming view styles.
const STRIP: &str = "tonk-sheet-binder__tabs";
const TAB: &str = "tonk-sheet-binder__tab";
const ACTIVE: &str = "is-active";

/// Build/refresh the tab strip from the `<tonk-sheet>` children, then
/// apply ordering + the active state. Idempotent.
fn project(this: &HtmlElement) {
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    let active = this.get_attribute("active").unwrap_or_default();

    // Collect sheets in `order` order.
    let mut sheets = collect_sheets(this);
    sheets.sort_by(|a, b| a.order.cmp(&b.order));

    let strip = ensure_strip(this, &document);

    // Reconcile tab buttons against the sheets: build one tab per
    // sheet (reuse by `data-sheet`), drop tabs whose sheet vanished.
    reconcile_tabs(&document, &strip, &sheets, &active);

    // Order + show/hide the sheet panels by `order` / active.
    for (rank, sheet) in sheets.iter().enumerate() {
        if let Some(html) = sheet.el.dyn_ref::<HtmlElement>() {
            let _ = html.style().set_property("order", &rank.to_string());
        }
        if sheet.id == active {
            let _ = sheet.el.remove_attribute("hidden");
        } else {
            let _ = sheet.el.set_attribute("hidden", "");
        }
    }
}

/// A `<tonk-sheet>` child and its metadata.
struct Sheet {
    el: Element,
    id: String,
    order: String,
    title: String,
}

fn collect_sheets(this: &HtmlElement) -> Vec<Sheet> {
    let mut out = Vec::new();
    let Ok(list) = this.query_selector_all("tonk-sheet") else {
        return out;
    };
    for i in 0..list.length() {
        let Some(node) = list.item(i) else { continue };
        let Ok(el) = node.dyn_into::<Element>() else {
            continue;
        };
        out.push(Sheet {
            id: el.get_attribute("sheet").unwrap_or_default(),
            order: el.get_attribute("order").unwrap_or_default(),
            title: el.get_attribute("title").unwrap_or_default(),
            el,
        });
    }
    out
}

/// Find or create the binder-owned tab strip (its last child).
fn ensure_strip(this: &HtmlElement, document: &Document) -> Element {
    if let Ok(Some(existing)) = this.query_selector(&format!(":scope > .{STRIP}")) {
        return existing;
    }
    let strip = document.create_element("div").expect("create strip div");
    let _ = strip.set_attribute("class", STRIP);
    let _ = strip.set_attribute("part", "tabs");
    let _ = this.append_child(&strip);
    strip
}

/// Build one tab button per sheet (reusing by `data-sheet`), set its
/// label / order / active state, and drop stale tabs.
fn reconcile_tabs(document: &Document, strip: &Element, sheets: &[Sheet], active: &str) {
    use std::collections::BTreeSet;
    let live: BTreeSet<&str> = sheets.iter().map(|s| s.id.as_str()).collect();

    // Remove tabs whose sheet is gone.
    if let Ok(existing) = strip.query_selector_all(&format!(".{TAB}")) {
        for i in 0..existing.length() {
            if let Some(node) = existing.item(i)
                && let Ok(el) = node.dyn_into::<Element>()
            {
                let id = el.get_attribute("data-sheet").unwrap_or_default();
                if !live.contains(id.as_str()) {
                    el.remove();
                }
            }
        }
    }

    for (rank, sheet) in sheets.iter().enumerate() {
        let selector = format!(".{TAB}[data-sheet=\"{}\"]", css_escape(&sheet.id));
        let tab = match strip.query_selector(&selector) {
            Ok(Some(el)) => el,
            _ => {
                let el = document.create_element("button").expect("create tab");
                let _ = el.set_attribute("class", TAB);
                let _ = el.set_attribute("type", "button");
                let _ = el.set_attribute("data-sheet", &sheet.id);
                let _ = strip.append_child(&el);
                el
            }
        };
        tab.set_text_content(Some(&sheet.title));
        if let Some(html) = tab.dyn_ref::<HtmlElement>() {
            let _ = html.style().set_property("order", &rank.to_string());
        }
        let _ = tab
            .class_list()
            .toggle_with_force(ACTIVE, sheet.id == active);
    }
}

/// Minimal CSS-attribute-value escape for the `data-sheet` selector
/// (entity ids contain `:` / `/`). Escapes `"` and `\`.
fn css_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Delegated click on a projected tab: switch focus optimistically,
/// then dispatch `activate` for the command to persist.
fn install_click(this: &HtmlElement, slot: &ClickClosure) {
    let host = this.clone();
    let listener = Closure::wrap(Box::new(move |event: Event| {
        let Some(target) = event.target() else { return };
        let Ok(node) = target.dyn_into::<Element>() else {
            return;
        };
        let Ok(Some(tab)) = node.closest(&format!(".{TAB}")) else {
            return;
        };
        let Some(sheet) = tab.get_attribute("data-sheet") else {
            return;
        };
        // Switch immediately — don't wait for the event → command →
        // rule → DB roundtrip. Setting `active` triggers
        // `attribute_changed_callback`, which re-projects (the tab +
        // panel update without latency). The command (fired below)
        // persists the same value, which flows back idempotently.
        let _ = host.set_attribute("active", &sheet);
        // Persist: the consuming view wires `activate` to a command.
        dispatch_activate(&host, &sheet);
    }) as Box<dyn FnMut(Event)>);

    let _ = this.add_event_listener_with_callback("click", listener.as_ref().unchecked_ref());
    *slot.borrow_mut() = Some(listener);
}

/// Dispatch `activate` with `detail = { sheet }`, bubbling + composed.
fn dispatch_activate(host: &HtmlElement, sheet: &str) {
    let detail = js_sys::Object::new();
    let _ = js_sys::Reflect::set(
        &detail,
        &JsValue::from_str("sheet"),
        &JsValue::from_str(sheet),
    );
    let init = CustomEventInit::new();
    init.set_detail(&detail);
    init.set_bubbles(true);
    init.set_composed(true);
    if let Ok(event) = CustomEvent::new_with_event_init_dict("activate", &init) {
        let _ = host.dispatch_event(&event);
    }
}

/// Watch for `<tonk-sheet>` children mounting / changing so the
/// projection re-runs as they land.
fn install_observer(this: &HtmlElement, slot: &ObserverCell, mutation: &MutationClosure) {
    let host = this.clone();
    let observer_slot = slot.clone();
    let callback = Closure::wrap(Box::new(move || {
        // Disconnect while projecting so the binder's own tab-strip
        // writes don't re-trigger this callback (an infinite loop).
        // Re-observe afterwards to keep watching for real sheet
        // changes. MutationObserver records are async, so any writes
        // made while disconnected are simply not delivered.
        let observer = observer_slot.borrow().clone();
        if let Some(obs) = &observer {
            obs.disconnect();
        }
        project(&host);
        if let Some(obs) = &observer {
            reobserve(obs, &host);
        }
    }) as Box<dyn FnMut()>);

    let Ok(observer) = MutationObserver::new(callback.as_ref().unchecked_ref()) else {
        return;
    };
    reobserve(&observer, this);

    *mutation.borrow_mut() = Some(callback);
    *slot.borrow_mut() = Some(observer);
}

/// (Re-)observe the binder subtree for child + attribute changes.
fn reobserve(observer: &MutationObserver, target: &HtmlElement) {
    let init = MutationObserverInit::new();
    init.set_child_list(true);
    init.set_subtree(true);
    init.set_attributes(true);
    let _ = observer.observe_with_options(target, &init);
}

/// Register `<tonk-sheet-binder>`. Idempotent.
pub(crate) fn register() {
    if already_registered() {
        return;
    }
    TonkSheetBinder::define("tonk-sheet-binder");
}

fn already_registered() -> bool {
    let Some(win) = window() else {
        return false;
    };
    !win.custom_elements()
        .get("tonk-sheet-binder")
        .is_undefined()
}
