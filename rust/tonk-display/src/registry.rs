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
//! definition and registers it. It knows nothing about how the tag came
//! to be needed — a view rendering it, a shadow root announcing by
//! hand, or anything else that dispatches the event.
//!
//! Resolution is by NAME, in two hops, and both are SUBSCRIPTIONS
//! rather than one-shot reads, because each hop changes for a different
//! reason and both have to be live:
//!
//! 1. `id:<tag>`'s `db.name/referent` — the entity the tag means now.
//!    This is what fires when a tag is rendered before anything defines
//!    it: the element sits inert, and the definition arriving later
//!    registers it with nothing re-rendered.
//! 2. that entity's `method` dictionary. Re-authoring a tag supersedes
//!    method facts on the SAME entity (the element's identity derives
//!    from its `name`, which does not change), so the name binding
//!    stays put and only this hop moves. It is what carries both a
//!    redefinition and a single added method through to instances
//!    already on the page.
//!
//! Each frame re-reads rather than merging the delta it was handed: a
//! subscription is used as a change notification, and the truth comes
//! from a fresh read. Method dictionaries are small, and it keeps this
//! code from having to agree with the wire's delta shape.
//!
//! A tag that resolves to nothing is left alone — the element stays
//! inert, exactly as before it announced — but its subscriptions stay
//! open, which is what makes "define it later" work.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use js_sys::Reflect;
use serde_json::json;
use wasm_bindgen::{JsCast, JsValue, prelude::Closure};
use web_sys::{CustomEvent, Element, window};

use tonk_host::consumer::{self, Subscription};
use tonk_host::events::ELEMENT_NEEDED;
use tonk_template::resolve::name_query;

use crate::element_source::element_module;

/// The runtime asset, embedded so the guest bundle carries it and no
/// extra fetch stands between a rendered tag and its definition.
const RUNTIME: &str = include_str!("../assets/element-runtime.js");

/// Everything held open for one watched tag.
struct Watch {
    /// The element the subscriptions are dispatched from. Kept in the
    /// document so `with` routing context keeps resolving.
    consumer: Element,
    /// Live subscription on `id:<tag>`'s referent.
    _name: Option<Subscription>,
    /// Live subscription on the current entity's methods. Replaced when
    /// the name comes to mean a different entity.
    methods: Option<Subscription>,
    /// The entity the tag currently names, so a name frame that does
    /// not actually move it does not churn the methods subscription.
    entity: Option<String>,
    /// The last module source applied, so an unchanged frame does not
    /// re-run every instance's `connected`.
    applied: Option<String>,
}

thread_local! {
    /// One watch per tag. Realm-global, like `customElements` itself.
    static WATCHED: RefCell<HashMap<String, Rc<RefCell<Watch>>>> =
        RefCell::new(HashMap::new());
}

/// Install the runtime and the listener that answers it. Idempotent.
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

    let listener = Closure::<dyn Fn(CustomEvent)>::new(|event: CustomEvent| {
        let Some(tag) = tag_of(&event) else {
            return;
        };
        // Claim it synchronously: the announcement is only remembered
        // when someone claims it, so a tag announced with no listener
        // is offered again rather than lost. Claiming means "mine to
        // answer" — the resolution below may still find nothing.
        event.prevent_default();
        let Some(source) = event
            .target()
            .and_then(|target| target.dyn_into::<Element>().ok())
        else {
            return;
        };
        wasm_bindgen_futures::spawn_local(async move {
            watch(&source, &tag).await;
        });
    });
    let _ = document
        .add_event_listener_with_callback(ELEMENT_NEEDED, listener.as_ref().unchecked_ref());
    // The listener lives as long as the document; there is nothing to
    // unregister it, so leaking the closure is the correct lifetime.
    listener.forget();

    // Injected LAST so the listener above is in place before the
    // runtime's first sweep. The runtime starts watching on its own
    // evaluation, which happens on a later task than this insertion —
    // it cannot be started from here.
    crate::element_source::execute(&document, RUNTIME);
}

/// Begin watching `tag`, if not already.
///
/// The consumer element is parked next to whatever announced the tag so
/// it shares that element's `with` ancestry, and stays in the document
/// for as long as the realm cares about the tag. A tag is watched once
/// per realm because `customElements` is realm-global: the first
/// announcement's routing context is the one that governs.
async fn watch(source: &Element, tag: &str) {
    let existing = WATCHED.with(|watched| watched.borrow().contains_key(tag));
    if existing {
        return;
    }
    let Some(document) = source.owner_document() else {
        return;
    };
    let Ok(consumer) = document.create_element("tonk-element-watch") else {
        return;
    };
    let _ = consumer.set_attribute("hidden", "");
    let _ = consumer.set_attribute("data-tag", tag);
    // Alongside the announcing element rather than inside it: an author
    // element's children are its own business, and a sibling shares the
    // same `with` ancestry.
    let parent = source
        .parent_element()
        .or_else(|| document.document_element());
    let Some(parent) = parent else {
        return;
    };
    if parent.append_child(&consumer).is_err() {
        return;
    }

    let watch = Rc::new(RefCell::new(Watch {
        consumer: consumer.clone(),
        _name: None,
        methods: None,
        entity: None,
        applied: None,
    }));
    WATCHED.with(|watched| {
        watched
            .borrow_mut()
            .insert(tag.to_owned(), Rc::clone(&watch))
    });

    install_frame_handlers(&consumer, tag);

    // Subscribe to the NAME first and keep it open even when it
    // resolves to nothing: that open subscription is what turns "this
    // tag has no definition" into "not yet".
    let Ok(body) = serde_wasm_bindgen::to_value(&name_query(tag)) else {
        return;
    };
    match consumer::subscribe_claimed(&consumer, &body, Some(&"name".into())).await {
        Ok(subscription) => watch.borrow_mut()._name = Some(subscription),
        Err(error) => {
            web_sys::console::warn_1(
                &format!("<{tag}>: name subscription failed: {}", error.message).into(),
            );
        }
    }

    // Resolve once now rather than waiting to be told. A host may or
    // may not open a subscription with a frame carrying current state;
    // depending on that would make first registration a race against a
    // detail of the transport. From here on the subscription carries
    // changes, which is all it is needed for.
    refresh(tag, "name").await;
}

/// Attach the `reset` / `update` / `error` methods the host calls on a
/// consumer. Every frame means the same thing here — something under
/// this tag changed — so all three route to one re-read.
fn install_frame_handlers(consumer: &Element, tag: &str) {
    for method in ["reset", "update"] {
        let tag = tag.to_owned();
        let handler = Closure::<dyn Fn(JsValue, JsValue)>::new(move |_payload, opts: JsValue| {
            let stream = Reflect::get(&opts, &"tag".into())
                .ok()
                .and_then(|value| value.as_string())
                .unwrap_or_default();
            let tag = tag.clone();
            wasm_bindgen_futures::spawn_local(async move {
                refresh(&tag, &stream).await;
            });
        });
        let _ = Reflect::set(consumer, &method.into(), handler.as_ref().unchecked_ref());
        handler.forget();
    }
    // An errored stream leaves the element as it is; logging beats
    // failing the view that rendered it.
    let tag_for_error = tag.to_owned();
    let on_error = Closure::<dyn Fn(JsValue, JsValue)>::new(move |payload: JsValue, _opts| {
        web_sys::console::warn_2(
            &format!("<{tag_for_error}>: subscription error").into(),
            &payload,
        );
    });
    let _ = Reflect::set(consumer, &"error".into(), on_error.as_ref().unchecked_ref());
    on_error.forget();
}

/// Re-read after a frame on `stream` and apply whatever is current.
async fn refresh(tag: &str, stream: &str) {
    let Some(watch) = WATCHED.with(|watched| watched.borrow().get(tag).map(Rc::clone)) else {
        return;
    };
    let consumer = watch.borrow().consumer.clone();

    if stream == "name" {
        let entity = named_entity(&consumer, tag).await;
        let unchanged = watch.borrow().entity == entity;
        if unchanged {
            return;
        }
        watch.borrow_mut().entity = entity.clone();
        // Drop the old stream before opening the new one: the tag now
        // means something else, and frames from the old entity would
        // otherwise keep re-registering it.
        watch.borrow_mut().methods = None;
        watch.borrow_mut().applied = None;
        let Some(entity) = entity else {
            return;
        };
        if let Some(body) = method_query(&entity) {
            match consumer::subscribe_claimed(&consumer, &body, Some(&"methods".into())).await {
                Ok(subscription) => watch.borrow_mut().methods = Some(subscription),
                Err(error) => {
                    web_sys::console::warn_1(
                        &format!("<{tag}>: method subscription failed: {}", error.message).into(),
                    );
                }
            }
        }
        // Same reason as the name hop: read the methods now rather than
        // waiting for the subscription to volunteer them.
        Box::pin(refresh(tag, "methods")).await;
        return;
    }

    let Some(entity) = watch.borrow().entity.clone() else {
        return;
    };
    let methods = methods_of(&consumer, &entity).await;
    if methods.is_empty() {
        return;
    }
    let source = element_module(tag, &methods);
    let already = watch.borrow().applied.as_deref() == Some(source.as_str());
    if already {
        return;
    }
    watch.borrow_mut().applied = Some(source.clone());
    if let Some(document) = consumer.owner_document() {
        // Keyed by tag, not by content hash: a definition reverted to
        // one already seen must still take effect.
        crate::element_source::execute_keyed(&document, tag, &source);
    }
}

/// The `detail.tag` of an announcement, when it carries one.
fn tag_of(event: &CustomEvent) -> Option<String> {
    Reflect::get(&event.detail(), &"tag".into())
        .ok()?
        .as_string()
        .filter(|tag| !tag.is_empty())
}

/// Hop one: `id:<tag>` -> the entity the tag currently names.
async fn named_entity(consumer: &Element, tag: &str) -> Option<String> {
    let body = serde_wasm_bindgen::to_value(&name_query(tag)).ok()?;
    let rows = consumer::query(consumer, &body).await.ok()?;
    first_field(&rows, "entity")
}

/// The wire query for one entity's `method` dictionary.
///
/// A keyed collection binds two terms — the field and its key operand —
/// because an entry is a `(key, value)` pair; requesting the field alone
/// leaves every entry keyless. Built through `Query` because every other
/// wire query in the system is, and the two do not serialize alike.
fn method_query(entity: &str) -> Option<JsValue> {
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
    let query = serde_json::from_value::<tonk_schema::query::Query>(body).ok()?;
    serde_wasm_bindgen::to_value(&query).ok()
}

/// Hop two: the entity's methods, folded to `(key, source)` in key
/// order.
async fn methods_of(consumer: &Element, entity: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(body) = method_query(entity) else {
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
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    use web_sys::Document;

    wasm_bindgen_test_configure!(run_in_browser);

    /// A mutable stand-in for the branch: the name bindings and method
    /// dictionaries the fake host answers from. Tests mutate it and
    /// then push a frame, which is exactly the shape of a real edit.
    #[derive(Default)]
    struct Branch {
        names: HashMap<String, String>,
        methods: HashMap<String, BTreeMap<String, String>>,
    }

    thread_local! {
        static BRANCH: RefCell<Branch> = RefCell::new(Branch::default());
        static HOST_INSTALLED: RefCell<bool> = const { RefCell::new(false) };
    }

    fn document() -> Document {
        window().expect("window").document().expect("document")
    }

    fn define(tag: &str, entity: &str, methods: &[(&str, &str)]) {
        BRANCH.with(|branch| {
            let mut branch = branch.borrow_mut();
            branch.names.insert(tag.to_owned(), entity.to_owned());
            branch.methods.insert(
                entity.to_owned(),
                methods
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                    .collect(),
            );
        });
    }

    /// Push a frame on one stream of one tag's watch, the way the host
    /// does when the branch moves under a live subscription.
    fn notify(tag: &str, stream: &str) {
        let selector = format!("tonk-element-watch[data-tag=\"{tag}\"]");
        let Ok(Some(watch)) = document().query_selector(&selector) else {
            panic!("no watch element for <{tag}>; was it ever announced?");
        };
        let reset = Reflect::get(&watch, &"reset".into()).expect("reset present");
        let reset: js_sys::Function = reset.dyn_into().expect("reset is a function");
        let opts = js_sys::Object::new();
        let _ = Reflect::set(&opts, &"tag".into(), &stream.into());
        reset
            .call2(&watch, &JsValue::NULL, &opts)
            .expect("call reset");
    }

    /// Claim `tonk-query` and `tonk-subscribe` and answer from
    /// [`BRANCH`]. Installed once per realm.
    ///
    /// Stands down when another host already claimed the event: this
    /// listener sits on `document`, so every consumer event in the realm
    /// bubbles through it, including ones a test's own element-level
    /// stub has answered.
    fn install_fake_host() {
        if HOST_INSTALLED.with(|installed| *installed.borrow()) {
            return;
        }
        HOST_INSTALLED.with(|installed| *installed.borrow_mut() = true);

        let on_query = Closure::<dyn Fn(CustomEvent)>::new(move |event: CustomEvent| {
            if event.default_prevented() {
                return;
            }
            let detail = event.detail();
            let Ok(query) = Reflect::get(&detail, &"query".into()) else {
                return;
            };
            // Read the body back through serde, not `JSON.stringify`:
            // `serde_wasm_bindgen` renders a Rust map as a JS `Map`,
            // whose contents stringify as `{}`.
            let body = serde_wasm_bindgen::from_value::<serde_json::Value>(query)
                .map(|value| value.to_string())
                .unwrap_or_default();
            let rows = answer(&body);
            let promise = js_sys::Promise::resolve(&rows);
            let _ = Reflect::set(&detail, &"result".into(), &promise);
            event.prevent_default();
        });
        let _ = document().add_event_listener_with_callback(
            tonk_host::events::QUERY,
            on_query.as_ref().unchecked_ref(),
        );
        on_query.forget();

        let on_subscribe = Closure::<dyn Fn(CustomEvent)>::new(move |event: CustomEvent| {
            if event.default_prevented() {
                return;
            }
            // A subscription handle with a no-op cancel is all the
            // consumer contract requires; frames are pushed by `notify`.
            let detail = event.detail();
            let subscription = js_sys::Object::new();
            let _ = Reflect::set(
                &subscription,
                &"cancel".into(),
                &js_sys::Function::new_no_args(""),
            );
            let _ = Reflect::set(&detail, &"subscription".into(), &subscription);
            event.prevent_default();
        });
        let _ = document().add_event_listener_with_callback(
            tonk_host::events::SUBSCRIBE,
            on_subscribe.as_ref().unchecked_ref(),
        );
        on_subscribe.forget();
    }

    /// The rows the fake branch returns for one query body.
    fn answer(body: &str) -> js_sys::Array {
        let rows = js_sys::Array::new();
        if body.contains("db.name/referent") {
            let entity = BRANCH.with(|branch| {
                branch
                    .borrow()
                    .names
                    .iter()
                    .find(|(tag, _)| body.contains(&format!("id:{tag}")))
                    .map(|(_, entity)| entity.clone())
            });
            if let Some(entity) = entity {
                let fields = js_sys::Object::new();
                let _ = Reflect::set(&fields, &"entity".into(), &entity.into());
                let row = js_sys::Object::new();
                let _ = Reflect::set(&row, &"fields".into(), &fields);
                rows.push(&row);
            }
            return rows;
        }
        if body.contains("xyz.tonk.element.method") {
            let found = BRANCH.with(|branch| {
                branch
                    .borrow()
                    .methods
                    .iter()
                    .find(|(entity, _)| body.contains(entity.as_str()))
                    .map(|(entity, methods)| (entity.clone(), methods.clone()))
            });
            if let Some((entity, methods)) = found {
                // One flat row per entry, as the wire delivers a keyed
                // collection.
                for (key, source) in methods {
                    let map = js_sys::Object::new();
                    let _ = Reflect::set(&map, &key.into(), &source.into());
                    let fields = js_sys::Object::new();
                    let _ = Reflect::set(&fields, &"method".into(), &map);
                    let row = js_sys::Object::new();
                    let _ = Reflect::set(&row, &"this".into(), &entity.clone().into());
                    let _ = Reflect::set(&row, &"fields".into(), &fields);
                    rows.push(&row);
                }
            }
        }
        rows
    }

    async fn settle_until(done: impl Fn() -> bool) {
        for _ in 0..400 {
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

    /// Render `tag` and wait for the registry to have looked it up.
    async fn render(tag: &str) -> Element {
        install_fake_host();
        install();
        let host = document().create_element(tag).expect("create element");
        document()
            .body()
            .expect("body")
            .append_child(&host)
            .expect("attach");
        // The watch element appearing is the registry's own signal that
        // it has taken the tag on.
        let selector = format!("tonk-element-watch[data-tag=\"{tag}\"]");
        settle_until(|| {
            document()
                .query_selector(&selector)
                .ok()
                .flatten()
                .is_some()
        })
        .await;
        host
    }

    fn defined(tag: &str) -> bool {
        !window()
            .expect("window")
            .custom_elements()
            .get(tag)
            .is_undefined()
    }

    /// The happy path: a tag already defined on the branch when it
    /// renders.
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn it_registers_an_element_already_defined() {
        define(
            "probe-ready",
            "did:key:zReady",
            &[("connected", "(self) => { self.textContent = 'ready'; }")],
        );
        let host = render("probe-ready").await;
        settle_until(|| host.text_content().as_deref() == Some("ready")).await;
        assert_eq!(host.text_content().as_deref(), Some("ready"));
        assert!(defined("probe-ready"));
    }

    /// Rendered with NO definition, defined afterwards, picked up with
    /// nothing re-rendered. The element sits inert in the meantime.
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn it_picks_up_a_definition_that_arrives_later() {
        let host = render("probe-late").await;
        assert!(
            !defined("probe-late"),
            "nothing defines it yet, so it must stay inert",
        );
        assert_eq!(host.text_content().as_deref(), Some(""));

        // The definition lands on the branch, and the open NAME
        // subscription is what carries it.
        define(
            "probe-late",
            "did:key:zLate",
            &[("connected", "(self) => { self.textContent = 'arrived'; }")],
        );
        notify("probe-late", "name");

        settle_until(|| host.text_content().as_deref() == Some("arrived")).await;
        assert_eq!(
            host.text_content().as_deref(),
            Some("arrived"),
            "the element already on the page should upgrade in place",
        );
    }

    /// Re-authoring the tag replaces the implementation for instances
    /// already on the page — `customElements.define` is never called a
    /// second time, because the wrapper dispatches through a table.
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn it_substitutes_a_redefined_implementation() {
        define(
            "probe-swap",
            "did:key:zSwap",
            &[("connected", "(self) => { self.textContent = 'v1'; }")],
        );
        let host = render("probe-swap").await;
        settle_until(|| host.text_content().as_deref() == Some("v1")).await;
        let constructor = window()
            .expect("window")
            .custom_elements()
            .get("probe-swap");

        // Same tag, same entity (its identity derives from `name`,
        // which has not changed) — only the method facts move.
        define(
            "probe-swap",
            "did:key:zSwap",
            &[("connected", "(self) => { self.textContent = 'v2'; }")],
        );
        notify("probe-swap", "methods");

        settle_until(|| host.text_content().as_deref() == Some("v2")).await;
        assert_eq!(
            host.text_content().as_deref(),
            Some("v2"),
            "the live instance should be running the new implementation",
        );
        assert_eq!(
            window()
                .expect("window")
                .custom_elements()
                .get("probe-swap"),
            constructor,
            "the tag must not have been re-registered",
        );
    }

    /// Reverting to a definition already seen must still apply —
    /// the regression the content-hash de-duplication would cause.
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn it_applies_a_definition_reverted_to_an_earlier_one() {
        define(
            "probe-revert",
            "did:key:zRevert",
            &[("connected", "(self) => { self.textContent = 'first'; }")],
        );
        let host = render("probe-revert").await;
        settle_until(|| host.text_content().as_deref() == Some("first")).await;

        define(
            "probe-revert",
            "did:key:zRevert",
            &[("connected", "(self) => { self.textContent = 'second'; }")],
        );
        notify("probe-revert", "methods");
        settle_until(|| host.text_content().as_deref() == Some("second")).await;

        // Back to the exact earlier source.
        define(
            "probe-revert",
            "did:key:zRevert",
            &[("connected", "(self) => { self.textContent = 'first'; }")],
        );
        notify("probe-revert", "methods");
        settle_until(|| host.text_content().as_deref() == Some("first")).await;
        assert_eq!(
            host.text_content().as_deref(),
            Some("first"),
            "a reverted definition must take effect, not be skipped as seen",
        );
    }

    /// Adding a method to an element already registered: the existing
    /// implementation gains it, on instances already mounted.
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn it_adds_a_method_to_a_registered_element() {
        define(
            "probe-grow",
            "did:key:zGrow",
            &[("connected", "(self) => { self.textContent = 'grown'; }")],
        );
        let host = render("probe-grow").await;
        settle_until(|| host.text_content().as_deref() == Some("grown")).await;
        assert!(
            Reflect::get(&host, &"bump".into())
                .expect("property lookup")
                .is_undefined(),
            "no bump method yet",
        );

        define(
            "probe-grow",
            "did:key:zGrow",
            &[
                ("connected", "(self) => { self.textContent = 'grown'; }"),
                ("bump", "(self) => 'bumped'"),
            ],
        );
        notify("probe-grow", "methods");

        settle_until(|| {
            Reflect::get(&host, &"bump".into())
                .map(|value| !value.is_undefined())
                .unwrap_or(false)
        })
        .await;
        let bump = Reflect::get(&host, &"bump".into()).expect("bump present");
        let bump: js_sys::Function = bump.dyn_into().expect("bump is a function");
        assert_eq!(
            bump.call0(&host).expect("call bump").as_string().as_deref(),
            Some("bumped"),
            "the instance already on the page should have gained the method",
        );
    }

    /// Repointing the NAME at a different entity swaps the definition
    /// too — the other way a tag can come to mean something else, and
    /// the one that has to re-aim the method subscription.
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn it_follows_the_name_to_a_different_entity() {
        define(
            "probe-repoint",
            "did:key:zOld",
            &[("connected", "(self) => { self.textContent = 'old'; }")],
        );
        let host = render("probe-repoint").await;
        settle_until(|| host.text_content().as_deref() == Some("old")).await;

        // A different value entirely, with the tag now naming it.
        define(
            "probe-repoint",
            "did:key:zNew",
            &[("connected", "(self) => { self.textContent = 'new'; }")],
        );
        notify("probe-repoint", "name");

        settle_until(|| host.text_content().as_deref() == Some("new")).await;
        assert_eq!(host.text_content().as_deref(), Some("new"));

        // And the OLD entity's stream must no longer reach it: a frame
        // on the stale subscription would otherwise re-register the
        // definition the tag has moved away from.
        BRANCH.with(|branch| {
            branch.borrow_mut().methods.insert(
                "did:key:zOld".to_owned(),
                [(
                    "connected".to_owned(),
                    "(self) => { self.textContent = 'resurrected'; }".to_owned(),
                )]
                .into_iter()
                .collect(),
            );
        });
        notify("probe-repoint", "methods");
        settle_until(|| host.text_content().as_deref() == Some("resurrected")).await;
        assert_eq!(
            host.text_content().as_deref(),
            Some("new"),
            "a superseded entity must not be able to reclaim the tag",
        );
    }

    /// Many instances of one tag: all of them upgrade, and a
    /// redefinition reaches every one.
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn it_updates_every_instance_of_a_tag() {
        define(
            "probe-many",
            "did:key:zMany",
            &[("connected", "(self) => { self.textContent = 'a'; }")],
        );
        let first = render("probe-many").await;
        let others: Vec<Element> = (0..3)
            .map(|_| {
                let el = document().create_element("probe-many").expect("create");
                document()
                    .body()
                    .expect("body")
                    .append_child(&el)
                    .expect("attach");
                el
            })
            .collect();
        settle_until(|| first.text_content().as_deref() == Some("a")).await;

        define(
            "probe-many",
            "did:key:zMany",
            &[("connected", "(self) => { self.textContent = 'b'; }")],
        );
        notify("probe-many", "methods");
        settle_until(|| first.text_content().as_deref() == Some("b")).await;
        for el in std::iter::once(&first).chain(others.iter()) {
            assert_eq!(
                el.text_content().as_deref(),
                Some("b"),
                "every instance should be running the new implementation",
            );
        }
    }

    /// An instance rendered AFTER registration gets the current
    /// implementation, not the one in force when the tag was first
    /// resolved.
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn it_gives_a_later_instance_the_current_implementation() {
        define(
            "probe-fresh",
            "did:key:zFresh",
            &[("connected", "(self) => { self.textContent = 'one'; }")],
        );
        let first = render("probe-fresh").await;
        settle_until(|| first.text_content().as_deref() == Some("one")).await;

        define(
            "probe-fresh",
            "did:key:zFresh",
            &[("connected", "(self) => { self.textContent = 'two'; }")],
        );
        notify("probe-fresh", "methods");
        settle_until(|| first.text_content().as_deref() == Some("two")).await;

        let later = document().create_element("probe-fresh").expect("create");
        document()
            .body()
            .expect("body")
            .append_child(&later)
            .expect("attach");
        assert_eq!(
            later.text_content().as_deref(),
            Some("two"),
            "a newly rendered instance should use the current methods",
        );
    }

    /// A frame that changes nothing must not re-run `connected` on live
    /// instances — a subscription can re-deliver, and re-running a hook
    /// that has already run is a visible side effect, not a no-op.
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn it_ignores_a_frame_that_changes_nothing() {
        define(
            "probe-idem",
            "did:key:zIdem",
            &[(
                "connected",
                "(self) => { globalThis.__idemRuns = (globalThis.__idemRuns || 0) + 1; }",
            )],
        );
        let _host = render("probe-idem").await;
        let runs = || {
            Reflect::get(&js_sys::global(), &"__idemRuns".into())
                .ok()
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0)
        };
        settle_until(|| runs() >= 1.0).await;
        assert_eq!(runs(), 1.0);

        // Same methods, new frame.
        notify("probe-idem", "methods");
        settle_until(|| runs() > 1.0).await;
        assert_eq!(
            runs(),
            1.0,
            "an unchanged definition must not re-run the hook",
        );
    }

    /// The swap hooks fire on a real branch-driven replacement, not
    /// just when `defineTonkElement` is called by hand: the outgoing
    /// definition tears down, the incoming one takes over.
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn it_runs_the_swap_hooks_on_a_branch_edit() {
        define(
            "probe-hooks",
            "did:key:zHooks",
            &[
                (
                    "connected",
                    "(self) => { self.dataset.count = '3'; self.textContent = 'v1'; }",
                ),
                (
                    "released",
                    "(self) => { globalThis.__hookLog = (globalThis.__hookLog || []).concat('released'); \
                     self.dataset.carried = self.dataset.count; }",
                ),
            ],
        );
        let host = render("probe-hooks").await;
        settle_until(|| host.text_content().as_deref() == Some("v1")).await;

        define(
            "probe-hooks",
            "did:key:zHooks",
            &[
                ("connected", "(self) => { self.textContent = 'fresh'; }"),
                (
                    "swapped",
                    "(self) => { globalThis.__hookLog = (globalThis.__hookLog || []).concat('swapped'); \
                     self.textContent = `kept ${self.dataset.carried}`; }",
                ),
            ],
        );
        notify("probe-hooks", "methods");

        settle_until(|| host.text_content().as_deref() == Some("kept 3")).await;
        assert_eq!(
            host.text_content().as_deref(),
            Some("kept 3"),
            "the incoming definition should have taken over the live instance",
        );
        let log = Reflect::get(&js_sys::global(), &"__hookLog".into())
            .ok()
            .and_then(|v| v.dyn_into::<js_sys::Array>().ok())
            .map(|a| a.iter().filter_map(|v| v.as_string()).collect::<Vec<_>>())
            .unwrap_or_default();
        assert_eq!(
            log,
            vec!["released", "swapped"],
            "the outgoing definition must tear down before the incoming takes over",
        );
    }

    /// A tag that resolves to nothing stays inert and is not
    /// registered — but it IS looked up, and its watch stays open.
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn it_leaves_an_unresolvable_tag_inert() {
        let host = render("probe-unknown").await;
        assert!(
            !defined("probe-unknown"),
            "a tag that resolves to nothing must not be registered",
        );
        assert_eq!(host.text_content().as_deref(), Some(""));
        assert!(
            document()
                .query_selector("tonk-element-watch[data-tag=\"probe-unknown\"]")
                .ok()
                .flatten()
                .is_some(),
            "the watch must stay open so a later definition can arrive",
        );
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
