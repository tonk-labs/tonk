//! Single-row DOM renderer for `<tonk-display>`.
//!
//! Mirrors the diffing strategy of `tonk-concept`'s `Renderer`, but
//! collapsed to one row: there is at most one mounted instance of
//! the cloned template at a time, identified implicitly (no `this`
//! key needed). Two inputs drive it:
//!
//! - [`Renderer::apply`] — a new entity conclusion arrived. Insert
//!   if no row is mounted, otherwise patch the existing row's
//!   bindings whose rendered value changed.
//! - [`Renderer::swap_template`] — the view's `display` text
//!   changed. Drop the mounted DOM, parse the new HTML, rebuild
//!   the binding plan, re-apply the cached conclusion (if any).

use tonk_concept::template::{Binding, BindingKind, BindingPlan, extract_plan, render_segments};
use tonk_schema::conclusion::Conclusion;
use wasm_bindgen::JsCast;
use web_sys::{DocumentFragment, Element, HtmlTemplateElement, Node, window};

/// One mounted row.
struct Row {
    /// Top-level cloned nodes (a template can have multiple roots).
    nodes: Vec<Node>,
    /// Per-binding rendered string from the last applied
    /// conclusion. Used to skip writes when nothing changed.
    last_values: Vec<String>,
}

/// Stateful renderer for one entity.
pub struct Renderer {
    /// Where rows get appended.
    host: Element,
    /// Cloneable template fragment built from the view's `display`
    /// text. Replaced on `swap_template`.
    template: DocumentFragment,
    /// Binding plan for `template`. Rebuilt on `swap_template`.
    plan: BindingPlan,
    /// Currently mounted row (if any).
    row: Option<Row>,
    /// Last conclusion successfully applied — replayed when the
    /// template swaps so the new DOM picks up immediately.
    last_conclusion: Option<Conclusion>,
}

impl Renderer {
    /// Construct a renderer from a host element and the view's
    /// initial `display` HTML.
    pub fn new(host: Element, display_html: &str) -> Option<Self> {
        let (template, plan) = build_template(display_html)?;
        Some(Self {
            host,
            template,
            plan,
            row: None,
            last_conclusion: None,
        })
    }

    /// Drop the mounted DOM and rebuild from a new template.
    /// Re-applies the last conclusion if one was cached.
    pub fn swap_template(&mut self, display_html: &str) -> bool {
        let Some((template, plan)) = build_template(display_html) else {
            return false;
        };
        self.clear();
        self.template = template;
        self.plan = plan;
        if let Some(conclusion) = self.last_conclusion.clone() {
            self.apply(&conclusion);
        }
        true
    }

    /// Apply an entity conclusion: insert if no row, else update
    /// in place. The conclusion is cached so a subsequent
    /// [`Renderer::swap_template`] can replay it without waiting
    /// for the next entity frame.
    pub fn apply(&mut self, conclusion: &Conclusion) {
        self.last_conclusion = Some(conclusion.clone());
        if self.row.is_some() {
            self.update_row(conclusion);
        } else {
            self.insert_row(conclusion);
        }
    }

    /// Remove any mounted DOM. Keeps the cached conclusion so
    /// re-`apply`ing after a `swap_template` Just Works.
    pub fn clear(&mut self) {
        if let Some(row) = self.row.take() {
            for n in row.nodes {
                if let Some(parent) = n.parent_node() {
                    let _ = parent.remove_child(&n);
                }
            }
        }
    }

    fn insert_row(&mut self, conclusion: &Conclusion) {
        let Some(clone) = self
            .template
            .clone_node_with_deep(true)
            .ok()
            .and_then(|n| n.dyn_into::<DocumentFragment>().ok())
        else {
            return;
        };

        let mut values: Vec<String> = Vec::with_capacity(self.plan.bindings.len());
        for binding in &self.plan.bindings {
            let rendered = render_binding(binding, conclusion);
            apply_binding(&clone, binding, &rendered);
            values.push(rendered);
        }

        let mut nodes: Vec<Node> = Vec::new();
        let children = clone.child_nodes();
        for i in 0..children.length() {
            if let Some(n) = children.item(i) {
                nodes.push(n);
            }
        }
        if let Some(first) = nodes.first().and_then(|n| n.dyn_ref::<Element>()) {
            let _ = first.set_attribute("data-this", &conclusion.this);
        }

        let _ = self.host.append_child(&clone);
        self.row = Some(Row {
            nodes,
            last_values: values,
        });
    }

    fn update_row(&mut self, conclusion: &Conclusion) {
        let Some(row) = self.row.as_mut() else {
            return;
        };
        for (i, binding) in self.plan.bindings.iter().enumerate() {
            let rendered = render_binding(binding, conclusion);
            if let Some(prev) = row.last_values.get(i)
                && *prev == rendered
            {
                continue;
            }
            patch_row(row, binding, &rendered);
            if let Some(slot) = row.last_values.get_mut(i) {
                *slot = rendered;
            }
        }
    }
}

/// Parse `html` into an off-document `DocumentFragment` and extract
/// a binding plan. Returns `None` if there is no `window`.
fn build_template(html: &str) -> Option<(DocumentFragment, BindingPlan)> {
    let document = window()?.document()?;
    let tpl = document.create_element("template").ok()?;
    let tpl: HtmlTemplateElement = tpl.dyn_into().ok()?;
    tpl.set_inner_html(html);
    let fragment = tpl.content();
    let plan = extract_plan(&fragment);
    Some((fragment, plan))
}

fn render_binding(binding: &Binding, conclusion: &Conclusion) -> String {
    let segments = match &binding.kind {
        BindingKind::Text { segments } => segments,
        BindingKind::Attribute { segments, .. } => segments,
    };
    render_segments(segments, &conclusion.this, &conclusion.fields)
}

fn apply_binding(fragment: &DocumentFragment, binding: &Binding, rendered: &str) {
    let root: Node = fragment.clone().into();
    let Some(target) = tonk_concept::template::navigate(&root, &binding.path) else {
        return;
    };
    write_binding(&target, binding, rendered);
}

fn patch_row(row: &Row, binding: &Binding, rendered: &str) {
    let Some(&first) = binding.path.first() else {
        return;
    };
    let Some(root) = row.nodes.get(first) else {
        return;
    };
    let rest = &binding.path[1..];
    let Some(target) = tonk_concept::template::navigate(root, rest) else {
        return;
    };
    write_binding(&target, binding, rendered);
}

fn write_binding(target: &Node, binding: &Binding, rendered: &str) {
    match &binding.kind {
        BindingKind::Text { .. } => {
            target.set_text_content(Some(rendered));
        }
        BindingKind::Attribute { attr_name, .. } => {
            if let Some(el) = target.dyn_ref::<Element>() {
                let _ = el.set_attribute(attr_name, rendered);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_browser);

    fn mount() -> Element {
        let document = window().expect("window").document().expect("document");
        let host: Element = document.create_element("div").expect("create div");
        document
            .body()
            .expect("body")
            .append_child(&host)
            .expect("attach host");
        host
    }

    fn conclusion(this: &str, fields: &[(&str, &str)]) -> Conclusion {
        let mut map: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        for (k, v) in fields {
            map.insert((*k).into(), serde_json::Value::String((*v).into()));
        }
        Conclusion {
            this: this.into(),
            fields: map,
        }
    }

    #[dialog_common::test]
    fn it_renders_a_single_entity_template() {
        let host = mount();
        let mut r =
            Renderer::new(host.clone(), "<p class=\"greeting\">{message}</p>").expect("renderer");
        r.apply(&conclusion("did:key:zG", &[("message", "Hello")]));
        assert!(host.inner_html().contains("Hello"));
        assert_eq!(
            host.query_selector_all("p").unwrap().length(),
            1,
            "exactly one paragraph",
        );
    }

    #[dialog_common::test]
    fn it_updates_fields_in_place_on_state_frame() {
        let host = mount();
        let mut r = Renderer::new(host.clone(), "<p>{message}</p>").expect("renderer");
        r.apply(&conclusion("did:key:zG", &[("message", "Hello")]));
        let p_before = host.query_selector("p").unwrap().expect("p");
        r.apply(&conclusion("did:key:zG", &[("message", "Hi")]));
        let p_after = host.query_selector("p").unwrap().expect("p");
        assert!(p_before.is_same_node(Some(p_after.unchecked_ref())));
        assert!(host.inner_html().contains("Hi"));
    }

    #[dialog_common::test]
    fn it_swaps_dom_wholesale_on_template_frame() {
        let host = mount();
        let mut r = Renderer::new(host.clone(), "<p>{message}</p>").expect("renderer");
        r.apply(&conclusion("did:key:zG", &[("message", "Hello")]));
        assert!(host.query_selector("p").unwrap().is_some());

        let swapped = r.swap_template("<h1>{message}</h1>");
        assert!(swapped, "swap_template should succeed");
        assert!(
            host.query_selector("p").unwrap().is_none(),
            "old <p> should be gone",
        );
        assert!(
            host.query_selector("h1").unwrap().is_some(),
            "new <h1> should be mounted with cached conclusion",
        );
        assert!(host.inner_html().contains("Hello"));
    }

    #[dialog_common::test]
    fn it_dedupes_writes_when_field_value_unchanged() {
        let host = mount();
        let mut r = Renderer::new(host.clone(), "<p>{message}</p>").expect("renderer");
        r.apply(&conclusion("did:key:zG", &[("message", "Hello")]));
        let p = host.query_selector("p").unwrap().expect("p");
        let text_node_before = p.first_child().expect("text node");
        r.apply(&conclusion("did:key:zG", &[("message", "Hello")]));
        let p_again = host.query_selector("p").unwrap().expect("p");
        let text_node_after = p_again.first_child().expect("text node again");
        assert!(text_node_before.is_same_node(Some(text_node_after.unchecked_ref())));
    }

    #[dialog_common::test]
    fn it_clears_dom_on_clear() {
        let host = mount();
        let mut r = Renderer::new(host.clone(), "<p>{message}</p>").expect("renderer");
        r.apply(&conclusion("did:key:zG", &[("message", "Hello")]));
        r.clear();
        assert!(host.query_selector("p").unwrap().is_none());
    }
}
