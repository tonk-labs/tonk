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
const TAB_LABEL: &str = "tonk-sheet-binder__tab-label";
const CLOSE: &str = "tonk-sheet-binder__close";
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

    // The sheet to *show*. Normally the one `active` names. If `active`
    // names no present sheet — a persisted stale pointer, or just the
    // transient frame mid-reconcile where the active sheet's
    // `<tonk-sheet>` was removed and not yet re-mounted — fall back to
    // the first sheet by order so a panel is always shown rather than
    // the layout collapsing.
    //
    // This is *display only*: we never write `active` back or dispatch
    // `activate` from here. Writing during the reconcile gap would
    // persist the wrong sheet and steal focus from the genuinely-active
    // one. Keeping `active` untouched means once the real active sheet
    // re-mounts the next `project()` shows it again, and a genuinely
    // stale pointer is corrected by the data model (the active-sheet
    // rules), not by the binder guessing.
    let shown = if sheets.iter().any(|s| s.id == active) {
        active.clone()
    } else {
        sheets.first().map(|s| s.id.clone()).unwrap_or_default()
    };

    let strip = ensure_strip(this, &document);

    // Reconcile tab buttons against the sheets: build one tab per
    // sheet (reuse by `data-sheet`), drop tabs whose sheet vanished.
    // The tab highlight follows the real `active`, not the display
    // fallback — a fallback panel shouldn't light up a tab as selected.
    reconcile_tabs(&document, &strip, &sheets, &active);

    // Order + show/hide the sheet panels: show the `shown` sheet (the
    // active one, or the fallback), hide the rest.
    for (rank, sheet) in sheets.iter().enumerate() {
        if let Some(html) = sheet.el.dyn_ref::<HtmlElement>() {
            let _ = html.style().set_property("order", &rank.to_string());
        }
        if sheet.id == shown {
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
                let el = build_tab(document, &sheet.id);
                let _ = strip.append_child(&el);
                el
            }
        };

        // Update only the label's text (the tab also holds a close
        // button child, so we can't blanket-set the tab's text).
        if let Ok(Some(label)) = tab.query_selector(&format!(".{TAB_LABEL}")) {
            label.set_text_content(Some(&sheet.title));
        }
        if let Some(html) = tab.dyn_ref::<HtmlElement>() {
            let _ = html.style().set_property("order", &rank.to_string());
        }
        let _ = tab
            .class_list()
            .toggle_with_force(ACTIVE, sheet.id == active);
    }
}

/// Build one tab button: a label span plus a `×` close button. The
/// view styles both via [`TAB_LABEL`] / [`CLOSE`]; the close button
/// reveals on tab hover (CSS) and dispatches `close` on click (the
/// delegated handler distinguishes it from a tab activation).
fn build_tab(document: &Document, sheet: &str) -> Element {
    let tab = document.create_element("button").expect("create tab");
    let _ = tab.set_attribute("class", TAB);
    let _ = tab.set_attribute("type", "button");
    let _ = tab.set_attribute("data-sheet", sheet);

    let label = document.create_element("span").expect("create label");
    let _ = label.set_attribute("class", TAB_LABEL);
    let _ = tab.append_child(&label);

    let close = document.create_element("span").expect("create close");
    let _ = close.set_attribute("class", CLOSE);
    let _ = close.set_attribute("part", "close");
    let _ = close.set_attribute("aria-label", "Close sheet");
    close.set_text_content(Some("×"));
    let _ = tab.append_child(&close);

    tab
}

/// Minimal CSS-attribute-value escape for the `data-sheet` selector
/// (entity ids contain `:` / `/`). Escapes `"` and `\`.
fn css_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Delegated click on a projected tab. A click on the close button
/// closes the sheet (`close` event); a click anywhere else on the
/// tab activates it (`activate` event).
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

        // Close button: don't activate — retract the sheet. Tell the
        // command which sheet to fall back to (the neighbour by
        // order) so closing the active tab reveals an adjacent one
        // rather than leaving the workspace blank.
        if node.closest(&format!(".{CLOSE}")).ok().flatten().is_some() {
            event.stop_propagation();
            let next = neighbour(&host, &sheet);

            // Optimistic: if the closed sheet was active, move `active`
            // to the neighbour *before* the data round-trip retracts
            // the sheet. Otherwise `active` keeps pointing at a sheet
            // that is about to vanish, and once its `<tonk-sheet>` panel
            // is gone the panel CSS (`:has(tonk-sheet:not([hidden]))`)
            // finds no visible panel and collapses the layout. Pointing
            // `active` at the surviving neighbour now keeps a panel
            // shown throughout. The command (fired below) persists the
            // same `active` via the reassign rule, idempotently.
            if host.get_attribute("active").as_deref() == Some(sheet.as_str())
                && let Some(next) = next.as_deref()
            {
                let _ = host.set_attribute("active", next);
            }

            dispatch_close(&host, &sheet, next.as_deref());
            return;
        }

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

/// The sheet adjacent to `sheet` in `order` order: the next one if
/// any, else the previous, else `None` (it was the only sheet).
/// Used to pick what becomes active when the active sheet closes.
fn neighbour(this: &HtmlElement, sheet: &str) -> Option<String> {
    let mut sheets = collect_sheets(this);
    sheets.sort_by(|a, b| a.order.cmp(&b.order));
    let idx = sheets.iter().position(|s| s.id == sheet)?;
    sheets
        .get(idx + 1)
        .or_else(|| idx.checked_sub(1).and_then(|prev| sheets.get(prev)))
        .map(|s| s.id.clone())
}

/// Dispatch `close` with `detail = { closed, next }`, bubbling +
/// composed. The closed sheet is carried as `closed` (NOT `sheet`):
/// the `activate` event uses `sheet`, and the workspace's
/// `activate-sheet` command reads `dom.event.detail/sheet`, so a close
/// carrying `sheet` would also match that command and select the
/// closed tab. `next` is the neighbour to activate after the close, or
/// absent when the closed sheet was the only one.
fn dispatch_close(host: &HtmlElement, closed: &str, next: Option<&str>) {
    let detail = js_sys::Object::new();
    let _ = js_sys::Reflect::set(
        &detail,
        &JsValue::from_str("closed"),
        &JsValue::from_str(closed),
    );
    if let Some(next) = next {
        let _ = js_sys::Reflect::set(
            &detail,
            &JsValue::from_str("next"),
            &JsValue::from_str(next),
        );
    }
    let init = CustomEventInit::new();
    init.set_detail(&detail);
    init.set_bubbles(true);
    init.set_composed(true);
    if let Ok(event) = CustomEvent::new_with_event_init_dict("close", &init) {
        let _ = host.dispatch_event(&event);
    }
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

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    /// Build a `<tonk-sheet-binder active=…>` with `<tonk-sheet>`
    /// children for each `(id, order, title)`, mounted in the body so
    /// the binder's `connectedCallback` projects synchronously.
    /// Returns the binder element.
    fn mount_binder(active: &str, sheets: &[(&str, &str, &str)]) -> Element {
        register();
        let document = window().unwrap().document().unwrap();
        let body = document.body().unwrap();

        let binder = document.create_element("tonk-sheet-binder").unwrap();
        binder.set_attribute("active", active).unwrap();
        for (id, order, title) in sheets {
            let sheet = document.create_element("tonk-sheet").unwrap();
            sheet.set_attribute("sheet", id).unwrap();
            sheet.set_attribute("order", order).unwrap();
            sheet.set_attribute("title", title).unwrap();
            binder.append_child(&sheet).unwrap();
        }
        body.append_child(&binder).unwrap();
        binder
    }

    /// Capture the next `event_type` CustomEvent's `detail` field
    /// `key` (a string) off the window. Returns a handle whose
    /// `borrow()` holds the captured value, plus the live listener
    /// (drop it to stop listening).
    fn capture_detail(
        event_type: &str,
        key: &'static str,
    ) -> (Rc<RefCell<Option<String>>>, Closure<dyn FnMut(Event)>) {
        let captured: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        let sink = captured.clone();
        let listener = Closure::wrap(Box::new(move |event: Event| {
            let event = event.dyn_into::<CustomEvent>().unwrap();
            let value = js_sys::Reflect::get(&event.detail(), &JsValue::from_str(key))
                .ok()
                .and_then(|v| v.as_string());
            *sink.borrow_mut() = value;
        }) as Box<dyn FnMut(Event)>);
        window()
            .unwrap()
            .add_event_listener_with_callback(event_type, listener.as_ref().unchecked_ref())
            .unwrap();
        (captured, listener)
    }

    #[dialog_common::test]
    async fn it_projects_a_close_button_on_every_tab() {
        let binder = mount_binder("a", &[("a", "a", "First"), ("b", "b", "Second")]);
        let tabs = binder.query_selector_all(&format!(".{TAB}")).unwrap();
        assert_eq!(tabs.length(), 2);
        let closes = binder.query_selector_all(&format!(".{CLOSE}")).unwrap();
        assert_eq!(closes.length(), 2, "one close button per tab");
        binder.remove();
    }

    #[dialog_common::test]
    async fn it_dispatches_close_with_the_neighbour_as_next() {
        let binder = mount_binder("a", &[("a", "a", "First"), ("b", "b", "Second")]);
        let (closed, _closed_l) = capture_detail("close", "closed");
        let (next, _next_l) = capture_detail("close", "next");

        // Click the close button of the first tab. Its neighbour by
        // order is the second sheet, so `next` should be `b`.
        let close = binder
            .query_selector(&format!(".{TAB}[data-sheet=\"a\"] .{CLOSE}"))
            .unwrap()
            .expect("close button on tab a");
        close.dyn_ref::<HtmlElement>().unwrap().click();

        assert_eq!(closed.borrow().as_deref(), Some("a"));
        assert_eq!(next.borrow().as_deref(), Some("b"));
        binder.remove();
    }

    #[dialog_common::test]
    async fn it_omits_next_when_closing_the_only_sheet() {
        let binder = mount_binder("a", &[("a", "a", "Only")]);
        let (closed, _closed_l) = capture_detail("close", "closed");
        let (next, _next_l) = capture_detail("close", "next");

        let close = binder
            .query_selector(&format!(".{TAB}[data-sheet=\"a\"] .{CLOSE}"))
            .unwrap()
            .expect("close button");
        close.dyn_ref::<HtmlElement>().unwrap().click();

        assert_eq!(closed.borrow().as_deref(), Some("a"));
        assert_eq!(
            next.borrow().as_deref(),
            None,
            "no neighbour to fall back to"
        );
        binder.remove();
    }

    #[dialog_common::test]
    async fn it_optimistically_moves_active_to_the_neighbour_on_close() {
        // Closing the active sheet must repoint `active` at the
        // neighbour immediately, so no frame shows `active` pointing
        // at the vanishing sheet (which collapses the panel layout).
        let binder = mount_binder("a", &[("a", "a", "First"), ("b", "b", "Second")]);
        let close = binder
            .query_selector(&format!(".{TAB}[data-sheet=\"a\"] .{CLOSE}"))
            .unwrap()
            .expect("close button on active tab a");
        close.dyn_ref::<HtmlElement>().unwrap().click();

        assert_eq!(
            binder.get_attribute("active").as_deref(),
            Some("b"),
            "active should move to the neighbour synchronously",
        );
        binder.remove();
    }

    #[dialog_common::test]
    async fn it_leaves_active_untouched_when_closing_a_non_active_sheet() {
        let binder = mount_binder("a", &[("a", "a", "First"), ("b", "b", "Second")]);
        let close = binder
            .query_selector(&format!(".{TAB}[data-sheet=\"b\"] .{CLOSE}"))
            .unwrap()
            .expect("close button on inactive tab b");
        close.dyn_ref::<HtmlElement>().unwrap().click();

        assert_eq!(
            binder.get_attribute("active").as_deref(),
            Some("a"),
            "closing a non-active sheet must not move active",
        );
        binder.remove();
    }

    #[dialog_common::test]
    async fn it_shows_the_first_sheet_when_active_is_dangling() {
        // `active` names a sheet that isn't present (a stale persisted
        // pointer). The binder must still show a panel — the first sheet
        // by order — rather than collapse the layout. It does NOT rewrite
        // `active` (display-only fallback), so it can't steal focus
        // during a transient reconcile gap.
        let binder = mount_binder("id:gone", &[("b", "b", "Second"), ("a", "a", "First")]);

        let first_panel = binder
            .query_selector("tonk-sheet[sheet=\"a\"]")
            .unwrap()
            .expect("first sheet present");
        assert!(
            !first_panel.has_attribute("hidden"),
            "the first sheet's panel must be shown as the fallback",
        );
        let second_panel = binder
            .query_selector("tonk-sheet[sheet=\"b\"]")
            .unwrap()
            .expect("second sheet present");
        assert!(
            second_panel.has_attribute("hidden"),
            "the non-fallback sheet must stay hidden",
        );
        // `active` is left untouched — the data model, not the binder,
        // owns correcting a stale pointer.
        assert_eq!(
            binder.get_attribute("active").as_deref(),
            Some("id:gone"),
            "the binder must not rewrite a dangling active",
        );
        binder.remove();
    }

    #[dialog_common::test]
    async fn it_activates_on_a_tab_body_click_not_close() {
        let binder = mount_binder("a", &[("a", "a", "First"), ("b", "b", "Second")]);
        let (activated, _l) = capture_detail("activate", "sheet");
        let (closed, _cl) = capture_detail("close", "closed");

        // Click the tab label (not the close button): activates, does
        // not close.
        let label = binder
            .query_selector(&format!(".{TAB}[data-sheet=\"b\"] .{TAB_LABEL}"))
            .unwrap()
            .expect("label on tab b");
        label.dyn_ref::<HtmlElement>().unwrap().click();

        assert_eq!(activated.borrow().as_deref(), Some("b"));
        assert_eq!(
            closed.borrow().as_deref(),
            None,
            "tab-body click must not close"
        );
        binder.remove();
    }
}
