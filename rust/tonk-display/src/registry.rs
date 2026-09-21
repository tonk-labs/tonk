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
//! 2. that entity's `method` dictionary. An edit can move either hop
//!    and the registry cannot tell which in advance: notation that
//!    names the entity supersedes method facts in place, so only this
//!    hop changes, while `tonk element add` derives a new element and
//!    repoints the name, so hop one changes and this one follows.
//!    Watching both is what carries a redefinition, a single added
//!    method, and a wholesale swap alike through to instances already
//!    on the page.
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
use wasm_bindgen::{JsCast, JsValue, prelude::Closure};
use web_sys::{CustomEvent, Element, window};

use tonk_host::consumer::{self, Subscription};
use tonk_host::events::ELEMENT_NEEDED;
use tonk_template::resolve::{ELEMENT_DICTIONARIES, name_query};

use crate::element_source::{ElementDefinition, element_module};

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
    /// Live subscriptions on the current entity's dictionaries, one
    /// per entry of [`ELEMENT_DICTIONARIES`], replaced together when
    /// the name comes to mean a different entity.
    ///
    /// One each rather than one for all, because they are separate
    /// queries: binding two keyed collections in one would join entry
    /// against entry and hand back their cross product, and an element
    /// with methods but no defaults — which is most of them — would
    /// match neither.
    dictionaries: Vec<Subscription>,
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
    // Carry the announcing element's routing context onto the watch
    // explicitly. `resolve_with` reads `with` off the consumer ITSELF,
    // and the host's observer that stamps it onto descendants runs on a
    // later task — so a watch that subscribed on creation would race it
    // and resolve no context at all. Copying it is also more honest:
    // the definition is fetched from the branch the element that needed
    // it was rendering against, not from wherever the watch landed.
    if let Some(context) = source
        .closest("[with]")
        .ok()
        .flatten()
        .and_then(|ancestor| ancestor.get_attribute("with"))
        .filter(|value| !value.is_empty())
    {
        let _ = consumer.set_attribute("with", &context);
    }
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
        dictionaries: Vec::new(),
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
        // Stringified into the message rather than passed as a second
        // argument: a bare object logs as `[object Object]` wherever the
        // console is captured as text, which is every place anyone reads
        // this from.
        let detail = js_sys::JSON::stringify(&payload)
            .ok()
            .and_then(|text| text.as_string())
            .unwrap_or_else(|| format!("{payload:?}"));
        web_sys::console::warn_1(
            &format!("<{tag_for_error}>: subscription error: {detail}").into(),
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
        // Drop the old streams before opening the new ones: the tag now
        // means something else, and frames from the old entity would
        // otherwise keep re-registering it.
        watch.borrow_mut().dictionaries.clear();
        watch.borrow_mut().applied = None;
        let Some(entity) = entity else {
            return;
        };
        for (field, domain) in ELEMENT_DICTIONARIES {
            let Some(body) = dictionary_query(&entity, field, domain) else {
                continue;
            };
            match consumer::subscribe_claimed(&consumer, &body, Some(&(*field).into())).await {
                Ok(subscription) => watch.borrow_mut().dictionaries.push(subscription),
                Err(error) => {
                    web_sys::console::warn_1(
                        &format!("<{tag}>: {field} subscription failed: {}", error.message).into(),
                    );
                }
            }
        }
        // Same reason as the name hop: read now rather than waiting for
        // the subscriptions to volunteer. One re-read covers every
        // dictionary, since the module is rendered from all of them.
        Box::pin(refresh(tag, "method")).await;
        return;
    }

    let Some(entity) = watch.borrow().entity.clone() else {
        return;
    };
    // Any stream re-reads every dictionary: the module is rendered
    // from all of them together, and `applied` below is what keeps the
    // extra reads from costing a re-registration.
    let definition = definition_of(&consumer, &entity).await;
    if definition.methods.is_empty() {
        return;
    }
    let source = element_module(tag, &definition);
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

/// The wire query for one of an entity's dictionaries, serialized for
/// the host bridge.
///
/// The query itself is built in [`tonk_template::resolve`], shared with
/// the CLI listing and with the test that runs it against a real branch
/// — the three have to agree, and every way they can silently disagree
/// reads as "this element declares none of these" rather than as an
/// error.
fn dictionary_query(entity: &str, field: &str, domain: &str) -> Option<JsValue> {
    let query = tonk_template::resolve::element_dictionary_query(entity, field, domain).ok()?;
    serde_wasm_bindgen::to_value(&query).ok()
}

/// Hop two: every dictionary the entity carries, each folded to
/// `(key, value)` in key order.
///
/// A dictionary an element declares none of reads empty, which is the
/// common case for all but `method` and is not an error.
async fn definition_of(consumer: &Element, entity: &str) -> ElementDefinition {
    let mut out = ElementDefinition::default();
    for (field, domain) in ELEMENT_DICTIONARIES {
        let Some(body) = dictionary_query(entity, field, domain) else {
            continue;
        };
        let entries = dictionary_of(consumer, &body, field).await;
        match *field {
            "method" => out.methods = entries,
            "attribute" => out.attributes = entries,
            "getter" => out.getters = entries,
            "setter" => out.setters = entries,
            // Unreachable while the list and this match agree; a new
            // dictionary added to one and not the other reads as
            // "declares none" rather than as a panic in a browser.
            _ => {}
        }
    }
    out
}

/// Run `body` and fold `field` — a keyed dictionary — out of the
/// response.
///
/// A dictionary query answers one flat row per ENTRY, each carrying a
/// one-entry `{key: value}` map under the field, so reading the first
/// row would see one entry and call it the whole map. Merge them.
async fn dictionary_of(
    consumer: &Element,
    body: &JsValue,
    field: &str,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Ok(rows) = consumer::query(consumer, body).await else {
        return out;
    };
    let Ok(rows) = rows.dyn_into::<js_sys::Array>() else {
        return out;
    };
    for row in rows.iter() {
        let Ok(fields) = Reflect::get(&row, &"fields".into()) else {
            continue;
        };
        let Ok(entries) = Reflect::get(&fields, &field.into()) else {
            continue;
        };
        if entries.is_undefined() || entries.is_null() {
            continue;
        }
        let object: js_sys::Object = entries.clone().unchecked_into();
        for key in js_sys::Object::keys(&object).iter() {
            let Some(key) = key.as_string() else { continue };
            if let Ok(value) = Reflect::get(&entries, &key.clone().into())
                && let Some(text) = value.as_string()
            {
                out.insert(key, text);
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

    /// A mutable stand-in for the branch: the name bindings, method
    /// dictionaries and attribute defaults the fake host answers from.
    /// Tests mutate it and then push a frame, which is exactly the
    /// shape of a real edit.
    #[derive(Default)]
    struct Branch {
        names: HashMap<String, String>,
        /// Entity -> field -> entries, mirroring [`ELEMENT_DICTIONARIES`]:
        /// keyed by field name so the fake host answers every dictionary
        /// the registry asks for without a table per kind. A field with
        /// no entry reads empty, which is what "declares none of these"
        /// looks like.
        dictionaries: HashMap<String, HashMap<String, BTreeMap<String, String>>>,
    }

    thread_local! {
        static BRANCH: RefCell<Branch> = RefCell::new(Branch::default());
        static HOST_INSTALLED: RefCell<bool> = const { RefCell::new(false) };
    }

    fn document() -> Document {
        window().expect("window").document().expect("document")
    }

    fn define(tag: &str, entity: &str, methods: &[(&str, &str)]) {
        define_with_defaults(tag, entity, methods, &[]);
    }

    /// [`define`] plus the element's attribute defaults.
    fn define_with_defaults(
        tag: &str,
        entity: &str,
        methods: &[(&str, &str)],
        attributes: &[(&str, &str)],
    ) {
        define_dictionaries(
            tag,
            entity,
            &[("method", methods), ("attribute", attributes)],
        );
    }

    /// [`define`] over any of the element's dictionaries, named the way
    /// [`ELEMENT_DICTIONARIES`] names them.
    fn define_dictionaries(tag: &str, entity: &str, maps: &[(&str, &[(&str, &str)])]) {
        BRANCH.with(|branch| {
            let mut branch = branch.borrow_mut();
            branch.names.insert(tag.to_owned(), entity.to_owned());
            for (field, kv) in maps {
                branch
                    .dictionaries
                    .entry((*field).to_owned())
                    .or_default()
                    .insert(
                        entity.to_owned(),
                        kv.iter()
                            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                            .collect(),
                    );
            }
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
            // Claim only the two shapes this branch knows. A
            // document-level listener sees every consumer query in the
            // realm — `<tonk-display>` boots and subscribes in other
            // tests of this same suite — and answering those would
            // change how they behave. Leaving them unclaimed is the
            // difference between a stand-in and a hijack.
            if !ours(&body) {
                return;
            }
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
            let detail = event.detail();
            let body = Reflect::get(&detail, &"query".into())
                .ok()
                .and_then(|query| serde_wasm_bindgen::from_value::<serde_json::Value>(query).ok())
                .map(|value| value.to_string())
                .unwrap_or_default();
            if !ours(&body) {
                return;
            }
            // A subscription handle with a no-op cancel is all the
            // consumer contract requires; frames are pushed by `notify`.
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

    /// Whether a query body is one of the three this fake branch
    /// serves.
    ///
    /// The attribute domain has to be listed even though most tests
    /// declare no defaults: leaving it out does not mean "answers
    /// none", it means the query is never claimed, and the registry
    /// then waits on a result nobody will produce — which showed up as
    /// every element failing to register at all.
    fn ours(body: &str) -> bool {
        body.contains("db.name/referent")
            || ELEMENT_DICTIONARIES
                .iter()
                .any(|(_, domain)| body.contains(domain))
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
        // The LONGEST matching domain wins. The four are distinct
        // strings today, but one being a prefix of another — which a
        // rename could introduce — would otherwise have the shorter
        // arm answer for the longer one's query, and the registry
        // would read a dictionary of the wrong kind without error.
        let matched = ELEMENT_DICTIONARIES
            .iter()
            .filter(|(_, domain)| body.contains(domain))
            .max_by_key(|(_, domain)| domain.len());
        if let Some((field, _)) = matched {
            let found = BRANCH.with(|branch| {
                let branch = branch.borrow();
                branch
                    .dictionaries
                    .get(*field)?
                    .iter()
                    .find(|(entity, _)| body.contains(entity.as_str()))
                    .map(|(entity, entries)| (entity.clone(), entries.clone()))
            });
            if let Some((entity, entries)) = found {
                // One flat row per entry, as the wire delivers a keyed
                // collection. An empty map yields no rows at all, which
                // is what an element declaring no defaults looks like.
                for (key, value) in entries {
                    let map = js_sys::Object::new();
                    let _ = Reflect::set(&map, &key.into(), &value.into());
                    let fields = js_sys::Object::new();
                    let _ = Reflect::set(&fields, &(*field).into(), &map);
                    let row = js_sys::Object::new();
                    let _ = Reflect::set(&row, &"this".into(), &entity.clone().into());
                    let _ = Reflect::set(&row, &"fields".into(), &fields);
                    rows.push(&row);
                }
            }
        }
        rows
    }

    /// Give anything in flight a bounded chance to happen, for
    /// asserting that it does NOT. Polling a negative with
    /// [`settle_until`] burns its whole budget every time — cheap when
    /// a page is shared, seconds each under one process per test.
    async fn settle_briefly() {
        for _ in 0..20 {
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
        notify("probe-swap", "method");

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
        notify("probe-revert", "method");
        settle_until(|| host.text_content().as_deref() == Some("second")).await;

        // Back to the exact earlier source.
        define(
            "probe-revert",
            "did:key:zRevert",
            &[("connected", "(self) => { self.textContent = 'first'; }")],
        );
        notify("probe-revert", "method");
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
        notify("probe-grow", "method");

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
            branch
                .borrow_mut()
                .dictionaries
                .entry("method".to_owned())
                .or_default()
                .insert(
                    "did:key:zOld".to_owned(),
                    [(
                        "connected".to_owned(),
                        "(self) => { self.textContent = 'resurrected'; }".to_owned(),
                    )]
                    .into_iter()
                    .collect(),
                );
        });
        notify("probe-repoint", "method");
        settle_briefly().await;
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
        notify("probe-many", "method");
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
        notify("probe-fresh", "method");
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
        notify("probe-idem", "method");
        settle_briefly().await;
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
        notify("probe-hooks", "method");

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

    // ── Library elements ──────────────────────────────────────────
    //
    // The elements the libraries define replaced Rust elements that only
    // ever needed the DOM. Their JS is read off the library text here,
    // so what runs under test is what a guest runs, not a restatement.

    const CORE_LIBRARY: &str = include_str!("../../tonk-core/assets/library/core.yaml");
    const PROFILE_LIBRARY: &str = include_str!("../../tonk-core/assets/library/profile.yaml");

    /// The dictionaries `element!: &{tag}` declares in `source`, as the
    /// registry receives them: `(dictionary, [(name, source)])`.
    fn library_element(source: &str, tag: &str) -> Vec<(String, Vec<(String, String)>)> {
        let heading = format!("element!: &{tag}");
        let mut lines = source
            .lines()
            .skip_while(|line| line.trim_end() != heading)
            .skip(1)
            .peekable();
        let mut out: Vec<(String, Vec<(String, String)>)> = Vec::new();
        let mut current: Option<usize> = None;
        while let Some(line) = lines.next() {
            if !line.is_empty() && !line.starts_with(' ') {
                break;
            }
            if line.trim().is_empty() || line.trim_start().starts_with('#') {
                continue;
            }
            if let Some(name) = line
                .strip_prefix("  ")
                .filter(|rest| !rest.starts_with(' '))
                .and_then(|rest| rest.strip_suffix(':'))
            {
                current = if matches!(name, "method" | "getter" | "setter" | "attribute") {
                    out.push((name.to_owned(), Vec::new()));
                    Some(out.len() - 1)
                } else {
                    None
                };
                continue;
            }
            let Some(index) = current else {
                continue;
            };
            let Some(entry) = line
                .strip_prefix("    ")
                .filter(|rest| !rest.starts_with(' '))
            else {
                continue;
            };
            if let Some(name) = entry.strip_suffix(": |") {
                let mut body = String::new();
                while let Some(next) = lines.peek() {
                    if next.trim().is_empty() {
                        body.push('\n');
                        lines.next();
                        continue;
                    }
                    let Some(rest) = next.strip_prefix("      ") else {
                        break;
                    };
                    body.push_str(rest);
                    body.push('\n');
                    lines.next();
                }
                out[index].1.push((name.to_owned(), body));
            } else if let Some((name, value)) = entry.split_once(": ") {
                out[index]
                    .1
                    .push((name.to_owned(), value.trim().trim_matches('"').to_owned()));
            }
        }
        assert!(!out.is_empty(), "no `element!: &{tag}` in the library");
        out
    }

    fn define_from_library(source: &str, tag: &str) {
        let dictionaries = library_element(source, tag);
        let borrowed: Vec<(&str, Vec<(&str, &str)>)> = dictionaries
            .iter()
            .map(|(name, entries)| {
                (
                    name.as_str(),
                    entries
                        .iter()
                        .map(|(key, value)| (key.as_str(), value.as_str()))
                        .collect(),
                )
            })
            .collect();
        let maps: Vec<(&str, &[(&str, &str)])> = borrowed
            .iter()
            .map(|(name, entries)| (*name, entries.as_slice()))
            .collect();
        let entity = format!("did:key:zLibrary{}", tag.replace('-', ""));
        define_dictionaries(tag, &entity, &maps);
    }

    /// `window.tonk`, the host's bridge into this realm, created if the
    /// page has none yet.
    fn host_bridge() -> js_sys::Object {
        let win = window().expect("window");
        match Reflect::get(&win, &"tonk".into())
            .ok()
            .filter(|value| value.is_object())
        {
            Some(bridge) => bridge.into(),
            None => {
                let bridge = js_sys::Object::new();
                let _ = Reflect::set(&win, &"tonk".into(), &bridge);
                bridge
            }
        }
    }

    /// A host function on the bridge that records what it is called with.
    fn record_bridge_calls(name: &str) -> js_sys::Array {
        let calls = js_sys::Array::new();
        let recorded = calls.clone();
        let function = Closure::<dyn FnMut(JsValue)>::new(move |payload| {
            recorded.push(&payload);
        });
        let _ = Reflect::set(
            &host_bridge(),
            &name.into(),
            function.as_ref().unchecked_ref::<js_sys::Function>(),
        );
        function.forget();
        calls
    }

    fn keydown(host: &Element, key: &str) {
        let make = js_sys::Function::new_with_args(
            "key",
            "return new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true });",
        );
        let event = make
            .call1(&JsValue::NULL, &key.into())
            .expect("keyboard event");
        host.dispatch_event(event.unchecked_ref())
            .expect("dispatch");
    }

    /// A bubbling event, as the events a page raises are.
    fn fire(target: &web_sys::EventTarget, name: &str) {
        let make = js_sys::Function::new_with_args(
            "name",
            "return new Event(name, { bubbles: true, cancelable: true });",
        );
        let event = make.call1(&JsValue::NULL, &name.into()).expect("event");
        target
            .dispatch_event(event.unchecked_ref())
            .expect("dispatch");
    }

    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn it_names_the_tab_through_the_host_from_the_library() {
        let titles = record_bridge_calls("setTitle");
        define_from_library(CORE_LIBRARY, "tab-title");
        let host = render("tab-title").await;
        settle_until(|| defined("tab-title")).await;
        assert!(
            defined("tab-title"),
            "the library definition of <tab-title> was never installed"
        );

        let _ = host.set_attribute("text", "welcome");
        settle_until(|| titles.length() >= 1).await;
        assert_eq!(
            titles.get(0).as_string().as_deref(),
            Some("welcome"),
            "a text is pushed to the host, which owns the title",
        );

        let _ = host.set_attribute("hidden", "");
        let _ = host.set_attribute("text", "unseen");
        settle_briefly().await;
        assert_eq!(titles.length(), 1, "nothing is pushed while hidden");

        let _ = host.remove_attribute("hidden");
        settle_until(|| titles.length() >= 2).await;
        assert_eq!(
            titles.get(1).as_string().as_deref(),
            Some("unseen"),
            "unhiding pushes the title it was keeping",
        );
        let _ = host.set_attribute("text", "");
        settle_briefly().await;
        assert_eq!(titles.length(), 2, "an empty text is not a title");
    }

    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn it_mounts_the_location_once_the_display_is_bound() {
        define_from_library(CORE_LIBRARY, "page-mount");
        install_fake_host();
        install();
        let display = document().create_element("tonk-display").expect("display");
        let page = document().create_element("page-mount").expect("page");
        display.append_child(&page).expect("nest");
        let mounts = js_sys::Array::new();
        let recorded = mounts.clone();
        let on_mount = Closure::<dyn FnMut(CustomEvent)>::new(move |event: CustomEvent| {
            recorded.push(&event.detail());
        });
        let _ =
            display.add_event_listener_with_callback("mount", on_mount.as_ref().unchecked_ref());
        on_mount.forget();
        document()
            .body()
            .expect("body")
            .append_child(&display)
            .expect("attach");
        settle_until(|| defined("page-mount")).await;
        assert!(
            defined("page-mount"),
            "the library definition of <page-mount> was never installed"
        );
        settle_briefly().await;
        assert_eq!(
            mounts.length(),
            0,
            "no mount before the enclosing display is bound"
        );

        let _ = display.set_attribute("data-bound", "");
        settle_until(|| mounts.length() >= 1).await;
        let detail = mounts.get(0);
        let pathname = Reflect::get(&detail, &"pathname".into())
            .ok()
            .and_then(|value| value.as_string());
        assert!(
            pathname
                .as_deref()
                .is_some_and(|path| path.starts_with('/')),
            "the detail is the parsed location, got {pathname:?}",
        );
        assert!(
            Reflect::get(&detail, &"searchParams".into()).is_ok_and(|value| value.is_object()),
            "search params ride as a plain object",
        );

        fire(&page, "tonk:join-retry");
        settle_until(|| mounts.length() >= 2).await;
        assert_eq!(mounts.length(), 2, "a join retry mounts again");
    }

    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn it_commits_on_enter_and_restores_on_escape_from_the_library() {
        define_from_library(CORE_LIBRARY, "inline-editable");
        let host = render("inline-editable").await;
        settle_until(|| host.get_attribute("role").as_deref() == Some("textbox")).await;
        assert!(
            defined("inline-editable"),
            "the library definition of <inline-editable> was never installed"
        );
        assert_eq!(
            host.get_attribute("role").as_deref(),
            Some("textbox"),
            "connected ran"
        );
        host.set_text_content(Some("before"));
        let changes = js_sys::Array::new();
        let recorded = changes.clone();
        let on_change = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            recorded.push(&JsValue::TRUE);
        });
        let _ = host.add_event_listener_with_callback("change", on_change.as_ref().unchecked_ref());
        on_change.forget();

        fire(&host, "dblclick");
        settle_briefly().await;
        assert_eq!(
            host.get_attribute("contenteditable").as_deref(),
            Some("plaintext-only"),
            "a double-click opens the edit",
        );
        host.set_text_content(Some("after"));
        keydown(&host, "Enter");
        settle_briefly().await;
        assert_eq!(
            host.get_attribute("contenteditable").as_deref(),
            Some("false"),
            "Enter ends the edit",
        );
        assert_eq!(changes.length(), 1, "a changed text fires change on commit");

        fire(&host, "dblclick");
        settle_briefly().await;
        host.set_text_content(Some("half-typed"));
        keydown(&host, "Escape");
        settle_briefly().await;
        assert_eq!(
            host.text_content().as_deref(),
            Some("after"),
            "Escape restores the text the edit began with",
        );
        assert_eq!(changes.length(), 1, "a cancelled edit fires no change");

        assert_eq!(
            Reflect::get(&host, &"value".into())
                .ok()
                .and_then(|value| value.as_string())
                .as_deref(),
            Some("after"),
            "value reads the text",
        );
        let _ = Reflect::set(&host, &"value".into(), &"set".into());
        assert_eq!(
            host.text_content().as_deref(),
            Some("set"),
            "setting value writes the text",
        );
    }

    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn it_asks_the_host_to_open_recovery_on_click_from_the_library() {
        let calls = record_bridge_calls("register");
        define_from_library(PROFILE_LIBRARY, "space-login");
        let host = render("space-login").await;
        settle_until(|| defined("space-login")).await;
        assert!(
            defined("space-login"),
            "the library definition of <space-login> was never installed"
        );
        settle_briefly().await;

        fire(&host, "click");
        settle_until(|| calls.length() >= 1).await;
        assert_eq!(
            calls.get(0).as_string().as_deref(),
            Some(r#"{"reason":"space-login"}"#),
            "a click asks the host to open account recovery",
        );
    }

    fn space_remove_host(attributes: &[(&str, &str)]) -> Element {
        install_fake_host();
        install();
        let host = document().create_element("space-remove").expect("host");
        for (name, value) in attributes {
            let _ = host.set_attribute(name, value);
        }
        host.set_inner_html(
            r#"<button type="button" data-space-remove-open disabled>checking…</button><fake-dialog data-space-remove-dialog heading="confirm"><form id="remove-x" data-remove></form><button type="submit" data-space-remove-submit></button></fake-dialog>"#,
        );
        document()
            .body()
            .expect("body")
            .append_child(&host)
            .expect("attach");
        host
    }

    fn text_of(host: &Element, selector: &str) -> String {
        host.query_selector(selector)
            .ok()
            .flatten()
            .and_then(|element| element.text_content())
            .unwrap_or_default()
    }

    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn it_names_delete_or_leave_for_a_hub_row_from_the_library() {
        define_from_library(PROFILE_LIBRARY, "space-remove");

        let local = space_remove_host(&[("data-space-name", "notes"), ("data-space-founded", "1")]);
        settle_until(|| local.get_attribute("data-space-action").is_some()).await;
        assert_eq!(
            local.get_attribute("data-space-action").as_deref(),
            Some("delete-local"),
            "a space founded here with no provider is deleted locally",
        );
        assert_eq!(text_of(&local, "[data-space-remove-open]"), "delete");
        assert!(
            local
                .query_selector("[data-space-remove-open][disabled]")
                .ok()
                .flatten()
                .is_none(),
            "classifying enables the opener",
        );

        let invited = space_remove_host(&[
            ("data-space-name", "forum"),
            ("data-space-provider", "did:key:zHost"),
        ]);
        settle_until(|| invited.get_attribute("data-space-action").is_some()).await;
        assert_eq!(
            invited.get_attribute("data-space-action").as_deref(),
            Some("leave"),
            "a space another account provides is left",
        );
        assert_eq!(text_of(&invited, "[data-space-remove-open]"), "leave");

        let hosted = space_remove_host(&[
            ("data-space-name", "mine"),
            ("data-space-provider", "did:key:zMe"),
            ("data-space-owner", "did:key:zMe"),
        ]);
        settle_until(|| hosted.get_attribute("data-space-action").is_some()).await;
        assert_eq!(
            hosted.get_attribute("data-space-action").as_deref(),
            Some("delete-hosted"),
            "a space this account provides is deleted, hosted copy included",
        );

        let _ = invited.set_attribute("data-space-owner", "did:key:zHost");
        settle_until(|| {
            invited.get_attribute("data-space-action").as_deref() == Some("delete-hosted")
        })
        .await;
        assert_eq!(
            invited.get_attribute("data-space-action").as_deref(),
            Some("delete-hosted"),
            "ownership arriving later re-classifies the row",
        );
    }

    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn it_prepares_and_opens_the_row_dialog_from_the_library() {
        define_from_library(PROFILE_LIBRARY, "space-remove");
        let host = space_remove_host(&[("data-space-name", "forum")]);
        settle_until(|| host.get_attribute("data-space-action").is_some()).await;
        let dialog = host
            .query_selector("[data-space-remove-dialog]")
            .ok()
            .flatten()
            .expect("dialog");
        let shown = js_sys::Array::new();
        let recorded = shown.clone();
        let show = Closure::<dyn FnMut()>::new(move || {
            recorded.push(&JsValue::TRUE);
        });
        let _ = Reflect::set(
            &dialog,
            &"show".into(),
            show.as_ref().unchecked_ref::<js_sys::Function>(),
        );
        show.forget();

        let opener = host
            .query_selector("[data-space-remove-open]")
            .ok()
            .flatten()
            .expect("opener");
        let click = js_sys::Function::new_no_args("return new Event('click', { bubbles: true });")
            .call0(&JsValue::NULL)
            .expect("click");
        opener
            .dispatch_event(click.unchecked_ref())
            .expect("dispatch");
        settle_until(|| shown.length() >= 1).await;

        assert_eq!(shown.length(), 1, "the opener shows the row's dialog");
        assert_eq!(
            dialog.get_attribute("heading").as_deref(),
            Some("confirm leaving space"),
            "the dialog is prepared for the row's verb",
        );
        assert!(
            text_of(&host, "form[data-remove]").starts_with("Leave forum?"),
            "the copy names the space",
        );
        assert_eq!(text_of(&host, "[data-space-remove-submit]"), "leave space");
    }

    fn drag_frame() -> Element {
        install_fake_host();
        install();
        let host = document().create_element("drag-frame").expect("frame");
        let _ = host.set_attribute(
            "style",
            "position:fixed;width:40px;height:40px;left:200px;top:150px;",
        );
        document()
            .body()
            .expect("body")
            .append_child(&host)
            .expect("attach");
        host
    }

    /// A synthetic pointer event, dispatched on `target`.
    fn pointer(target: &web_sys::EventTarget, kind: &str, x: f64, y: f64, buttons: i32) {
        let make = js_sys::Function::new_with_args(
            "kind, x, y, buttons",
            "return new PointerEvent(kind, { clientX: x, clientY: y, buttons, button: 0, pointerId: 1, pointerType: 'mouse', bubbles: true, cancelable: true });",
        );
        let event = make
            .call4(
                &JsValue::NULL,
                &kind.into(),
                &x.into(),
                &y.into(),
                &buttons.into(),
            )
            .expect("pointer event");
        target
            .dispatch_event(event.unchecked_ref())
            .expect("dispatch");
    }

    fn px(host: &Element, property: &str) -> f64 {
        let style: web_sys::CssStyleDeclaration =
            host.unchecked_ref::<web_sys::HtmlElement>().style();
        style
            .get_property_value(property)
            .ok()
            .and_then(|value| value.trim_end_matches("px").parse().ok())
            .unwrap_or(f64::NAN)
    }

    fn viewport() -> (f64, f64) {
        let win = window().expect("window");
        let read =
            |value: Result<JsValue, JsValue>| value.ok().and_then(|v| v.as_f64()).unwrap_or(0.0);
        (read(win.inner_width()), read(win.inner_height()))
    }

    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn it_drags_a_frame_and_docks_it_to_the_nearest_edge_from_the_library() {
        define_from_library(CORE_LIBRARY, "drag-frame");
        let host = drag_frame();
        settle_until(|| host.has_attribute("inset")).await;
        let ends = js_sys::Array::new();
        let recorded = ends.clone();
        let on_end = Closure::<dyn FnMut(CustomEvent)>::new(move |event: CustomEvent| {
            recorded.push(&event.detail());
        });
        let _ = host.add_event_listener_with_callback("drag-end", on_end.as_ref().unchecked_ref());
        on_end.forget();
        let win = window().expect("window");
        let (_, vh) = viewport();
        let target_y = (vh / 2.0).floor();

        let clicks = js_sys::Array::new();
        let recorded = clicks.clone();
        let on_click = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            recorded.push(&JsValue::TRUE);
        });
        let _ =
            document().add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref());
        on_click.forget();
        pointer(&host, "pointerdown", 220.0, 170.0, 1);
        pointer(&win, "pointermove", 80.0, target_y, 1);
        assert!(
            host.has_attribute("dragging"),
            "past the dead zone the press is a drag"
        );
        pointer(&win, "pointerup", 80.0, target_y, 0);
        // The click a browser raises right after the release is not a click.
        fire(&host, "click");
        assert_eq!(
            clicks.length(),
            0,
            "the click that ends a drag is swallowed"
        );
        settle_until(|| ends.length() >= 1).await;
        fire(&host, "click");
        assert_eq!(clicks.length(), 1, "a later click reaches the page again");

        assert!(!host.has_attribute("dragging"));
        assert_eq!(
            host.get_attribute("docked").as_deref(),
            Some("left"),
            "release docks to the nearest edge"
        );
        assert_eq!(px(&host, "left"), 16.0, "the docked edge sits at the inset");
        let expected_top = 150.0 + (target_y - 170.0);
        assert!(
            (px(&host, "top") - expected_top).abs() < 1.0,
            "docking keeps the coordinate along the edge, got {} for {expected_top}",
            px(&host, "top"),
        );
        let edge = Reflect::get(&ends.get(0), &"edge".into())
            .ok()
            .and_then(|value| value.as_string());
        assert_eq!(edge.as_deref(), Some("left"), "drag-end names the edge");
    }

    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn it_treats_a_press_within_the_dead_zone_as_a_tap_from_the_library() {
        define_from_library(CORE_LIBRARY, "drag-frame");
        let host = drag_frame();
        settle_until(|| host.has_attribute("inset")).await;
        let clicks = js_sys::Array::new();
        let recorded = clicks.clone();
        let on_click = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            recorded.push(&JsValue::TRUE);
        });
        let _ =
            document().add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref());
        on_click.forget();
        let win = window().expect("window");

        pointer(&host, "pointerdown", 220.0, 170.0, 1);
        pointer(&win, "pointermove", 222.0, 171.0, 1);
        assert!(!host.has_attribute("dragging"), "two pixels is still a tap");
        pointer(&win, "pointerup", 222.0, 171.0, 0);
        settle_briefly().await;

        assert_eq!(
            px(&host, "left"),
            200.0,
            "a tap leaves the frame where it was"
        );
        assert!(!host.has_attribute("docked"), "a tap does not dock");
        fire(&host, "click");
        settle_briefly().await;
        assert_eq!(
            clicks.length(),
            1,
            "the tap's click reaches the content and the page"
        );
    }

    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn it_keeps_the_frame_inside_the_viewport_on_resize_from_the_library() {
        define_from_library(CORE_LIBRARY, "drag-frame");
        let host = drag_frame();
        settle_until(|| host.has_attribute("inset")).await;
        let (vw, _) = viewport();
        let style: web_sys::CssStyleDeclaration =
            host.unchecked_ref::<web_sys::HtmlElement>().style();
        let _ = style.set_property("left", "5000px");

        fire(&window().expect("window"), "resize");
        settle_briefly().await;

        assert_eq!(host.get_attribute("docked").as_deref(), Some("right"));
        assert_eq!(
            px(&host, "left"),
            vw - 40.0 - 16.0,
            "the frame is pulled back to the right edge"
        );
    }

    /// The account cell is a menu button only while an account is
    /// linked. Unlinked, one press is one action and the dropdown ARIA
    /// would promise a menu that never opens; linked, the menu finds its
    /// opener through `aria-controls`, so the attributes must come back.
    #[dialog_common::test]
    async fn it_dresses_the_account_cell_as_a_menu_button_only_when_linked() {
        install_fake_host();
        install();
        define_from_library(PROFILE_LIBRARY, "hub-bar");
        let host = document().create_element("hub-bar").expect("host");
        host.set_inner_html(
            r#"<nav class="hubbar"><button type="button" data-tab="account" data-account-trigger><span data-account-label>add an account</span><span data-registered></span></button></nav>"#,
        );
        document()
            .body()
            .expect("body")
            .append_child(&host)
            .expect("attach");
        settle_until(|| defined("hub-bar")).await;
        assert!(
            defined("hub-bar"),
            "the library definition of <hub-bar> was never installed"
        );
        settle_briefly().await;
        let trigger = host
            .query_selector("[data-account-trigger]")
            .expect("query")
            .expect("the trigger");

        assert_eq!(
            trigger.get_attribute("aria-haspopup"),
            None,
            "unlinked, the cell is not a menu button"
        );
        assert_eq!(trigger.get_attribute("aria-controls"), None);

        let registered = host
            .query_selector("[data-registered]")
            .expect("query")
            .expect("the registration slot");
        registered.set_inner_html(r#"<span data-account-linked hidden></span>"#);
        settle_until(|| trigger.has_attribute("aria-haspopup")).await;
        assert_eq!(
            trigger.get_attribute("aria-haspopup").as_deref(),
            Some("menu"),
            "linked, the cell is the account-menu button"
        );
        assert_eq!(
            trigger.get_attribute("aria-controls").as_deref(),
            Some("hub-account-menu")
        );
        assert_eq!(
            trigger.get_attribute("aria-expanded").as_deref(),
            Some("false")
        );

        registered.set_inner_html("");
        settle_until(|| !trigger.has_attribute("aria-haspopup")).await;
        assert_eq!(
            trigger.get_attribute("aria-haspopup"),
            None,
            "unlinking strips the menu-button ARIA again"
        );
        assert_eq!(trigger.get_attribute("aria-expanded"), None);
    }

    /// The first link opens the ceremony in place; adding an account
    /// parks it for the reload the branch rotation brings. Both cross to
    /// the top page as `window.tonk.register`, and only the reason tells
    /// them apart, so the reason is what this checks.
    #[dialog_common::test]
    async fn it_asks_the_top_page_with_the_reason_that_fits_the_click() {
        install_fake_host();
        install();
        let calls = record_bridge_calls("register");
        define_from_library(PROFILE_LIBRARY, "hub-bar");
        let host = document().create_element("hub-bar").expect("host");
        host.set_inner_html(
            r#"<nav class="hubbar"><button type="button" data-tab="account" data-account-trigger>add an account<span data-account-registration data-state="empty"></span></button></nav><button type="button" data-add-profile>add account</button>"#,
        );
        document()
            .body()
            .expect("body")
            .append_child(&host)
            .expect("attach");
        settle_until(|| defined("hub-bar")).await;
        assert!(
            defined("hub-bar"),
            "the library definition of <hub-bar> was never installed"
        );
        settle_briefly().await;

        let trigger = host
            .query_selector("[data-account-trigger]")
            .expect("query")
            .expect("the trigger");
        fire(&trigger, "click");
        settle_until(|| calls.length() >= 1).await;
        let first: serde_json::Value =
            serde_json::from_str(&calls.get(0).as_string().expect("a payload")).expect("json");
        assert_eq!(
            first["reason"], "needs-account",
            "an unlinked cell opens the ceremony in place"
        );
        assert!(
            first["anchor"].is_object(),
            "and seats it at the bar: {first}"
        );
        assert_eq!(host.get_attribute("linking").as_deref(), Some("true"));
        assert_eq!(host.get_attribute("tab").as_deref(), Some("account"));

        let add = host
            .query_selector("[data-add-profile]")
            .expect("query")
            .expect("the add-profile control");
        fire(&add, "click");
        settle_until(|| calls.length() >= 2).await;
        let second: serde_json::Value =
            serde_json::from_str(&calls.get(1).as_string().expect("a payload")).expect("json");
        assert_eq!(
            second["reason"], "profile-transition",
            "adding an account parks the ceremony for the reload"
        );
    }

    /// A click before the registration display has resolved neither
    /// opens a menu nor raises the ceremony: it waits, and acts once the
    /// display says which. A slower page (CI) clicked before the display
    /// resolved and got a signup where the menu was meant.
    #[dialog_common::test]
    async fn it_waits_for_the_registration_before_acting_on_the_account_cell() {
        install_fake_host();
        install();
        let calls = record_bridge_calls("register");
        define_from_library(PROFILE_LIBRARY, "hub-bar");
        let host = document().create_element("hub-bar").expect("host");
        host.set_inner_html(
            r#"<nav class="hubbar"><button type="button" data-tab="account" data-account-trigger><span data-registered></span><span data-account-registration data-state="loading"></span></button></nav>"#,
        );
        document()
            .body()
            .expect("body")
            .append_child(&host)
            .expect("attach");
        settle_until(|| defined("hub-bar")).await;
        assert!(
            defined("hub-bar"),
            "the library definition of <hub-bar> was never installed"
        );
        settle_briefly().await;
        let before = calls.length();

        let trigger = host
            .query_selector("[data-account-trigger]")
            .expect("query")
            .expect("the trigger");
        fire(&trigger, "click");
        settle_briefly().await;
        assert_eq!(
            calls.length(),
            before,
            "an unresolved registration asks for nothing yet"
        );
        assert!(host.has_attribute("deciding"), "the click is remembered");

        let registration = host
            .query_selector("[data-account-registration]")
            .expect("query")
            .expect("the registration display");
        let _ = registration.set_attribute("data-state", "empty");
        settle_until(|| calls.length() > before).await;
        let asked: serde_json::Value =
            serde_json::from_str(&calls.get(before).as_string().expect("a payload")).expect("json");
        assert_eq!(
            asked["reason"], "needs-account",
            "resolved empty, the click links"
        );
        assert!(!host.has_attribute("deciding"));

        // The top page tearing the ceremony down returns the bar to spaces.
        assert_eq!(host.get_attribute("tab").as_deref(), Some("account"));
        fire(&window().expect("window"), "tonk:registration-closed");
        settle_briefly().await;
        assert_eq!(host.get_attribute("linking").as_deref(), Some("false"));
        assert_eq!(host.get_attribute("tab").as_deref(), Some("spaces"));
    }

    /// A settings page opened by a browser with no account raises the
    /// ceremony itself once the registration resolves empty: that page
    /// is the door, and there is no cell to press.
    #[dialog_common::test]
    async fn it_raises_the_ceremony_on_an_unlinked_settings_page() {
        install_fake_host();
        install();
        let calls = record_bridge_calls("register");
        define_from_library(PROFILE_LIBRARY, "hub-bar");
        let host = document().create_element("hub-bar").expect("host");
        let _ = host.set_attribute("tab", "account");
        host.set_inner_html(
            r#"<nav class="hubbar"><button type="button" data-tab="account"><span data-account-registration data-state="loading"></span></button></nav>"#,
        );
        document()
            .body()
            .expect("body")
            .append_child(&host)
            .expect("attach");
        settle_until(|| defined("hub-bar")).await;
        assert!(
            defined("hub-bar"),
            "the library definition of <hub-bar> was never installed"
        );
        settle_briefly().await;
        let before = calls.length();

        let registration = host
            .query_selector("[data-account-registration]")
            .expect("query")
            .expect("the registration display");
        let _ = registration.set_attribute("data-state", "empty");
        settle_until(|| calls.length() > before).await;
        let asked: serde_json::Value =
            serde_json::from_str(&calls.get(before).as_string().expect("a payload")).expect("json");
        assert_eq!(asked["reason"], "needs-account");
        assert_eq!(host.get_attribute("linking").as_deref(), Some("true"));
    }

    /// Mount the settings panel's element with `markup` inside it.
    fn account_settings(markup: &str) -> Element {
        install_fake_host();
        install();
        define_from_library(PROFILE_LIBRARY, "account-settings");
        let host = document().create_element("account-settings").expect("host");
        host.set_inner_html(markup);
        document()
            .body()
            .expect("body")
            .append_child(&host)
            .expect("attach");
        host
    }

    fn set_context(fields: &[(&str, &str)]) {
        let context = js_sys::Object::new();
        for (key, value) in fields {
            let _ = Reflect::set(&context, &(*key).into(), &(*value).into());
        }
        let _ = Reflect::set(&host_bridge(), &"context".into(), &context);
    }

    /// A terminal's request rides the location. The element reads it
    /// off the injected context (the guest's own location is
    /// about:srcdoc), shows the request pane, and copies the request
    /// onto the approve control so the click can assert it, with the
    /// callback base58-encoded the way the worker decodes it. Declining
    /// hands the terminal a deny on its own loopback callback.
    #[dialog_common::test]
    async fn it_reads_a_terminal_request_off_the_location_from_the_library() {
        let navigations = record_bridge_calls("navigate");
        set_context(&[
            ("origin", "https://tonk.test"),
            ("path", "/settings/link"),
            (
                "search",
                "?audience=did%3Akey%3AzTerminal&callback=http%3A%2F%2F127.0.0.1%3A4321%2F&name=e2e+terminal",
            ),
            ("hash", ""),
        ]);
        let host = account_settings(
            r#"<div class="pane" data-pane="account"></div><div class="pane" data-pane="link" hidden><b data-link-name></b><b data-link-account></b><b data-link-did></b><button type="button" data-link-decline>decline</button><button type="button" data-link-approve data-audience="" data-callback="" data-name="" data-expected-account="">approve</button></div><p data-ceremony-status hidden></p>"#,
        );
        settle_until(|| defined("account-settings")).await;
        settle_briefly().await;

        let link = host
            .query_selector("[data-pane=\"link\"]")
            .expect("query")
            .expect("the link pane");
        assert!(!link.has_attribute("hidden"), "the request pane is shown");
        let account = host
            .query_selector("[data-pane=\"account\"]")
            .expect("query")
            .expect("the account pane");
        assert!(
            account.has_attribute("hidden"),
            "and the account pane is not"
        );
        assert_eq!(text_of(&host, "[data-link-name]"), "e2e terminal");
        assert_eq!(text_of(&host, "[data-link-did]"), "did:key:zTerminal");
        assert_eq!(
            text_of(&host, "[data-link-account]"),
            "your signed-in account"
        );
        let approve = host
            .query_selector("[data-link-approve]")
            .expect("query")
            .expect("the approve control");
        assert_eq!(
            approve.get_attribute("data-audience").as_deref(),
            Some("did:key:zTerminal")
        );
        assert_eq!(
            approve.get_attribute("data-callback").as_deref(),
            Some("VMK7D6XBoL4m6GErdKmWdY4t7ordCS"),
            "the callback is base58 over the URL"
        );
        assert_eq!(
            approve.get_attribute("data-name").as_deref(),
            Some("e2e terminal")
        );
        assert_eq!(
            approve.get_attribute("data-expected-account").as_deref(),
            Some(""),
            "a request naming no account leaves the field blank, which the click omits"
        );

        let decline = host
            .query_selector("[data-link-decline]")
            .expect("query")
            .expect("the decline control");
        fire(&decline, "click");
        settle_until(|| navigations.length() >= 1).await;
        assert_eq!(
            navigations.get(0).as_string().as_deref(),
            Some(
                "http://127.0.0.1:4321/#deny=declined+in+the+browser&redirect=https%3A%2F%2Ftonk.test%2Fsettings"
            ),
            "declining answers the terminal on its callback and returns here"
        );
        set_context(&[
            ("origin", "https://tonk.test"),
            ("path", "/"),
            ("search", ""),
            ("hash", ""),
        ]);
    }

    /// Deleting is armed by the exact phrase, and what it deletes is
    /// counted off the owned-space rows the view renders.
    #[dialog_common::test]
    async fn it_arms_the_deletion_on_the_exact_phrase_from_the_library() {
        set_context(&[
            ("origin", "https://tonk.test"),
            ("path", "/settings"),
            ("search", ""),
            ("hash", ""),
        ]);
        let host = account_settings(
            r#"<div class="pane" data-pane="account"><button type="button" data-delete-account-open>delete</button><div data-delete-account-dialog><span data-delete-scope>loading</span><span data-delete-owned hidden><span data-owned-space><span data-space-name>Welcome</span></span><span data-owned-space><span data-space-name>Doomed Garden</span></span></span><label>type <b data-delete-confirm-label>delete account</b></label><input data-delete-confirm type="text"><button type="button" data-delete-account-submit data-email="owner@example.com" disabled>delete</button></div></div><p data-ceremony-status hidden></p>"#,
        );
        settle_until(|| defined("account-settings")).await;
        settle_briefly().await;

        let opener = host
            .query_selector("[data-delete-account-open]")
            .expect("query")
            .expect("the opener");
        fire(&opener, "click");
        settle_briefly().await;
        assert_eq!(
            text_of(&host, "[data-delete-scope]"),
            "2 owned hosted spaces will be deleted: Welcome, Doomed Garden. Spaces you joined are left intact."
        );
        let submit = host
            .query_selector("[data-delete-account-submit]")
            .expect("query")
            .expect("the submit");
        assert!(
            submit.has_attribute("disabled"),
            "nothing typed, nothing armed"
        );

        let field: web_sys::HtmlInputElement = host
            .query_selector("[data-delete-confirm]")
            .expect("query")
            .expect("the field")
            .unchecked_into();
        field.set_value("delete");
        fire(&field, "input");
        settle_briefly().await;
        assert!(
            submit.has_attribute("disabled"),
            "a partial phrase does not arm"
        );

        field.set_value("delete account");
        fire(&field, "input");
        settle_until(|| !submit.has_attribute("disabled")).await;
        assert!(
            !submit.has_attribute("disabled"),
            "the exact phrase arms the submit"
        );
    }

    /// The ceremony row is worded for the person, and a terminal state
    /// stays on screen after the passkey cluster closes.
    #[dialog_common::test]
    async fn it_words_the_ceremony_row_from_the_library() {
        set_context(&[
            ("origin", "https://tonk.test"),
            ("path", "/settings"),
            ("search", ""),
            ("hash", ""),
        ]);
        let host = account_settings(
            r#"<div class="pane" data-pane="account"></div><p data-ceremony-status hidden></p><span data-rows></span>"#,
        );
        settle_until(|| defined("account-settings")).await;
        settle_briefly().await;
        let rows = host
            .query_selector("[data-rows]")
            .expect("query")
            .expect("the rows slot");
        let status = host
            .query_selector("[data-ceremony-status]")
            .expect("query")
            .expect("the status line");

        rows.set_inner_html(
            r#"<span data-ceremony-row data-ceremony="add-passkey" data-ceremony-state="pending-ceremony" data-ceremony-detail="" hidden></span>"#,
        );
        settle_until(|| !status.has_attribute("hidden")).await;
        assert_eq!(
            text_of(&host, "[data-ceremony-status]"),
            "Adding the passkey: waiting for your passkey\u{2026}"
        );
        assert_eq!(
            host.get_attribute("data-ceremony-state").as_deref(),
            Some("pending-ceremony")
        );

        rows.set_inner_html(
            r#"<span data-ceremony-row data-ceremony="add-passkey" data-ceremony-state="refused" data-ceremony-detail="no passkey" hidden></span>"#,
        );
        settle_until(|| text_of(&host, "[data-ceremony-status]").contains("did not finish")).await;
        assert_eq!(
            text_of(&host, "[data-ceremony-status]"),
            "Adding the passkey did not finish: no passkey"
        );

        // The cluster closing clears a transient line, not a verdict.
        fire(&window().expect("window"), "tonk:custody-closed");
        settle_briefly().await;
        assert_eq!(
            text_of(&host, "[data-ceremony-status]"),
            "Adding the passkey did not finish: no passkey",
            "a refusal stays on screen after the cluster closes"
        );
    }
}
