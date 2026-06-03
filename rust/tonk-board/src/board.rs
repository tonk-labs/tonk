//! `<tonk-board source="…">` — the board wrapper element.
//!
//! Two jobs on mount:
//!
//! 1. **Resolve the board name** to an entity URI by dispatching
//!    `tonk-query` against the branch's `Name` index — same host
//!    event the other consumer elements use. The route only knows
//!    the board's bookmark name from the URL; the element does
//!    the lookup. The board schema, view templates, and `demo`
//!    board it resolves against are seeded into the branch from the
//!    standard library at repository creation, not by this element.
//!
//! 2. **Mount a `<tonk-display>`** against the resolved entity
//!    with `model="board-view"` (no `view`, so the built-in view
//!    for that model is used). The view template chain drives
//!    strip / column / tile rendering.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use serde_json::json;
use tonk_host::consumer as host_consumer;
use tonk_schema::conclusion::Conclusion;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Document, Element, HtmlElement, window};

/// Outer per-element struct.
#[derive(Default)]
pub(crate) struct TonkBoard {
    /// Captures the most recently resolved board entity URI so the
    /// element can avoid re-resolving when the same name lands
    /// twice (e.g. attribute set then re-set during Leptos
    /// reactivity).
    last_resolved: Rc<RefCell<Option<(String, String)>>>,
    /// Name currently being resolved. Custom elements fire
    /// `attributeChangedCallback` for each observed attribute at
    /// upgrade time AND `connectedCallback` immediately after, so
    /// without this flag the same `source` triggers two parallel
    /// resolves (and two evaluates + two queries). Cleared by the
    /// spawned task when it finishes.
    in_flight: Rc<RefCell<Option<String>>>,
}

impl CustomElement for TonkBoard {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["source"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        start_resolve(this, &self.last_resolved, &self.in_flight);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {}

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        _name: String,
        old: Option<String>,
        new: Option<String>,
    ) {
        // No-op when the attribute was rewritten to the same
        // value. Leptos's reactive bindings can fire setAttribute
        // even when the rendered string didn't change, and the
        // browser still raises `attributeChangedCallback` either
        // way; without this guard a no-op write would re-run the
        // resolve through `start_resolve`'s memo path.
        if old == new {
            return;
        }
        start_resolve(this, &self.last_resolved, &self.in_flight);
    }
}

/// Kick off the name→entity resolve; on success rebuild the host
/// with a fresh `<tonk-display>`. Memoizes the most-recent
/// resolution so an attribute change that lands the same name
/// again is a no-op, and tracks an in-flight resolve so two
/// lifecycle callbacks (attribute + connected) firing back-to-
/// back at upgrade time don't both kick off the work.
fn start_resolve(
    this: &HtmlElement,
    last_resolved: &Rc<RefCell<Option<(String, String)>>>,
    in_flight: &Rc<RefCell<Option<String>>>,
) {
    let host: Element = this.clone().into();
    let Some(name) = host.get_attribute("source").filter(|s| !s.is_empty()) else {
        return;
    };

    // Memo check: same name already resolved, reuse the URI.
    if let Some((cached_name, cached_uri)) = last_resolved.borrow().clone()
        && cached_name == name
    {
        mount_display(&host, &cached_uri);
        return;
    }

    // Dedupe: an earlier callback in this same tick may have
    // already spawned the resolve for this name.
    if in_flight.borrow().as_deref() == Some(name.as_str()) {
        return;
    }
    *in_flight.borrow_mut() = Some(name.clone());

    let last_resolved = last_resolved.clone();
    let in_flight = in_flight.clone();
    let host_for_async = host.clone();
    let name_for_async = name.clone();
    spawn_local(async move {
        let uri = if looks_like_uri(&name_for_async) {
            name_for_async.clone()
        } else {
            // The board schema, view templates, and demo data are
            // seeded once at repository creation (see the shell's
            // `init`), so the board just resolves its name to an
            // entity URI here.
            match resolve_name(&host_for_async, &name_for_async).await {
                Ok(Some(u)) => u,
                Ok(None) => {
                    show_not_found(&host_for_async, &name_for_async);
                    if in_flight.borrow().as_deref() == Some(name_for_async.as_str()) {
                        *in_flight.borrow_mut() = None;
                    }
                    return;
                }
                Err(msg) => {
                    web_sys::console::warn_1(
                        &format!("tonk-board: resolve `{name_for_async}` failed: {msg}").into(),
                    );
                    if in_flight.borrow().as_deref() == Some(name_for_async.as_str()) {
                        *in_flight.borrow_mut() = None;
                    }
                    return;
                }
            }
        };
        *last_resolved.borrow_mut() = Some((name_for_async.clone(), uri.clone()));
        if in_flight.borrow().as_deref() == Some(name_for_async.as_str()) {
            *in_flight.borrow_mut() = None;
        }
        mount_display(&host_for_async, &uri);
    });
}

fn looks_like_uri(s: &str) -> bool {
    s.contains(':')
}

/// Dispatch `tonk-query` to find the board whose
/// `xyz.tonk.board/name` field equals `name`. Returns the
/// board's entity URI or `None` if no board matched.
async fn resolve_name(host: &Element, name: &str) -> Result<Option<String>, String> {
    let body = json!({
        "terms": {
            "this": { "?": { "name": "this" } },
            "name": name,
        },
        "predicate": {
            "with": {
                "name": {
                    "the": "xyz.tonk.board/name",
                    "as": "Text",
                    "cardinality": "one",
                }
            }
        }
    });
    let body_js = serde_wasm_bindgen::to_value(&body).map_err(|e| format!("body: {e}"))?;
    let result = host_consumer::query(host, &body_js)
        .await
        .map_err(|e| e.message)?;
    let conclusions: Vec<Conclusion> =
        serde_wasm_bindgen::from_value(result).map_err(|e| format!("parse result: {e}"))?;
    Ok(conclusions.into_iter().next().map(|c| c.this))
}

/// Replace the host's contents with a `<tonk-display>` pointed at
/// `entity`.
fn mount_display(host: &Element, entity: &str) {
    host.set_inner_html("");
    let Some(document) = document() else { return };
    let Ok(display) = document.create_element("tonk-display") else {
        return;
    };
    let _ = display.set_attribute("entity", entity);
    // No `view` attribute: with `model="board-view"` the display
    // resolves the built-in view whose `model` is `board-view`.
    let _ = display.set_attribute("model", "board-view");
    let _ = host.append_child(&display);
}

/// Replace the host's contents with a "not found" section.
fn show_not_found(host: &Element, name: &str) {
    host.set_inner_html("");
    let Some(document) = document() else { return };
    let Ok(section) = document.create_element("section") else {
        return;
    };
    let _ = section.set_attribute("class", "not-found");
    section.set_text_content(Some(&format!("No board is named {name}")));
    let _ = host.append_child(&section);
}

fn document() -> Option<Document> {
    window()?.document()
}

/// Register `<tonk-board>` with the page. Idempotent.
pub(crate) fn register() {
    if already_registered() {
        return;
    }
    TonkBoard::define("tonk-board");
}

fn already_registered() -> bool {
    let Some(win) = window() else {
        return false;
    };
    !win.custom_elements().get("tonk-board").is_undefined()
}
