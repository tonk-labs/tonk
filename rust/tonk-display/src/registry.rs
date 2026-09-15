//! The element registry: the half that answers
//! `tonk-element-needed`.
//!
//! Two concerns, deliberately not the same one.
//!
//! **Announcing** lives in `assets/element-runtime.js`: it watches the
//! document for custom elements nobody has registered and dispatches
//! `tonk-element-needed` from each. It knows nothing about branches,
//! queries, or how a definition is found.
//!
//! **Answering** lives here: one document-level listener, installed at
//! bootstrap so no page can forget it, which resolves a tag to a
//! definition and registers it. It knows nothing about how the tag
//! came to be needed — a view rendering it, a shadow root announcing
//! by hand, or anything else that dispatches the event.
//!
//! Resolution is by NAME, in two hops, both through the routing
//! context of the element that announced (so a tag needed under
//! `with="other@repo"` resolves against that branch):
//!
//! 1. `id:<tag>`'s `db.name/referent` — the entity the tag means now.
//! 2. that entity's `method` dictionary.
//!
//! Then the methods are rendered into a module
//! ([`crate::element_source`]) and executed, which calls
//! `defineTonkElement` and registers the tag. A tag that resolves to
//! nothing is left alone: the element stays inert, exactly as it was
//! before it announced.

use js_sys::Reflect;
use serde_json::json;
use wasm_bindgen::{JsCast, JsValue, prelude::Closure};
use web_sys::{CustomEvent, Element, window};

use tonk_host::consumer;
use tonk_host::events::ELEMENT_NEEDED;
use tonk_template::resolve::name_query;

use crate::element_source::element_module;

/// The runtime asset, embedded so the guest bundle carries it and no
/// extra fetch stands between a rendered tag and its definition.
const RUNTIME: &str = include_str!("../assets/element-runtime.js");

/// Install the runtime and the listener that answers it, then start
/// watching. Idempotent — the runtime guards its own re-entry and the
/// listener is installed once per document.
pub fn install() {
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    if Reflect::get(&js_sys::global(), &"__tonkElementRegistry".into())
        .is_ok_and(|held| held.is_truthy())
    {
        return;
    }
    let _ = Reflect::set(
        &js_sys::global(),
        &"__tonkElementRegistry".into(),
        &JsValue::TRUE,
    );

    crate::element_source::execute(&document, RUNTIME);

    let listener = Closure::<dyn Fn(CustomEvent)>::new(|event: CustomEvent| {
        let Some(tag) = tag_of(&event) else {
            return;
        };
        let Some(consumer) = event
            .target()
            .and_then(|target| target.dyn_into::<Element>().ok())
        else {
            return;
        };
        wasm_bindgen_futures::spawn_local(async move {
            resolve(&consumer, &tag).await;
        });
    });
    let _ = document
        .add_event_listener_with_callback(ELEMENT_NEEDED, listener.as_ref().unchecked_ref());
    // The listener lives as long as the document; there is nothing to
    // unregister it, so leaking the closure is the correct lifetime.
    listener.forget();

    start_watching();
}

/// Call the runtime's `startTonkElements()`, which sweeps the document
/// once and then observes it.
fn start_watching() {
    let Ok(start) = Reflect::get(&js_sys::global(), &"startTonkElements".into()) else {
        return;
    };
    if let Ok(start) = start.dyn_into::<js_sys::Function>() {
        let _ = start.call0(&JsValue::NULL);
    }
}

/// The `detail.tag` of an announcement, when it carries one.
fn tag_of(event: &CustomEvent) -> Option<String> {
    Reflect::get(&event.detail(), &"tag".into())
        .ok()?
        .as_string()
        .filter(|tag| !tag.is_empty())
}

/// Resolve `tag` against the branch `consumer` sits on and register it.
///
/// Every failure is silent-but-logged on purpose: a tag with no
/// definition is the ordinary case (a typo in a template, an element
/// from another branch), and it leaves the element inert rather than
/// breaking the view that rendered it.
async fn resolve(consumer: &Element, tag: &str) {
    let Some(entity) = named_entity(consumer, tag).await else {
        return;
    };
    let methods = methods_of(consumer, &entity).await;
    if methods.is_empty() {
        return;
    }
    let source = element_module(tag, &methods);
    if let Some(document) = consumer.owner_document() {
        crate::element_source::execute(&document, &source);
    }
}

/// Hop one: `id:<tag>` -> the entity the tag currently names.
async fn named_entity(consumer: &Element, tag: &str) -> Option<String> {
    let body = serde_wasm_bindgen::to_value(&name_query(tag)).ok()?;
    let rows = consumer::query(consumer, &body).await.ok()?;
    first_field(&rows, "entity")
}

/// Hop two: the entity's `method` dictionary, folded to `(key,
/// source)` pairs in key order.
///
/// The wire shape mirrors a view's `show`: the field and its key
/// operand are both bound, because a dictionary entry is a `(key,
/// value)` pair and requesting the field alone leaves every entry
/// keyless.
async fn methods_of(
    consumer: &Element,
    entity: &str,
) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    let body = json!({
        "terms": {
            "this":       entity,
            "method":     { "?": { "name": "method" } },
            "method/key": { "?": { "name": "method/key" } },
        },
        "predicate": {
            "with": {
                "method": {
                    "the": { "domain": "xyz.tonk.element.method", "keyed": "dictionary" },
                    "as": "Text",
                    "cardinality": "one"
                }
            }
        }
    });
    let Ok(body) = serde_wasm_bindgen::to_value(&body) else {
        return out;
    };
    let Ok(rows) = consumer::query(consumer, &body).await else {
        return out;
    };
    let Ok(rows) = rows.dyn_into::<js_sys::Array>() else {
        return out;
    };
    // One flat row per entry, each carrying a one-entry `{key: source}`
    // map; merge them into the whole dictionary.
    for row in rows.iter() {
        let Ok(fields) = Reflect::get(&row, &"fields".into()) else {
            continue;
        };
        let Ok(method) = Reflect::get(&fields, &"method".into()) else {
            continue;
        };
        if method.is_undefined() || method.is_null() {
            continue;
        }
        let method_object: js_sys::Object = method.clone().unchecked_into();
        for key in js_sys::Object::keys(&method_object).iter() {
            let Some(key) = key.as_string() else { continue };
            if let Ok(value) = Reflect::get(&method, &key.clone().into())
                && let Some(source) = value.as_string()
            {
                out.insert(key, source);
            }
        }
    }
    out
}

/// The named field of the first row of a query response.
fn first_field(rows: &JsValue, field: &str) -> Option<String> {
    let rows: js_sys::Array = rows.clone().dyn_into().ok()?;
    let row = rows.get(0);
    let fields = Reflect::get(&row, &"fields".into()).ok()?;
    Reflect::get(&fields, &field.into()).ok()?.as_string()
}
