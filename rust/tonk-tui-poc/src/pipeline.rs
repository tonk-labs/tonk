//! The real view pipeline, up to the `Vec<Node>` seam.
//!
//! Every call here is one `tonk render` also makes. Nothing in this
//! module is TUI-specific, which is the claim under test: a template
//! written in a terminal vocabulary parses, plans and resolves through
//! the existing HTML pipeline unchanged, because `html5gum` does not
//! care what a tag is called and `tonk-template` is DOM-free.

use std::collections::{BTreeMap, BTreeSet};

use ipld_core::ipld::Ipld;
use tonk_render::{Conclusion, Node};

/// Parse, collect bindings, plan, and render `template` against
/// `conclusions`, returning the resolved node tree.
///
/// Each conclusion gains a synthetic `dom.notation/source` field: its
/// own notation rendering. It is provided the same way `dom.host/*`
/// fields are, so `<notation>{dom.notation/source}</notation>` needs no
/// pipeline support of its own — it is ordinary interpolation, and it
/// is what lets `tonk show` be a view instead of a special case.
pub fn resolve(template: &str, conclusions: &[Conclusion], head: &str) -> Vec<Node> {
    let conclusions: Vec<Conclusion> = conclusions
        .iter()
        .map(|conclusion| {
            let mut enriched = conclusion.clone();
            enriched.fields.insert(
                "dom.notation/source".to_string(),
                Ipld::String(crate::notation::source(conclusion, head)),
            );
            enriched
        })
        .collect();
    let conclusions = &conclusions[..];
    let mut roots = tonk_render::parse_fragment(template);
    let bindings = tonk_render::collect_bindings(&mut roots);
    let repeat_root = tonk_template::this_repeat_root(&bindings);
    let scalars = scalar_fields(conclusions);
    let plan = tonk_template::split_plan_with_scalars(bindings, repeat_root, &scalars);
    tonk_render::render_nodes(&roots, &plan, conclusions)
}

/// Which fields are `cardinality: one`.
///
/// A real host reads this from the model's descriptor. With no branch
/// to ask, infer it: a field that is never a list is a scalar, and a
/// scalar hole substitutes once instead of becoming an iteration axis.
fn scalar_fields(conclusions: &[Conclusion]) -> BTreeSet<String> {
    let mut listed = BTreeSet::new();
    let mut seen = BTreeSet::new();
    for conclusion in conclusions {
        for (field, value) in &conclusion.fields {
            seen.insert(field.clone());
            if matches!(value, Ipld::List(_)) {
                listed.insert(field.clone());
            }
        }
    }
    seen.difference(&listed).cloned().collect()
}

/// Read `[{"this": "...", "fields": {...}}, ...]` into conclusions.
pub fn conclusions_from_json(json: &str) -> Result<Vec<Conclusion>, String> {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(json).map_err(|error| format!("parsing data: {error}"))?;
    rows.into_iter()
        .map(|row| {
            let this = row
                .get("this")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string();
            let fields = row
                .get("fields")
                .and_then(|value| value.as_object())
                .map(|map| {
                    map.iter()
                        .map(|(key, value)| (key.clone(), to_ipld(value)))
                        .collect::<BTreeMap<_, _>>()
                })
                .unwrap_or_default();
            Ok(Conclusion { this, fields })
        })
        .collect()
}

fn to_ipld(value: &serde_json::Value) -> Ipld {
    match value {
        serde_json::Value::Null => Ipld::Null,
        serde_json::Value::Bool(flag) => Ipld::Bool(*flag),
        serde_json::Value::Number(number) => number
            .as_i64()
            .map(|value| Ipld::Integer(i128::from(value)))
            .or_else(|| number.as_f64().map(Ipld::Float))
            .unwrap_or(Ipld::Null),
        serde_json::Value::String(text) => Ipld::String(text.clone()),
        serde_json::Value::Array(items) => Ipld::List(items.iter().map(to_ipld).collect()),
        serde_json::Value::Object(map) => Ipld::Map(
            map.iter()
                .map(|(key, value)| (key.clone(), to_ipld(value)))
                .collect(),
        ),
    }
}
