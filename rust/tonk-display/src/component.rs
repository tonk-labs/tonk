//! `<tonk-component>` — a realm-level loader for author-defined web
//! components.
//!
//! View templates are rendered through inert fragments
//! (`innerHTML` → snapshot → `cloneNode`), so a `<script>` written in
//! a view's `display` never executes. This element is the sanctioned
//! bridge across that gap: it carries author JavaScript as *data* and,
//! on connect, executes it exactly once per document realm by
//! appending a real `<script type="module">` to `<head>` — the one
//! insertion path the HTML spec does run.
//!
//! Authoring shapes, in precedence order:
//!
//! 1. **Attribute** — `<tonk-component module={module}>` where the
//!    view interpolates the source out of a concept field (the
//!    `tonk:component` concept in the standard library). Components
//!    are then branch data: hot-swappable, queryable, shared by every
//!    view in the realm.
//! 2. **Inert child holder** — a `<script type="tonk/module">` child
//!    written directly in a view's `display`. The parser gives
//!    `<script>` raw-text treatment (so JS braces and `<` survive) and
//!    the template walker never descends into scripts (so `{…}` in the
//!    JS is not mistaken for bindings); the holder itself stays inert,
//!    and this element lifts its text into the executing module.
//!
//! De-duplication is by content hash, keyed in the document's `<head>`
//! (`script[data-tonk-component=<hash>]`): the same module mounted by
//! many rows or many views executes once. A *changed* source is a new
//! hash and executes fresh — but `customElements.define` cannot
//! redefine a name, so component authors should guard with
//! `customElements.get(name) ||` and expect a reload to pick up
//! replacements.
//!
//! Trust: the module runs in the same realm as every other view in
//! the sealed guest — the same power a view author already has over
//! sibling views' DOM and the command pipeline. The branch remains the
//! trust boundary, exactly as it is for views and portals.

use custom_elements::CustomElement;
use wasm_bindgen::JsCast;
use web_sys::{Element, HtmlElement, window};

/// The custom element. Stateless: all bookkeeping lives in the
/// document (`<head>` marker scripts), so clones and re-mounts
/// coordinate naturally.
#[derive(Default)]
pub struct TonkComponent;

/// FNV-1a 64-bit over the module source — stable, dependency-free,
/// and collision-safe enough for "have I run this exact text".
fn fnv1a64(source: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in source.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Resolve the module source: the `module` attribute wins; otherwise
/// concatenate the text of child `<script>` holders (any `type` —
/// they are inert by construction, never executed by the parser).
fn source_of(host: &Element) -> Option<String> {
    if let Some(module) = host.get_attribute("module")
        && !module.trim().is_empty()
    {
        return Some(module);
    }
    let holders = host.query_selector_all("script").ok()?;
    let mut source = String::new();
    for i in 0..holders.length() {
        if let Some(text) = holders.item(i).and_then(|node| node.text_content()) {
            source.push_str(&text);
            source.push('\n');
        }
    }
    let trimmed = source.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// Execute the host's module source once per document realm. Marks
/// the host `data-state=loaded|empty` for inspectability.
fn load(this: &HtmlElement) {
    let host: Element = this.clone().into();
    let Some(source) = source_of(&host) else {
        let _ = host.set_attribute("data-state", "empty");
        return;
    };
    let document = host.owner_document().expect("element has a document");
    let Some(head) = document.head() else {
        return;
    };
    let hash = format!("{:016x}", fnv1a64(&source));
    let marker = format!("script[data-tonk-component=\"{hash}\"]");
    if document.query_selector(&marker).ok().flatten().is_some() {
        let _ = host.set_attribute("data-state", "loaded");
        return;
    }
    let Ok(script) = document.create_element("script") else {
        return;
    };
    let _ = script.set_attribute("type", "module");
    let _ = script.set_attribute("data-tonk-component", &hash);
    script.set_text_content(Some(&source));
    // Dynamically inserted scripts default to `async` (run in
    // whatever order they finish), but components load as a set and
    // one module frequently defines helpers the next one uses.
    // Forcing in-order execution makes mount order (directory row
    // order) the execution order — deterministic for authors.
    if let Some(script) = script.dyn_ref::<web_sys::HtmlScriptElement>() {
        script.set_async(false);
    }
    // Appending a created script element is the execution path —
    // unlike innerHTML/clone insertion, the spec runs it (module
    // semantics: deferred to the next microtask checkpoint).
    let _ = head.append_child(&script);
    let _ = host.set_attribute("data-state", "loaded");
}

impl CustomElement for TonkComponent {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["module"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        load(this);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        // Nothing to tear down: an executed module is realm-global by
        // design (customElements registrations can't be undone).
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        _name: String,
        old: Option<String>,
        new: Option<String>,
    ) {
        // A re-pointed `module` (edited component row) loads the new
        // source; hash de-dup makes repeats free. Pre-connect upgrade
        // callbacks are ignored — `connected_callback` reads live.
        if old != new && this.is_connected() {
            load(this);
        }
    }
}

/// Register `<tonk-component>` with the page. Idempotent.
pub fn register() {
    if already_registered() {
        return;
    }
    TonkComponent::define("tonk-component");
}

fn already_registered() -> bool {
    let Some(win) = window() else {
        return false;
    };
    !win.custom_elements().get("tonk-component").is_undefined()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    fn document() -> web_sys::Document {
        web_sys::window().expect("window").document().expect("doc")
    }

    /// Wait one macrotask. Inline module scripts evaluate in a task
    /// queued after insertion, so a single hop is not enough — use
    /// [`settle_until`] to wait for an observable effect.
    async fn tick() {
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            let win = web_sys::window().expect("window");
            let _ = win
                .set_timeout_with_callback_and_timeout_and_arguments_0(resolve.unchecked_ref(), 0);
        });
        let _ = JsFuture::from(promise).await;
    }

    /// Poll `done` across macrotasks until it holds (or ~1s passes —
    /// the assertion after the wait then reports the real failure).
    /// Module evaluation is async by spec with no completion event on
    /// inline scripts, so tests wait on its effect, not its timing.
    async fn settle_until(done: impl Fn() -> bool) {
        for _ in 0..200 {
            if done() {
                // One more hop so work queued alongside the observed
                // effect (e.g. sibling modules) also lands.
                tick().await;
                return;
            }
            tick().await;
        }
    }

    fn mount_with_module(source: &str) -> Element {
        register();
        let host = document()
            .create_element("tonk-component")
            .expect("create tonk-component");
        let _ = host.set_attribute("module", source);
        document()
            .body()
            .expect("body")
            .append_child(&host)
            .expect("attach");
        host
    }

    fn global_marker(name: &str) -> f64 {
        js_sys::Reflect::get(&js_sys::global(), &name.into())
            .ok()
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
    }

    #[dialog_common::test]
    async fn it_executes_a_module_attribute_once_per_realm() {
        let source =
            "globalThis.__tonkComponentProbeA = (globalThis.__tonkComponentProbeA || 0) + 1;";
        let first = mount_with_module(source);
        let second = mount_with_module(source);
        settle_until(|| global_marker("__tonkComponentProbeA") >= 1.0).await;
        assert_eq!(
            global_marker("__tonkComponentProbeA"),
            1.0,
            "the same source mounted twice must execute once",
        );
        assert_eq!(first.get_attribute("data-state").as_deref(), Some("loaded"));
        assert_eq!(
            second.get_attribute("data-state").as_deref(),
            Some("loaded"),
        );
    }

    #[dialog_common::test]
    async fn it_executes_an_inert_child_script_holder() {
        register();
        let host = document()
            .create_element("tonk-component")
            .expect("create tonk-component");
        // Built the way a view template delivers it: parsed from
        // markup into an inert child script, never executed by the
        // parser itself.
        host.set_inner_html(
            "<script type=\"tonk/module\">globalThis.__tonkComponentProbeB = (globalThis.__tonkComponentProbeB || 0) + 1;</script>",
        );
        document()
            .body()
            .expect("body")
            .append_child(&host)
            .expect("attach");
        settle_until(|| global_marker("__tonkComponentProbeB") >= 1.0).await;
        assert_eq!(
            global_marker("__tonkComponentProbeB"),
            1.0,
            "child holder source should execute exactly once",
        );
    }

    #[dialog_common::test]
    async fn it_can_define_a_custom_element_visible_to_the_whole_realm() {
        let source = "customElements.get('probe-defined-by-component') || customElements.define('probe-defined-by-component', class extends HTMLElement { connectedCallback() { this.textContent = 'defined'; } });";
        mount_with_module(source);
        settle_until(|| {
            !window()
                .expect("window")
                .custom_elements()
                .get("probe-defined-by-component")
                .is_undefined()
        })
        .await;
        let probe = document()
            .create_element("probe-defined-by-component")
            .expect("create probe element");
        document()
            .body()
            .expect("body")
            .append_child(&probe)
            .expect("attach probe");
        assert_eq!(
            probe.text_content().as_deref(),
            Some("defined"),
            "an element defined by a component module should upgrade anywhere in the realm",
        );
    }

    // The end-to-end authoring path: a component written inside a
    // view's `display` template. The template goes through the full
    // inert pipeline (innerHTML → snapshot → clone) — which never
    // executes scripts — and the mounted `<tonk-component>` clone is
    // what lifts the holder's source into a real executing module.
    #[dialog_common::test]
    async fn it_executes_a_component_authored_inside_a_view_template() {
        register();
        crate::view::register();
        let host = document()
            .create_element("tonk-view")
            .expect("create tonk-view");
        host.set_inner_html(
            "<div><p>{name}</p><tonk-component><script type=\"tonk/module\">globalThis.__tonkComponentProbeView = (globalThis.__tonkComponentProbeView || 0) + 1;</script></tonk-component></div>",
        );
        document()
            .body()
            .expect("body")
            .append_child(&host)
            .expect("attach");
        // Drive the view the way `<tonk-display>` would.
        let draw = js_sys::Reflect::get(host.as_ref(), &"draw".into()).expect("draw");
        let draw: js_sys::Function = draw.dyn_into().expect("draw is a function");
        let detail = serde_wasm_bindgen::to_value(&tonk_schema::conclusion::Conclusion {
            this: "did:key:zProbe".to_owned(),
            fields: [(
                "name".to_owned(),
                ipld_core::ipld::Ipld::String("Ada".to_owned()),
            )]
            .into_iter()
            .collect(),
        })
        .expect("serialize conclusion");
        draw.call1(&wasm_bindgen::JsValue::NULL, &detail)
            .expect("call draw");
        settle_until(|| global_marker("__tonkComponentProbeView") >= 1.0).await;
        assert_eq!(
            global_marker("__tonkComponentProbeView"),
            1.0,
            "a component authored in a view display should execute once when the view renders",
        );
        assert!(
            host.inner_html().contains("Ada"),
            "the view's own bindings should still render: {}",
            host.inner_html(),
        );
    }

    #[dialog_common::test]
    async fn it_marks_an_empty_host_and_executes_nothing() {
        register();
        let host = document()
            .create_element("tonk-component")
            .expect("create tonk-component");
        document()
            .body()
            .expect("body")
            .append_child(&host)
            .expect("attach");
        tick().await;
        assert_eq!(host.get_attribute("data-state").as_deref(), Some("empty"));
    }

    #[dialog_common::test]
    async fn it_loads_a_changed_module_attribute() {
        let host = mount_with_module(
            "globalThis.__tonkComponentProbeC = (globalThis.__tonkComponentProbeC || 0) + 1;",
        );
        settle_until(|| global_marker("__tonkComponentProbeC") >= 1.0).await;
        let _ = host.set_attribute(
            "module",
            "globalThis.__tonkComponentProbeD = (globalThis.__tonkComponentProbeD || 0) + 1;",
        );
        settle_until(|| global_marker("__tonkComponentProbeD") >= 1.0).await;
        assert_eq!(global_marker("__tonkComponentProbeC"), 1.0);
        assert_eq!(
            global_marker("__tonkComponentProbeD"),
            1.0,
            "a re-pointed module attribute should execute the new source",
        );
    }
}
