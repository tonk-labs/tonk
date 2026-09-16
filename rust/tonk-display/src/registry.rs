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
        // Claim it synchronously: the announcement is only remembered
        // when someone claims it, so a tag announced with no listener
        // is offered again rather than lost. Claiming means "mine to
        // answer" — the resolution below may still find nothing.
        event.prevent_default();
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

    // The runtime starts watching on its own evaluation. It cannot be
    // started from here: a module script evaluates on a later task than
    // the insertion above, so `startTonkElements` does not exist yet —
    // and the listener registered above is in place well before it
    // runs, which is the ordering that matters.
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
    // Through `Query` rather than straight from the JSON: every other
    // wire query in the system is built that way, and the two do not
    // serialize alike.
    let Ok(query) = serde_json::from_value::<tonk_schema::query::Query>(body) else {
        return out;
    };
    let Ok(body) = serde_wasm_bindgen::to_value(&query) else {
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

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen::prelude::Closure;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    use web_sys::Document;

    wasm_bindgen_test_configure!(run_in_browser);

    fn document() -> Document {
        window().expect("window").document().expect("document")
    }

    /// A stand-in for the host: claims `tonk-query` and answers from a
    /// canned branch. Answers the two hops the registry makes —
    /// `db.name/referent` for the tag, then the `method` dictionary for
    /// the entity it named.
    ///
    /// Records every query it saw so the test can assert the registry
    /// asked for the right things, not merely that something worked.
    fn install_fake_host(tag: &str, entity: &str, methods: &[(&str, &str)]) {
        let tag = tag.to_owned();
        let entity = entity.to_owned();
        let methods: Vec<(String, String)> = methods
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();

        let handler = Closure::<dyn Fn(CustomEvent)>::new(move |event: CustomEvent| {
            // Stand down if another host already claimed it. This
            // listener sits on `document`, so every consumer query in
            // the realm bubbles through it — including ones a test's
            // own element-level stub has already answered, whose result
            // must not be overwritten.
            if event.default_prevented() {
                return;
            }
            let detail = event.detail();
            let Ok(query) = Reflect::get(&detail, &"query".into()) else {
                return;
            };
            // Read it back through serde, not `JSON.stringify`:
            // `serde_wasm_bindgen` renders a Rust map as a JS `Map`,
            // whose contents stringify as `{}`.
            let body = serde_wasm_bindgen::from_value::<serde_json::Value>(query)
                .map(|value| value.to_string())
                .unwrap_or_default();
            // Record it for the assertions below.
            let seen = Reflect::get(&js_sys::global(), &"__seenQueries".into())
                .ok()
                .and_then(|v| v.dyn_into::<js_sys::Array>().ok())
                .unwrap_or_else(|| {
                    let fresh = js_sys::Array::new();
                    let _ = Reflect::set(&js_sys::global(), &"__seenQueries".into(), &fresh);
                    fresh
                });
            seen.push(&body.clone().into());

            let rows = js_sys::Array::new();
            if body.contains("db.name/referent") {
                // Only answer for the tag we were given; anything else
                // resolves to nothing, like an unknown name.
                if body.contains(&format!("id:{tag}")) {
                    rows.push(&row(&[("entity", &entity)]));
                }
            } else if body.contains("xyz.tonk.element.method") && body.contains(&entity) {
                // One flat row per entry, as the wire delivers a keyed
                // collection.
                for (key, source) in &methods {
                    let map = js_sys::Object::new();
                    let _ = Reflect::set(&map, &key.into(), &source.into());
                    let fields = js_sys::Object::new();
                    let _ = Reflect::set(&fields, &"method".into(), &map);
                    let conclusion = js_sys::Object::new();
                    let _ = Reflect::set(&conclusion, &"this".into(), &entity.clone().into());
                    let _ = Reflect::set(&conclusion, &"fields".into(), &fields);
                    rows.push(&conclusion);
                }
            }

            let promise = js_sys::Promise::resolve(&rows);
            let _ = Reflect::set(&detail, &"result".into(), &promise);
            // Claiming the event is what tells the consumer a host
            // answered; without it the query errors out.
            event.prevent_default();
        });
        let _ = document().add_event_listener_with_callback(
            tonk_host::events::QUERY,
            handler.as_ref().unchecked_ref(),
        );
        handler.forget();
    }

    /// One query-result row: `{ this, fields: { … } }`.
    fn row(fields: &[(&str, &str)]) -> JsValue {
        let map = js_sys::Object::new();
        for (key, value) in fields {
            let _ = Reflect::set(&map, &(*key).into(), &(*value).into());
        }
        let conclusion = js_sys::Object::new();
        let _ = Reflect::set(&conclusion, &"fields".into(), &map);
        conclusion.into()
    }

    async fn settle_until(done: impl Fn() -> bool) {
        for _ in 0..300 {
            if done() {
                return;
            }
            let promise = js_sys::Promise::new(&mut |resolve, _| {
                let _ = window()
                    .expect("window")
                    .set_timeout_with_callback_and_timeout_and_arguments_0(
                        resolve.unchecked_ref(),
                        0,
                    );
            });
            let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
        }
    }

    fn seen_queries() -> Vec<String> {
        Reflect::get(&js_sys::global(), &"__seenQueries".into())
            .ok()
            .and_then(|v| v.dyn_into::<js_sys::Array>().ok())
            .map(|a| a.iter().filter_map(|v| v.as_string()).collect())
            .unwrap_or_default()
    }

    /// The whole path, end to end: an undefined element renders, the
    /// runtime announces it, this listener resolves the tag by name,
    /// reads its methods, and registers it — and the element already in
    /// the DOM upgrades.
    ///
    /// One test rather than several because `install` is
    /// realm-idempotent by design and every wasm test here shares a
    /// realm.
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn it_registers_an_element_announced_by_the_runtime() {
        install_fake_host(
            "probe-widget",
            "did:key:zProbeEntity",
            &[(
                "connected",
                "(self) => { self.textContent = 'registered'; }",
            )],
        );
        install();

        let host = document()
            .create_element("probe-widget")
            .expect("create probe-widget");
        document()
            .body()
            .expect("body")
            .append_child(&host)
            .expect("attach");

        settle_until(|| host.text_content().as_deref() == Some("registered")).await;
        assert_eq!(
            host.text_content().as_deref(),
            Some("registered"),
            "the announced element should have been resolved and \
             upgraded; queries seen: {:?}",
            seen_queries(),
        );
        assert!(
            !window()
                .expect("window")
                .custom_elements()
                .get("probe-widget")
                .is_undefined(),
            "the tag should be registered",
        );

        // The registry asked for the right things, in the right order:
        // resolve the NAME first, then read that entity's methods.
        let queries = seen_queries();
        let name_hop = queries
            .iter()
            .position(|q| q.contains("db.name/referent") && q.contains("id:probe-widget"))
            .expect("a name query for the tag");
        let method_hop = queries
            .iter()
            .position(|q| q.contains("xyz.tonk.element.method"))
            .expect("a method query");
        assert!(name_hop < method_hop, "name must resolve before methods");
        assert!(
            queries[method_hop].contains("did:key:zProbeEntity"),
            "methods must be read off the entity the NAME resolved to: {}",
            queries[method_hop],
        );
    }

    /// A tag that resolves to no entity leaves the element inert rather
    /// than throwing — the ordinary case for a typo'd tag, and it must
    /// not break the view that rendered it.
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn it_leaves_an_unresolvable_tag_inert() {
        let host = document()
            .create_element("probe-unknown")
            .expect("create probe-unknown");
        document()
            .body()
            .expect("body")
            .append_child(&host)
            .expect("attach");
        settle_until(|| {
            seen_queries()
                .iter()
                .any(|q| q.contains("id:probe-unknown"))
        })
        .await;
        // Assert the lookup HAPPENED before asserting its outcome —
        // otherwise this passes just as well when nothing ever ran.
        assert!(
            seen_queries()
                .iter()
                .any(|q| q.contains("id:probe-unknown")),
            "the unknown tag should still have been looked up: {:?}",
            seen_queries(),
        );
        assert!(
            window()
                .expect("window")
                .custom_elements()
                .get("probe-unknown")
                .is_undefined(),
            "a tag that resolves to nothing must not be registered",
        );
        assert_eq!(host.text_content().as_deref(), Some(""));
    }

    #[wasm_bindgen_test::wasm_bindgen_test]
    fn it_reads_the_tag_off_an_announcement() {
        let detail = js_sys::Object::new();
        let _ = Reflect::set(&detail, &"tag".into(), &"x-y".into());
        let init = web_sys::CustomEventInit::new();
        init.set_detail(&detail);
        let event = CustomEvent::new_with_event_init_dict(ELEMENT_NEEDED, &init).expect("event");
        assert_eq!(tag_of(&event).as_deref(), Some("x-y"));

        let bare = CustomEvent::new(ELEMENT_NEEDED).expect("event");
        assert_eq!(tag_of(&bare), None);
    }
}
