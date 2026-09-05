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

use crate::template::snapshot_template;
use custom_elements::CustomElement;
use js_sys::{Function, Reflect};
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

/// Read the `data-scalar-fields` attribute (a comma-separated list of the model
/// concept's `cardinality: one` field names) into a set for the planner. Empty
/// or absent yields an empty set — the value-driven default.
fn scalar_fields_attr(host: &Element) -> std::collections::BTreeSet<String> {
    host.get_attribute("data-scalar-fields")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
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
        //
        // `data-scalar-fields` (a comma-separated list the owning
        // `<tonk-display>` stamps from the model concept's `cardinality: one`
        // fields) tells the planner which fields are scalar substitutions, so an
        // optional scalar field used in a template is never treated as an
        // iteration root that drops its host element when the value is absent.
        // Absent attribute → empty set → the value-driven default.
        let scalar_fields = scalar_fields_attr(&host);
        let renderer = snapshot_template(&host)
            .ok()
            .map(|snapshot| Renderer::from_snapshot_with_scalars(snapshot, &scalar_fields));

        // Surface the event-handler bindings the renderer
        // discovered as JSON on a `data-event-bindings` attribute,
        // so the owning `<tonk-display>` can read them without
        // poking at the renderer through JS interop. Skip the
        // attribute entirely when the template has no handlers —
        // an empty `{events:[],concepts:[]}` is visual noise on
        // structural views (layout containers, etc.).
        if let Some(renderer) = &renderer {
            let bindings = renderer.event_bindings();
            if !bindings.event_types.is_empty()
                || !bindings.concept_names.is_empty()
                || !bindings.event_names.is_empty()
            {
                let json = serde_json::json!({
                    "events": bindings.event_types.iter().collect::<Vec<_>>(),
                    "concepts": bindings.concept_names.iter().collect::<Vec<_>>(),
                    "declarations": bindings.event_names.iter().collect::<Vec<_>>(),
                });
                if let Ok(serialized) = serde_json::to_string(&json) {
                    let _ = host.set_attribute("data-event-bindings", &serialized);
                }
            }
            // Advertise which HOST attributes this template reads via
            // `{dom.host/<attr>}`, space-separated, so the owning
            // `<tonk-display>` can watch exactly those for changes and
            // replay the frame through the binding diff. Skipped when the
            // template reads none — no attribute, no watcher.
            let host_attrs = renderer.host_attributes();
            if !host_attrs.is_empty() {
                let joined = host_attrs.iter().cloned().collect::<Vec<_>>().join(" ");
                let _ = host.set_attribute("data-host-bindings", &joined);
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
            // `draw` accepts a FRAME (an array of conclusions) — the
            // renderer renders one row per conclusion. A single
            // conclusion (`{this, fields}`) is accepted too and treated
            // as a one-row frame, so callers passing one entity still
            // work.
            let frame: Vec<Conclusion> =
                match serde_wasm_bindgen::from_value::<Vec<Conclusion>>(detail.clone()) {
                    Ok(frame) => frame,
                    Err(frame_err) => match serde_wasm_bindgen::from_value::<Conclusion>(detail) {
                        Ok(c) => vec![c],
                        Err(_) => {
                            web_sys::console::warn_1(&JsValue::from_str(&format!(
                                "tonk-view: draw payload parse failed: {frame_err}"
                            )));
                            return;
                        }
                    },
                };
            let mut s = state.borrow_mut();
            if let Some(renderer) = s.renderer.as_mut() {
                renderer.apply(&frame);
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
    use ipld_core::ipld::Ipld;
    use std::collections::BTreeMap;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    /// Convert a `serde_json::Value` into the equivalent [`Ipld`]
    /// for test setup — `Conclusion::fields` holds `Ipld` since the
    /// dag-json migration. `to_ipld(&Value::Null)` errors, so we
    /// walk the shape ourselves.
    fn json_to_ipld(value: &serde_json::Value) -> Ipld {
        match value {
            serde_json::Value::Null => Ipld::Null,
            serde_json::Value::Bool(b) => Ipld::Bool(*b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Ipld::Integer(i as i128)
                } else if let Some(u) = n.as_u64() {
                    Ipld::Integer(u as i128)
                } else if let Some(f) = n.as_f64() {
                    Ipld::Float(f)
                } else {
                    Ipld::Null
                }
            }
            serde_json::Value::String(s) => Ipld::String(s.clone()),
            serde_json::Value::Array(items) => Ipld::List(items.iter().map(json_to_ipld).collect()),
            serde_json::Value::Object(map) => Ipld::Map(
                map.iter()
                    .map(|(k, v)| (k.clone(), json_to_ipld(v)))
                    .collect(),
            ),
        }
    }

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

    /// Like [`mount`], but stamps `data-scalar-fields` (a comma-separated list
    /// of `cardinality: one` field names) before the element connects, so the
    /// renderer plans those fields as scalar substitutions rather than iteration
    /// axes.
    fn mount_with_scalars(template_html: &str, scalar_fields: &str) -> Element {
        register();
        let document = web_sys::window().expect("window").document().expect("doc");
        let host = document
            .create_element("tonk-view")
            .expect("create tonk-view");
        let _ = host.set_attribute("data-scalar-fields", scalar_fields);
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
        let mut map: BTreeMap<String, Ipld> = BTreeMap::new();
        for (k, v) in fields {
            map.insert((*k).to_owned(), Ipld::String((*v).to_owned()));
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

    /// A connected view advertises the host attributes its template reads
    /// via `{dom.host/<attr>}` on `data-host-bindings`, space-separated —
    /// the owning `<tonk-display>` watches exactly those for changes.
    #[dialog_common::test]
    fn it_advertises_dom_host_bindings_on_connect() {
        let host = mount(
            "<x-ring with=\"main@{dom.host/data-space}\"></x-ring>\
             <span>{dom.host/data-label}</span>",
        );
        assert_eq!(
            host.get_attribute("data-host-bindings").as_deref(),
            Some("data-label data-space"),
            "every dom.host reference is advertised, sorted"
        );
    }

    /// A template with no `dom.host/*` references advertises nothing — no
    /// attribute means the display installs no watcher.
    #[dialog_common::test]
    fn it_advertises_no_host_bindings_without_dom_host_refs() {
        let host = mount("<span>{name}</span>");
        assert_eq!(host.get_attribute("data-host-bindings"), None);
    }

    // The fix: a `cardinality: one` field declared in `data-scalar-fields` is a
    // scalar substitution, not an iteration axis. An element whose only hole is
    // such a field must render once (blank when absent) — `<tonk-site
    // path={rest}>` survives a bare-root render with no `rest`.
    #[dialog_common::test]
    fn it_keeps_a_scalar_field_host_when_the_value_is_absent() {
        // `{id}` (present) makes the <div> the repeat root, so the sibling <a>
        // bearing the only `{rest}` hole becomes an iteration root — which, when
        // `rest` is absent and undeclared, would clone zero times and drop it.
        let host = mount_with_scalars(
            "<div><span class=\"id\">{id}</span><a class=\"probe\" path=\"{rest}\">x</a></div>",
            "id,rest",
        );
        call_draw(&host, &detail("did:key:zX", &[("id", "did:key:zRepo")]));
        assert!(
            host.query_selector("a.probe").unwrap().is_some(),
            "declared scalar field host was dropped when its value was absent: {}",
            host.inner_html(),
        );
    }

    // The contrast: WITHOUT the scalar declaration the same field is
    // value-driven, so an absent value clones its host zero times and drops it.
    // This is exactly the behaviour the `data-scalar-fields` threading fixes.
    #[dialog_common::test]
    fn it_drops_an_undeclared_absent_field_host() {
        let host = mount(
            "<div><span class=\"id\">{id}</span><a class=\"probe\" path=\"{rest}\">x</a></div>",
        );
        call_draw(&host, &detail("did:key:zX", &[("id", "did:key:zRepo")]));
        assert!(
            host.query_selector("a.probe").unwrap().is_none(),
            "expected the undeclared absent field to drop its host (value-driven): {}",
            host.inner_html(),
        );
    }

    // A display body that embeds a nested template-owning component
    // must not have that component's `<template>` stolen by
    // `<tonk-view>`'s own template snapshot. With the snapshot scoped
    // to skip nested components, the view finds no template of its
    // own, renders the subject once, and the nested `<template>`
    // survives intact for that component to hydrate against.
    #[dialog_common::test]
    fn it_preserves_a_nested_component_template() {
        let host = mount(
            "<div class=\"app\"><tonk-display entity=\"did:key:zBook\"><ul><template><li>{title}</li></template></ul></tonk-display></div>",
        );
        call_draw(&host, &detail("did:key:zLibrary", &[]));
        assert!(
            host.query_selector("tonk-display template")
                .unwrap()
                .is_some(),
            "nested component template was stripped: {}",
            host.inner_html(),
        );
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

    // A view whose entire template is a bare `{field}` text node, with no
    // wrapping element, must still substitute the field — the Hub's
    // repository-label view (`display: {name}`) is exactly this shape.
    // The text node survives the snapshot, but its binding must also
    // resolve and apply, or the card renders blank.
    #[dialog_common::test]
    fn it_renders_a_bare_text_node_template() {
        let host = mount("{name}");
        call_draw(&host, &detail("did:key:zX", &[("name", "home")]));
        let html = host.inner_html();
        assert!(
            html.contains("home"),
            "a bare `{{name}}` text template should render its value, got: {html:?}",
        );
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

    #[dialog_common::test]
    fn it_omits_the_event_bindings_attribute_when_the_template_has_no_handlers() {
        let host = mount("<div><span>{label}</span></div>");
        assert!(
            !host.has_attribute("data-event-bindings"),
            "structural templates should not write empty event-binding metadata; got: {}",
            host.outer_html(),
        );
    }

    // --- Iteration / cardinality-many tests ----------------------------

    /// Like [`detail`], but lets callers mix scalar and array
    /// field values. Used to drive the iteration-aware renderer
    /// with a folded conclusion (the shape `<tonk-display>::fold_rows`
    /// produces from a cardinality-many SSE frame).
    fn detail_json(this: &str, fields: &[(&str, serde_json::Value)]) -> JsValue {
        let mut map: BTreeMap<String, Ipld> = BTreeMap::new();
        for (k, v) in fields {
            map.insert((*k).to_owned(), json_to_ipld(v));
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

    #[dialog_common::test]
    fn it_preserves_existing_row_when_field_grows_from_scalar_to_array() {
        // The fold step collapses a one-row frame to a scalar
        // value (the JSON is `"col-a"`, not `["col-a"]`). A
        // subsequent frame with two rows arrives as an array
        // (`["col-a", "col-b"]`). The reconciler MUST keep the
        // existing `col-a` row mounted — without that, adding a
        // sibling column trashes the first column's sub-tree on
        // every cardinality transition.
        let host = mount("<ul><li subject={column}><leaf data-c={column} /></li></ul>");
        call_draw(
            &host,
            &detail_json("did:key:zBoard", &[("column", serde_json::json!("col-a"))]),
        );
        let first_li_before = host
            .query_selector("li")
            .unwrap()
            .expect("li mounted for col-a");
        let first_leaf_before = host
            .query_selector("leaf")
            .unwrap()
            .expect("leaf mounted for col-a");
        assert_eq!(
            first_li_before.get_attribute("subject").as_deref(),
            Some("col-a")
        );

        // Add col-b — `column` now arrives as an array.
        call_draw(
            &host,
            &detail_json(
                "did:key:zBoard",
                &[("column", serde_json::json!(["col-a", "col-b"]))],
            ),
        );

        // Two <li>s expected, col-a first, col-b second.
        let lis = host.query_selector_all("li").unwrap();
        assert_eq!(
            lis.length(),
            2,
            "expected two rows after growing scalar to array, got: {}",
            host.inner_html(),
        );
        let subjects: Vec<String> = (0..lis.length())
            .filter_map(|i| {
                lis.item(i)
                    .and_then(|n| n.dyn_into::<Element>().ok())
                    .and_then(|el| el.get_attribute("subject"))
            })
            .collect();
        assert_eq!(subjects, vec!["col-a".to_owned(), "col-b".to_owned()]);

        // Crucially: the FIRST row's node identity (and its inner
        // leaf) must be the same instance as before. Otherwise
        // anything stateful inside (custom-element mounts,
        // focus, live subscriptions) would have been torn down.
        let first_li_after = lis
            .item(0)
            .and_then(|n| n.dyn_into::<Element>().ok())
            .expect("first li present");
        let first_leaf_after = first_li_after
            .query_selector("leaf")
            .unwrap()
            .expect("leaf present in first li");
        assert!(
            first_li_before.is_same_node(Some(first_li_after.unchecked_ref())),
            "first row node identity should survive the scalar→array transition",
        );
        assert!(
            first_leaf_before.is_same_node(Some(first_leaf_after.unchecked_ref())),
            "first row descendant node identity should survive the transition",
        );
    }

    #[dialog_common::test]
    fn it_updates_a_scalar_attr_placeholder_in_place_without_rebuilding() {
        // A scalar field whose only placeholder lives on an
        // iteration root must NOT key the iteration row on the
        // value itself — otherwise changing the scalar (e.g.
        // editing a column's width) destroys the row and rebuilds
        // every descendant, which trashes inner state (focus,
        // mounted custom elements, in-flight subscriptions).
        //
        // The expected reconciliation: the same wrapper node
        // survives, its bound attribute is patched in place, and
        // any nested child node identity is preserved across the
        // update.
        let host = mount("<wrapper data-w={width}><leaf data-marker=\"keep-me\" /></wrapper>");
        call_draw(
            &host,
            &detail_json("did:key:zCol", &[("width", serde_json::json!("12"))]),
        );
        let wrapper_before = host
            .query_selector("wrapper")
            .unwrap()
            .expect("wrapper mounted on first draw");
        let leaf_before = host
            .query_selector("leaf")
            .unwrap()
            .expect("leaf mounted on first draw");
        assert_eq!(
            wrapper_before.get_attribute("data-w").as_deref(),
            Some("12")
        );

        call_draw(
            &host,
            &detail_json("did:key:zCol", &[("width", serde_json::json!("16"))]),
        );
        let wrapper_after = host
            .query_selector("wrapper")
            .unwrap()
            .expect("wrapper still mounted after scalar update");
        let leaf_after = host
            .query_selector("leaf")
            .unwrap()
            .expect("leaf still mounted after scalar update");

        assert_eq!(
            wrapper_after.get_attribute("data-w").as_deref(),
            Some("16"),
            "wrapper attribute should reflect the new scalar value",
        );
        assert!(
            wrapper_before.is_same_node(Some(wrapper_after.unchecked_ref())),
            "wrapper node identity should survive a scalar update; got rebuild",
        );
        assert!(
            leaf_before.is_same_node(Some(leaf_after.unchecked_ref())),
            "descendant node identity should survive a scalar update; got rebuild",
        );
    }

    #[dialog_common::test]
    fn it_substitutes_scalar_attr_placeholder_and_iterates_a_nested_marker() {
        // Mirrors the column-view template:
        //   <wrapper attr={width}>            scalar field on wrapper
        //     <div subject={tile}>             cardinality-many on child
        //       <leaf data={tile} />
        //     </div>
        //   </wrapper>
        //
        // The attr placeholder is a scalar field — the wrapper
        // should NOT iterate. The inner `subject={tile}` marker is
        // the only iteration root. Expected after one draw:
        //   - wrapper rendered once
        //   - wrapper@attr substituted with the scalar value
        //   - one inner row per tile value
        let host = mount(
            "<wrapper data-w={width}><div class=\"tile\" subject={tile}><leaf data-t={tile} /></div></wrapper>",
        );
        call_draw(
            &host,
            &detail_json(
                "did:key:zCol",
                &[
                    ("width", serde_json::json!("12")),
                    ("tile", serde_json::json!(["t-a", "t-b"])),
                ],
            ),
        );

        let wrappers = host.query_selector_all("wrapper").unwrap();
        assert_eq!(
            wrappers.length(),
            1,
            "wrapper should mount once for a scalar attr field, got: {}",
            host.inner_html(),
        );
        let wrapper = wrappers
            .item(0)
            .and_then(|n| n.dyn_into::<Element>().ok())
            .expect("wrapper element");
        assert_eq!(
            wrapper.get_attribute("data-w").as_deref(),
            Some("12"),
            "wrapper attr should interpolate scalar field, got: {}",
            wrapper.outer_html(),
        );

        let tiles = host.query_selector_all(".tile").unwrap();
        assert_eq!(
            tiles.length(),
            2,
            "inner iteration should produce one .tile per tile value, got: {}",
            host.inner_html(),
        );
        let leaves = host.query_selector_all("leaf").unwrap();
        let leaf_attrs: Vec<String> = (0..leaves.length())
            .filter_map(|i| {
                leaves
                    .item(i)
                    .and_then(|n| n.dyn_into::<Element>().ok())
                    .and_then(|el| el.get_attribute("data-t"))
            })
            .collect();
        assert_eq!(leaf_attrs, vec!["t-a".to_owned(), "t-b".to_owned()]);
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

    /// Build a serialized FRAME — a JS array of `{ this, fields }`
    /// conclusions, the shape `<tonk-display>` passes for a directory
    /// (one conclusion per instance). `draw` fans it out into one repeat
    /// row per conclusion keyed by `this`.
    fn frame(members: &[(&str, &[(&str, &str)])]) -> JsValue {
        let conclusions: Vec<Conclusion> = members
            .iter()
            .map(|(this, fields)| {
                let mut map: BTreeMap<String, Ipld> = BTreeMap::new();
                for (k, v) in *fields {
                    map.insert((*k).to_owned(), Ipld::String((*v).to_owned()));
                }
                Conclusion {
                    this: (*this).to_owned(),
                    fields: map,
                }
            })
            .collect();
        serde_wasm_bindgen::to_value(&conclusions).expect("serialize frame")
    }

    // A directory frame with N conclusions must render N repeat rows,
    // one per subject. Regression: the slide-mount replay used to push
    // only the lead conclusion (a single-conclusion serialize), so a
    // directory of [Alice, Bob] rendered Alice alone. The `{this}`
    // marker on `<wa-carousel-item>` makes it the repeat node; each
    // conclusion clones it once, stamped with its own `this`.
    #[dialog_common::test]
    fn it_renders_one_repeat_row_per_conclusion_in_a_directory_frame() {
        let host = mount(
            "<wa-carousel><wa-carousel-item subject={this}>{name}</wa-carousel-item></wa-carousel>",
        );
        call_draw(
            &host,
            &frame(&[
                ("did:key:zAlice", &[("name", "Alice")]),
                ("did:key:zBob", &[("name", "Bob")]),
            ]),
        );

        let items = host.query_selector_all("wa-carousel-item").unwrap();
        assert_eq!(
            items.length(),
            2,
            "a 2-conclusion frame must render 2 rows, got: {}",
            host.inner_html(),
        );

        // Each row carries its own `data-this=<this>` debug attribute and the
        // per-conclusion `{name}` resolved against that conclusion. Rows
        // are keyed in sorted-`this` order (zAlice < zBob).
        let first = items.item(0).unwrap();
        let first_el = first.dyn_ref::<Element>().expect("element");
        assert_eq!(
            first_el.get_attribute("data-this").as_deref(),
            Some("did:key:zAlice")
        );
        assert_eq!(first_el.text_content().as_deref(), Some("Alice"));

        let second = items.item(1).unwrap();
        let second_el = second.dyn_ref::<Element>().expect("element");
        assert_eq!(
            second_el.get_attribute("data-this").as_deref(),
            Some("did:key:zBob")
        );
        assert_eq!(second_el.text_content().as_deref(), Some("Bob"));
    }

    // A single-conclusion frame is just a one-row directory — the same
    // path, one repeat row. Guards the cardinality-one case the unified
    // "everything is a list of folds" model collapses into.
    #[dialog_common::test]
    fn it_renders_a_single_repeat_row_for_a_one_conclusion_frame() {
        let host = mount(
            "<wa-carousel><wa-carousel-item subject={this}>{name}</wa-carousel-item></wa-carousel>",
        );
        call_draw(&host, &frame(&[("did:key:zAlice", &[("name", "Alice")])]));

        let items = host.query_selector_all("wa-carousel-item").unwrap();
        assert_eq!(items.length(), 1, "one conclusion → one row");
        let only = items.item(0).unwrap();
        let only_el = only.dyn_ref::<Element>().expect("element");
        assert_eq!(
            only_el.get_attribute("data-this").as_deref(),
            Some("did:key:zAlice")
        );
        assert_eq!(only_el.text_content().as_deref(), Some("Alice"));
    }

    // Re-applying a frame that grew by one subject adds exactly one row
    // and preserves the existing row's node identity — the incremental
    // path the directory carousel relies on when an instance appears.
    #[dialog_common::test]
    fn it_appends_a_repeat_row_when_the_frame_grows() {
        let host = mount(
            "<wa-carousel><wa-carousel-item subject={this}>{name}</wa-carousel-item></wa-carousel>",
        );
        call_draw(&host, &frame(&[("did:key:zAlice", &[("name", "Alice")])]));
        let alice_before = host
            .query_selector_all("wa-carousel-item")
            .unwrap()
            .item(0)
            .expect("alice row");

        call_draw(
            &host,
            &frame(&[
                ("did:key:zAlice", &[("name", "Alice")]),
                ("did:key:zBob", &[("name", "Bob")]),
            ]),
        );

        let items = host.query_selector_all("wa-carousel-item").unwrap();
        assert_eq!(items.length(), 2, "frame grew to 2: {}", host.inner_html());
        let alice_after = items.item(0).expect("alice row still first");
        assert!(
            alice_before.is_same_node(Some(alice_after.unchecked_ref())),
            "Alice's row must keep its node identity across the grow",
        );
    }
}
