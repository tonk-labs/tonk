//! Single-row DOM renderer used by `<tonk-view>`. Mirrors the
//! diffing strategy of `tonk-concept`'s renderer, but collapsed to
//! one row: there is at most one mounted instance of the cloned
//! template at a time. `<tonk-view>` builds one from its snapshotted
//! children-as-template and feeds conclusions through `apply`.

use tonk_concept::template::{
    Binding, BindingKind, BindingPlan, Snapshot, extract_plan, render_segments,
};
use tonk_schema::conclusion::Conclusion;
use wasm_bindgen::JsCast;
use web_sys::{DocumentFragment, Element, Node};

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
    /// Cloneable template fragment captured from the host's
    /// children at construction time.
    template: DocumentFragment,
    /// Binding plan extracted from `template` at construction time.
    plan: BindingPlan,
    /// Currently mounted row (if any).
    row: Option<Row>,
}

impl Renderer {
    /// Construct a renderer from a pre-snapshotted template +
    /// container pair. Used by `<tonk-view>` after it pulls the
    /// host's children into a `DocumentFragment` at
    /// `connectedCallback`. `snapshot.container` becomes the
    /// renderer's append target.
    pub fn from_snapshot(snapshot: Snapshot) -> Self {
        let plan = extract_plan(&snapshot.fragment);
        Self {
            host: snapshot.container,
            template: snapshot.fragment,
            plan,
            row: None,
        }
    }

    /// Apply an entity conclusion: insert if no row, else update
    /// in place. Per-binding write-deduping inside `update_row`
    /// avoids touching DOM nodes whose rendered value didn't
    /// change since the last frame.
    pub fn apply(&mut self, conclusion: &Conclusion) {
        if self.row.is_some() {
            self.update_row(conclusion);
        } else {
            self.insert_row(conclusion);
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
