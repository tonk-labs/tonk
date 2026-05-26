//! `<tonk-view>` — a dumb single-template renderer.
//!
//! Authoring shape:
//!
//! ```html
//! <tonk-view>
//!   <p class="greeting">{message}</p>
//! </tonk-view>
//! ```
//!
//! Children at `connectedCallback` time become the template; the
//! element keeps a [`Renderer`] (binding plan + cloneable fragment)
//! and waits to be told what to paint. Owners drive it by calling
//! `el.render(conclusion)` — a single-argument method that accepts
//! a `{ this, fields }` JS object. The element inserts on the first
//! call and patches in place on subsequent calls; if no template
//! was supplied at mount time, calls are silently dropped.
//!
//! No network, no subscriptions, no upward events. The element is
//! purely "given X, paint" — owners (typically `<tonk-display>`)
//! are responsible for arranging the data and the lifetime.
//!
//! Internally the public `render` method on the prototype is a
//! tiny shim that reads `this.draw` — a per-instance closure
//! installed by `connectedCallback` — and calls it. The `draw`
//! closure captures the element's renderer state via
//! `Rc<RefCell<…>>`, so the prototype method doesn't need to
//! reach into any global element-to-state map.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::{Function, Reflect};
use tonk_concept::template::snapshot_template;
use tonk_schema::conclusion::Conclusion;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use web_sys::{Element, HtmlElement, window};

use crate::render::Renderer;

/// Per-instance state. Held by the `draw` closure attached to
/// the element so the prototype `render` method can find it via
/// a plain property lookup.
struct Inner {
    renderer: Option<Renderer>,
}

/// The custom element.
#[derive(Default)]
pub struct TonkView {
    inner: RefCell<Option<Rc<RefCell<Inner>>>>,
}

impl CustomElement for TonkView {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        let host: Element = this.clone().into();

        // Build a renderer from the child template, if there is one.
        // Empty hosts are fine — `.render()` calls then no-op until
        // someone fills children in and re-mounts.
        let renderer = snapshot_template(&host).ok().map(Renderer::from_snapshot);

        // Surface the event-handler bindings the renderer
        // discovered as JSON on a `data-event-bindings` attribute,
        // so the owning `<tonk-display>` can read them without
        // poking at the renderer through JS interop. Cheap stable
        // contract: two-field JSON object with sorted distinct
        // event types and concept names.
        if let Some(renderer) = &renderer {
            let bindings = renderer.event_bindings();
            let json = serde_json::json!({
                "events": bindings.event_types.iter().collect::<Vec<_>>(),
                "concepts": bindings.concept_names.iter().collect::<Vec<_>>(),
            });
            if let Ok(serialized) = serde_json::to_string(&json) {
                let _ = host.set_attribute("data-event-bindings", &serialized);
            }
        }

        let state = Rc::new(RefCell::new(Inner { renderer }));
        *self.inner.borrow_mut() = Some(state.clone());

        // Attach a per-instance `draw(detail)` closure. The
        // prototype `render` method (installed once by `register`)
        // looks this up via `Reflect.get` and calls it — that's
        // how a single prototype method reaches the right
        // instance's state.
        let draw = Closure::wrap(Box::new(move |detail: JsValue| {
            let conclusion: Conclusion = match serde_wasm_bindgen::from_value(detail) {
                Ok(c) => c,
                Err(_) => return,
            };
            let mut s = state.borrow_mut();
            if let Some(renderer) = s.renderer.as_mut() {
                renderer.apply(&conclusion);
            }
        }) as Box<dyn FnMut(JsValue)>);
        let draw_fn: &Function = draw.as_ref().unchecked_ref();
        let _ = Reflect::set(this.as_ref(), &"draw".into(), draw_fn);
        // The closure has to outlive the element — leaking is
        // correct here. The browser releases the element's
        // properties (including `draw`) when it garbage-collects
        // the host, so the closure becomes unreachable naturally.
        draw.forget();
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        // Drop the state — and with it the renderer. The mounted
        // row's DOM nodes are children of the host, which the
        // browser garbage-collects when the host detaches.
        self.inner.borrow_mut().take();
    }

    fn attribute_changed_callback(
        &mut self,
        _this: &HtmlElement,
        _name: String,
        _old: Option<String>,
        _new: Option<String>,
    ) {
    }
}

/// Register the `<tonk-view>` element with the page and augment
/// `HTMLElement.prototype.render` so callers can write
/// `el.render(conclusion)`.
///
/// Idempotent — re-registration is a no-op.
pub fn register() {
    if already_registered() {
        return;
    }
    TonkView::define("tonk-view");
    install_render_method();
}

fn already_registered() -> bool {
    let Some(win) = window() else {
        return false;
    };
    !win.custom_elements().get("tonk-view").is_undefined()
}

/// Install a single `render(detail)` method on the `<tonk-view>`
/// prototype that delegates to the per-instance `draw` closure
/// attached during `connected_callback`. Implemented as a real JS
/// function (not a `wasm_bindgen::Closure`) so that `this` follows
/// JS method-call semantics — `el.render(x)` binds `this=el`,
/// which lets us look up `this.draw` correctly. A `Closure::wrap`
/// would have given us a plain function with no `this` binding,
/// so the same call would land with `this=detail`.
fn install_render_method() {
    let Some(win) = window() else {
        return;
    };
    let constructor = win.custom_elements().get("tonk-view");
    if constructor.is_undefined() {
        return;
    }
    let Ok(proto) = Reflect::get(&constructor, &"prototype".into()) else {
        return;
    };
    let render_fn = Function::new_with_args(
        "detail",
        "if (typeof this.draw === 'function') this.draw(detail);",
    );
    let _ = Reflect::set(&proto, &"render".into(), &render_fn);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    /// Mount a `<tonk-view>` with the given template HTML as
    /// children and attach it to the body. `register()` runs at the
    /// top so the custom-element prototype is set up before the
    /// element connects.
    fn mount(template_html: &str) -> Element {
        register();
        let document = web_sys::window().expect("window").document().expect("doc");
        let host = document
            .create_element("tonk-view")
            .expect("create tonk-view");
        host.set_inner_html(template_html);
        document
            .body()
            .expect("body")
            .append_child(&host)
            .expect("attach");
        host
    }

    /// Build a serialized `{ this, fields }` JsValue conclusion the
    /// way `<tonk-display>` would pass it into `el.render(detail)`.
    fn detail(this: &str, fields: &[(&str, &str)]) -> JsValue {
        let mut map: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        for (k, v) in fields {
            map.insert((*k).to_owned(), serde_json::Value::String((*v).to_owned()));
        }
        let conclusion = Conclusion {
            this: this.to_owned(),
            fields: map,
        };
        serde_wasm_bindgen::to_value(&conclusion).expect("serialize conclusion")
    }

    /// Pull the `draw` closure off the element and call it
    /// directly. Mirrors how `<tonk-display>::call_render` invokes
    /// the per-instance closure; using the prototype `render`
    /// method would also work but adds an indirection that's not
    /// what we're testing here.
    fn call_draw(el: &Element, detail: &JsValue) {
        let draw = Reflect::get(el.as_ref(), &"draw".into()).expect("draw");
        let func: Function = draw.dyn_into().expect("draw is a function");
        func.call1(&JsValue::NULL, detail).expect("call draw");
    }

    #[dialog_common::test]
    fn it_installs_a_per_instance_draw_closure_on_connect() {
        let host = mount("<p>{name}</p>");
        // `draw` is the per-instance shim the prototype `render`
        // method delegates to. Without it, callers couldn't drive
        // the element at all.
        let draw = Reflect::get(host.as_ref(), &"draw".into()).expect("draw present");
        assert!(
            draw.dyn_into::<Function>().is_ok(),
            "expected draw to be a JS Function",
        );
    }

    #[dialog_common::test]
    fn it_renders_the_template_when_draw_is_called() {
        let host = mount("<article><h1>{name}</h1></article>");
        call_draw(&host, &detail("did:key:zX", &[("name", "Alice")]));
        let html = host.inner_html();
        assert!(html.contains("Alice"), "expected Alice in {html}");
    }

    #[dialog_common::test]
    fn it_reflects_the_latest_draw_payload() {
        let host = mount("<article><h1>{name}</h1></article>");
        call_draw(&host, &detail("did:key:zX", &[("name", "Alice")]));
        call_draw(&host, &detail("did:key:zX", &[("name", "Alicia")]));
        // Re-renders happen wholesale (no in-place node patching),
        // so node identity is *not* preserved across draws. What
        // matters is that the latest payload is what shows.
        assert!(host.inner_html().contains("Alicia"));
        assert!(
            !host.inner_html().contains("Alice<"),
            "stale name leaked into: {}",
            host.inner_html(),
        );
        assert_eq!(
            host.query_selector_all("article").unwrap().length(),
            1,
            "expected exactly one article after second draw",
        );
    }

    #[dialog_common::test]
    fn it_exposes_a_prototype_render_method_that_delegates_to_draw() {
        let host = mount("<p>{name}</p>");
        // Call `render` through the prototype the way external JS
        // would: `host.render(detail)`. This proves the `this`
        // binding works — without it, `render` would invoke the
        // closure with `this = detail`, which silently no-ops.
        let render = Reflect::get(host.as_ref(), &"render".into()).expect("render present");
        let render_fn: Function = render.dyn_into().expect("render is a Function");
        render_fn
            .call1(host.as_ref(), &detail("did:key:zX", &[("name", "Bob")]))
            .expect("call render");
        assert!(
            host.inner_html().contains("Bob"),
            "got: {}",
            host.inner_html()
        );
    }

    #[dialog_common::test]
    fn it_silently_drops_draws_when_no_template_was_supplied() {
        let host = mount("");
        // No template → no renderer → calling draw is a no-op.
        // Just checking that we don't panic and the host stays empty.
        call_draw(&host, &detail("did:key:zX", &[("name", "Alice")]));
        assert_eq!(host.inner_html(), "");
    }

    #[dialog_common::test]
    fn it_rewrites_on_event_attributes_in_the_template() {
        let host = mount("<article><button onclick=increment>+</button></article>");
        call_draw(&host, &detail("did:key:zCounter", &[("count", "0")]));
        // The browser still has the literal `onclick=increment`
        // attribute in the parsed-source DOM (HTML attribute parser
        // accepts unquoted values up to the next whitespace), so it
        // becomes `onclick="increment"`. Preprocess rewrites it
        // before plan extraction and DOM mount, so the rendered
        // button carries `data-onclick`, not `onclick`. Query the
        // button by attribute name rather than searching inner_html:
        // a string contains-check would match `data-onclick` as a
        // substring of `onclick` and silently invert the assertion.
        let button = host
            .query_selector("button")
            .expect("query_selector")
            .expect("button present after render");
        assert_eq!(
            button.get_attribute("data-onclick").as_deref(),
            Some("increment"),
            "expected data-onclick='increment' on the button; got attrs: {:?}",
            button.outer_html(),
        );
        assert!(
            !button.has_attribute("onclick"),
            "raw onclick should be gone from rendered button; got: {}",
            button.outer_html(),
        );
    }

    #[dialog_common::test]
    fn it_publishes_event_bindings_to_a_data_attribute_on_the_host() {
        let host = mount(
            "<article><button onclick=increment>+</button><button onkeydown=cancel>x</button></article>",
        );
        let raw = host
            .get_attribute("data-event-bindings")
            .expect("data-event-bindings present");
        let parsed: serde_json::Value =
            serde_json::from_str(&raw).expect("data-event-bindings is JSON");
        let events: Vec<String> = parsed["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        let concepts: Vec<String> = parsed["concepts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        assert_eq!(events, vec!["click", "keydown"]);
        assert_eq!(concepts, vec!["cancel", "increment"]);
    }

    // --- Iteration / cardinality-many tests ----------------------------

    /// Like [`detail`], but lets callers mix scalar and array
    /// field values. Used to drive the iteration-aware renderer
    /// with a folded conclusion (the shape `<tonk-display>::fold_rows`
    /// produces from a cardinality-many SSE frame).
    fn detail_json(this: &str, fields: &[(&str, serde_json::Value)]) -> JsValue {
        let mut map: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        for (k, v) in fields {
            map.insert((*k).to_owned(), v.clone());
        }
        let conclusion = Conclusion {
            this: this.to_owned(),
            fields: map,
        };
        serde_wasm_bindgen::to_value(&conclusion).expect("serialize conclusion")
    }

    #[dialog_common::test]
    fn it_repeats_the_iteration_root_once_per_value_in_an_array_field() {
        // The marker (`subject={item}`) lifts the iteration root
        // to the <li> — without it, the LCA of the two `{item}`
        // occurrences would be the inner <tonk-display> and only
        // that would repeat, leaving a single <li> wrapping every
        // clone (which is what the `<li>` direct-child-of-<ul>
        // rule effectively forbids in semantic terms).
        let host = mount(
            "<ul><li subject={item}><tonk-display entity={item}>{item}</tonk-display></li></ul>",
        );
        call_draw(
            &host,
            &detail_json(
                "did:key:zList",
                &[(
                    "item",
                    serde_json::json!(["did:key:zA", "did:key:zB", "did:key:zC"]),
                )],
            ),
        );
        let items = host.query_selector_all("li").unwrap();
        assert_eq!(items.length(), 3, "got: {}", host.inner_html());
        let texts: Vec<String> = (0..items.length())
            .filter_map(|i| items.item(i).and_then(|n| n.text_content()))
            .collect();
        assert!(texts.contains(&"did:key:zA".to_owned()));
        assert!(texts.contains(&"did:key:zB".to_owned()));
        assert!(texts.contains(&"did:key:zC".to_owned()));
    }

    #[dialog_common::test]
    fn it_substitutes_per_iteration_value_into_attributes_inside_the_root() {
        // The inner <tonk-display>'s `entity` attribute should
        // resolve to the current iteration's value — that's the
        // mechanism that makes the nested element subscribe to
        // the right entity. The `subject={item}` marker on <li>
        // raises the iteration root above the inner element so
        // each value gets its own <li> wrapper.
        let host = mount(
            "<ul><li subject={item}><tonk-display entity={item} model=todo></tonk-display></li></ul>",
        );
        call_draw(
            &host,
            &detail_json(
                "did:key:zList",
                &[("item", serde_json::json!(["did:key:zA", "did:key:zB"]))],
            ),
        );
        let displays = host.query_selector_all("tonk-display").unwrap();
        assert_eq!(displays.length(), 2);
        let entities: Vec<String> = (0..displays.length())
            .filter_map(|i| {
                displays
                    .item(i)
                    .and_then(|n| n.dyn_into::<Element>().ok())
                    .and_then(|el| el.get_attribute("entity"))
            })
            .collect();
        assert_eq!(
            entities,
            vec!["did:key:zA".to_owned(), "did:key:zB".to_owned()]
        );
    }

    #[dialog_common::test]
    fn it_removes_the_iteration_root_when_the_array_is_empty() {
        // No values → no `<li>` mounted. The empty list is the
        // result, not a stray template node.
        let host = mount("<ul><li>{item}</li></ul>");
        call_draw(
            &host,
            &detail_json("did:key:zList", &[("item", serde_json::json!([]))]),
        );
        let items = host.query_selector_all("li").unwrap();
        assert_eq!(items.length(), 0, "got: {}", host.inner_html());
        // The <ul> chrome stays — only the iteration root vanishes.
        assert!(host.query_selector("ul").unwrap().is_some());
    }

    #[dialog_common::test]
    fn it_renders_independent_iteration_roots_for_sibling_placeholders() {
        // <p>{item}</p> and <li>{item}</li> share no inner ancestor
        // — each becomes its own iteration root, each repeats per
        // value of `item`.
        let host = mount("<section><p>{item}</p><ul><li>{item}</li></ul></section>");
        call_draw(
            &host,
            &detail_json(
                "did:key:zList",
                &[("item", serde_json::json!(["one", "two"]))],
            ),
        );
        assert_eq!(host.query_selector_all("p").unwrap().length(), 2);
        assert_eq!(host.query_selector_all("li").unwrap().length(), 2);
    }

    #[dialog_common::test]
    fn it_treats_a_scalar_field_value_as_a_single_iteration() {
        // A folded conclusion keeps a scalar when every row
        // agreed; the renderer treats that as a one-element list
        // so the iteration root is mounted exactly once.
        let host = mount("<p>{name}</p>");
        call_draw(&host, &detail("did:key:zX", &[("name", "Alice")]));
        assert_eq!(host.query_selector_all("p").unwrap().length(), 1);
        assert_eq!(
            host.query_selector("p")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("Alice"),
        );
    }

    // --- Incremental update / DOM stability tests ---------------------
    //
    // The renderer's contract: subsequent applies reconcile in
    // place. Existing iteration rows keep their node identity;
    // new rows are added; vanished rows are removed; bindings
    // whose rendered value didn't change don't touch the DOM. The
    // tests below pin that contract — node identity preservation
    // is what authors who attach imperative state (focus, event
    // listeners, animation timelines) depend on.

    #[dialog_common::test]
    fn it_preserves_node_identity_for_unchanged_bindings_across_applies() {
        let host = mount("<p>{name}</p>");
        call_draw(&host, &detail("did:key:zX", &[("name", "Alice")]));
        let text_before = host
            .query_selector("p")
            .unwrap()
            .expect("p mounted")
            .first_child()
            .expect("p has text child");

        // Same payload → no DOM mutation expected.
        call_draw(&host, &detail("did:key:zX", &[("name", "Alice")]));
        let text_after = host
            .query_selector("p")
            .unwrap()
            .expect("p still mounted")
            .first_child()
            .expect("p still has text child");

        assert!(
            text_before.is_same_node(Some(text_after.unchecked_ref())),
            "text node identity should survive an unchanged apply",
        );
    }

    #[dialog_common::test]
    fn it_rewrites_only_changed_bindings_on_subsequent_apply() {
        let host = mount("<article><h1>{name}</h1><p>{bio}</p></article>");
        call_draw(
            &host,
            &detail("did:key:zX", &[("name", "Alice"), ("bio", "Hi")]),
        );
        let bio_text_before = host
            .query_selector("p")
            .unwrap()
            .unwrap()
            .first_child()
            .expect("bio text node");

        // Update only `name`; bio stays the same.
        call_draw(
            &host,
            &detail("did:key:zX", &[("name", "Alicia"), ("bio", "Hi")]),
        );

        // The name change is reflected.
        assert_eq!(
            host.query_selector("h1")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("Alicia"),
        );
        // Bio's text node was not rewritten — identity preserved.
        let bio_text_after = host
            .query_selector("p")
            .unwrap()
            .unwrap()
            .first_child()
            .expect("bio text node still present");
        assert!(
            bio_text_before.is_same_node(Some(bio_text_after.unchecked_ref())),
            "unchanged bindings should not touch the DOM",
        );
    }

    #[dialog_common::test]
    fn it_appends_a_new_row_without_disturbing_existing_rows() {
        let host = mount("<ul><li subject={item}>{item}</li></ul>");
        call_draw(
            &host,
            &detail_json(
                "did:key:zList",
                &[("item", serde_json::json!(["did:key:zA", "did:key:zB"]))],
            ),
        );
        let li_a_before = host
            .query_selector_all("li")
            .unwrap()
            .item(0)
            .expect("first li mounted");
        let li_b_before = host
            .query_selector_all("li")
            .unwrap()
            .item(1)
            .expect("second li mounted");

        // Add a third item — sorts last (zC > zB > zA).
        call_draw(
            &host,
            &detail_json(
                "did:key:zList",
                &[(
                    "item",
                    serde_json::json!(["did:key:zA", "did:key:zB", "did:key:zC"]),
                )],
            ),
        );

        let after = host.query_selector_all("li").unwrap();
        assert_eq!(after.length(), 3, "after apply: {}", host.inner_html());

        // Existing rows kept their node identity. New row mounted
        // in sorted position (last, since `zC` > `zB`).
        let li_a_after = after.item(0).expect("first li");
        let li_b_after = after.item(1).expect("second li");
        assert!(
            li_a_before.is_same_node(Some(li_a_after.unchecked_ref())),
            "row for zA should be the same node before and after",
        );
        assert!(
            li_b_before.is_same_node(Some(li_b_after.unchecked_ref())),
            "row for zB should be the same node before and after",
        );

        let li_c = after.item(2).expect("third li");
        let li_c_el = li_c.dyn_ref::<Element>().expect("li element");
        assert_eq!(
            li_c_el.text_content().as_deref(),
            Some("did:key:zC"),
            "new row's text",
        );
    }

    #[dialog_common::test]
    fn it_removes_a_row_for_a_vanished_key_without_touching_others() {
        let host = mount("<ul><li subject={item}>{item}</li></ul>");
        call_draw(
            &host,
            &detail_json(
                "did:key:zList",
                &[(
                    "item",
                    serde_json::json!(["did:key:zA", "did:key:zB", "did:key:zC"]),
                )],
            ),
        );
        let li_a_before = host
            .query_selector_all("li")
            .unwrap()
            .item(0)
            .expect("zA row");
        let li_c_before = host
            .query_selector_all("li")
            .unwrap()
            .item(2)
            .expect("zC row");

        // Drop zB.
        call_draw(
            &host,
            &detail_json(
                "did:key:zList",
                &[("item", serde_json::json!(["did:key:zA", "did:key:zC"]))],
            ),
        );

        let after = host.query_selector_all("li").unwrap();
        assert_eq!(after.length(), 2);
        let li_a_after = after.item(0).expect("first li");
        let li_c_after = after.item(1).expect("second li");
        assert!(
            li_a_before.is_same_node(Some(li_a_after.unchecked_ref())),
            "zA row identity preserved",
        );
        assert!(
            li_c_before.is_same_node(Some(li_c_after.unchecked_ref())),
            "zC row identity preserved",
        );
    }

    #[dialog_common::test]
    fn it_handles_successive_appends_without_duplicating_existing_rows() {
        // Simulates the user-reported bug shape: list starts with
        // one item, items get appended over multiple applies.
        // After three appends we should have four distinct rows
        // with no duplication or content bleed.
        let host = mount("<ul><li subject={item}>{item}</li></ul>");
        call_draw(
            &host,
            &detail_json("did:key:zList", &[("item", serde_json::json!(["zA"]))]),
        );
        call_draw(
            &host,
            &detail_json(
                "did:key:zList",
                &[("item", serde_json::json!(["zA", "zB"]))],
            ),
        );
        call_draw(
            &host,
            &detail_json(
                "did:key:zList",
                &[("item", serde_json::json!(["zA", "zB", "zC"]))],
            ),
        );
        call_draw(
            &host,
            &detail_json(
                "did:key:zList",
                &[("item", serde_json::json!(["zA", "zB", "zC", "zD"]))],
            ),
        );

        let items = host.query_selector_all("li").unwrap();
        let texts: Vec<String> = (0..items.length())
            .filter_map(|i| items.item(i).and_then(|n| n.text_content()))
            .collect();
        assert_eq!(
            texts,
            vec!["zA", "zB", "zC", "zD"],
            "expected four distinct rows in sorted order, got: {}",
            host.inner_html(),
        );
    }

    #[dialog_common::test]
    fn it_inserts_new_rows_in_sorted_key_order() {
        // Start with the middle key; add a key that sorts before
        // it and one that sorts after.
        let host = mount("<ul><li subject={item}>{item}</li></ul>");
        call_draw(
            &host,
            &detail_json("did:key:zList", &[("item", serde_json::json!(["zB"]))]),
        );
        call_draw(
            &host,
            &detail_json(
                "did:key:zList",
                &[("item", serde_json::json!(["zB", "zA", "zC"]))],
            ),
        );

        let items: Vec<String> = (0..host.query_selector_all("li").unwrap().length())
            .filter_map(|i| {
                host.query_selector_all("li")
                    .unwrap()
                    .item(i)
                    .and_then(|n| n.text_content())
            })
            .collect();
        assert_eq!(
            items,
            vec!["zA".to_owned(), "zB".to_owned(), "zC".to_owned()],
            "rows should appear in sorted-key order regardless of input array order",
        );
    }
}
