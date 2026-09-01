//! Project a flat conclusion frame into one conclusion per subject.
//!
//! The dialog query engine emits one row per tuple — for a
//! cardinality-many attribute, each value comes back as its own
//! [`Conclusion`] sharing the same `this` and differing only in the
//! projected field; for a many-instance query, rows span several
//! subjects. [`select_rows`] groups by `this` and folds each group, so
//! `<tonk-display>` always gets a frame of one folded conclusion per
//! subject — cardinality-one is just a one-element frame.
//!
//! The fold preserves order: distinct values for the same field
//! appear in the array in the order they first appeared across the
//! rows. Identical values across all rows are kept as a scalar (no
//! needless array wrapping).
//!
//! Target-independent so the fold logic can be unit-tested natively
//! without spinning up the orchestrator.

use std::collections::BTreeMap;

use ipld_core::ipld::Ipld;
use tonk_schema::conclusion::Conclusion;

/// Read one facet's template out of a folded view conclusion.
///
/// A view-resolution frame folds to at most one conclusion (the model
/// entity) whose `show` field is the facet dictionary
/// (`{facet: template}`). Returns `None` when the facet has no entry
/// — the caller then falls back (the `tonk:_` default, or the
/// notation dump).
pub fn show_template<'a>(conclusion: &'a Conclusion, facet: &str) -> Option<&'a str> {
    match conclusion.fields.get("show")? {
        Ipld::Map(entries) => match entries.get(facet)? {
            Ipld::String(template) => Some(template),
            _ => None,
        },
        _ => None,
    }
}

/// Group a flat conclusion frame by `this` and fold each group,
/// yielding **one conclusion per distinct subject** with its
/// cardinality-many fields collapsed to `Ipld::List`. Groups appear in
/// first-seen `this` order.
///
/// This is the universal projection: the query engine emits one flat
/// row per tuple, so a query returns rows for one or many subjects.
/// Grouping by `this` turns that into the conclusion-per-subject frame
/// the renderer iterates — cardinality-one is just a one-element frame.
/// Models dialog's `select`/`merge`/`add` aggregation
/// (`query/src/selector.js`): `this` is the grouping key, many-fields
/// are the array accumulators.
pub fn select_rows(conclusions: Vec<Conclusion>) -> Vec<Conclusion> {
    let mut order: Vec<String> = Vec::new();
    let mut groups: BTreeMap<String, Vec<Conclusion>> = BTreeMap::new();
    for row in conclusions {
        if !groups.contains_key(&row.this) {
            order.push(row.this.clone());
        }
        groups.entry(row.this.clone()).or_default().push(row);
    }
    order
        .into_iter()
        .filter_map(|this| groups.remove(&this))
        .map(fold_group)
        .collect()
}

/// Fold a non-empty group of conclusions (sharing one `this`, though
/// only the first row's `this` is used) into one conclusion,
/// accumulating distinct per-field values in first-seen order and
/// collapsing multi-valued fields to `Ipld::List`.
fn fold_group(conclusions: Vec<Conclusion>) -> Conclusion {
    let mut iter = conclusions.into_iter();
    let first = iter.next().expect("fold_group requires a non-empty group");

    // For every field, accumulate distinct values in first-seen
    // order. We only need to switch to a `List` representation
    // if more than one distinct value shows up.
    let mut per_field: BTreeMap<String, Vec<Ipld>> = BTreeMap::new();
    for (name, value) in &first.fields {
        per_field.insert(name.clone(), vec![value.clone()]);
    }
    for row in iter {
        for (name, value) in row.fields {
            let bucket = per_field.entry(name).or_default();
            // A keyed-collection field holds one `{key: value}` entry
            // per row; entries merge into one map keyed by the
            // collection's own keys, rather than accumulating a list.
            if let (Some(Ipld::Map(entries)), Ipld::Map(entry)) = (bucket.last_mut(), &value) {
                entries.extend(entry.clone());
                continue;
            }
            if !bucket.iter().any(|existing| existing == &value) {
                bucket.push(value);
            }
        }
    }

    let mut folded: BTreeMap<String, Ipld> = BTreeMap::new();
    for (name, values) in per_field {
        let value = match values.len() {
            0 => Ipld::Null,
            1 => values.into_iter().next().expect("single value present"),
            _ => Ipld::List(values),
        };
        folded.insert(name, value);
    }

    Conclusion {
        this: first.this,
        fields: folded,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipld_core::serde::to_ipld;
    use serde_json::{Value, json};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    fn ipld(value: Value) -> Ipld {
        to_ipld(&value).expect("json value converts to ipld")
    }

    fn conclusion(this: &str, fields: &[(&str, Value)]) -> Conclusion {
        let mut map = BTreeMap::new();
        for (k, v) in fields {
            map.insert((*k).to_owned(), ipld(v.clone()));
        }
        Conclusion {
            this: this.to_owned(),
            fields: map,
        }
    }

    /// Fold a single-subject frame to its one conclusion (the common
    /// shape these tests exercise). `select_rows` of one `this` yields
    /// a one-element frame.
    fn fold_one(rows: Vec<Conclusion>) -> Option<Conclusion> {
        select_rows(rows).into_iter().next()
    }

    /// A view frame folds to the model entity's `show` dictionary;
    /// `show_template` reads one facet's template out of it.
    #[dialog_common::test]
    fn it_reads_a_facet_template_from_a_folded_view() {
        let rows = vec![
            conclusion(
                "tonk:counter",
                &[("show", json!({"ui": "<h1>{count}</h1>"}))],
            ),
            conclusion(
                "tonk:counter",
                &[("show", json!({"title": "Counter {count}"}))],
            ),
        ];
        let folded = fold_one(rows).expect("rows fold");
        assert_eq!(show_template(&folded, "ui"), Some("<h1>{count}</h1>"));
        assert_eq!(show_template(&folded, "title"), Some("Counter {count}"));
        assert_eq!(
            show_template(&folded, "directory"),
            None,
            "an absent facet reads as none, so the caller falls back"
        );
    }

    /// A keyed-collection field arrives as one `{key: value}` entry
    /// per row; the fold merges them into one map keyed by the
    /// collection's own keys, which for positions is list order.
    #[dialog_common::test]
    fn it_merges_collection_entries_by_key() {
        let rows = vec![
            conclusion("did:key:zX", &[("block", json!({"N5": "second"}))]),
            conclusion("did:key:zX", &[("block", json!({"N": "first"}))]),
        ];
        let folded = fold_one(rows).expect("rows fold");
        assert_eq!(
            folded.fields.get("block"),
            Some(&ipld(json!({"N": "first", "N5": "second"}))),
            "entries merge into one map, in key order"
        );
    }

    #[dialog_common::test]
    fn it_returns_none_for_empty_input() {
        assert!(fold_one(vec![]).is_none());
    }

    #[dialog_common::test]
    fn it_returns_a_single_row_unchanged() {
        let c = conclusion("did:key:zX", &[("name", json!("Alice"))]);
        let folded = fold_one(vec![c.clone()]).expect("single row folds");
        assert_eq!(folded.this, "did:key:zX");
        assert_eq!(folded.fields.get("name"), Some(&ipld(json!("Alice"))));
    }

    #[dialog_common::test]
    fn it_keeps_scalar_when_every_row_agrees() {
        let rows = vec![
            conclusion("did:key:zX", &[("name", json!("Alice"))]),
            conclusion("did:key:zX", &[("name", json!("Alice"))]),
        ];
        let folded = fold_one(rows).expect("rows fold");
        // Both agree on `name` — should not become an array.
        assert_eq!(folded.fields.get("name"), Some(&ipld(json!("Alice"))));
    }

    #[dialog_common::test]
    fn it_collects_distinct_values_into_an_array() {
        // The todo-list shape: three rows for one entity, each
        // with a different `item` value.
        let rows = vec![
            conclusion(
                "did:key:zList",
                &[
                    ("name", json!("Groceries")),
                    ("item", json!("did:key:zTodo-1")),
                ],
            ),
            conclusion(
                "did:key:zList",
                &[
                    ("name", json!("Groceries")),
                    ("item", json!("did:key:zTodo-2")),
                ],
            ),
            conclusion(
                "did:key:zList",
                &[
                    ("name", json!("Groceries")),
                    ("item", json!("did:key:zTodo-3")),
                ],
            ),
        ];
        let folded = fold_one(rows).expect("rows fold");
        assert_eq!(folded.fields.get("name"), Some(&ipld(json!("Groceries"))));
        assert_eq!(
            folded.fields.get("item"),
            Some(&ipld(json!([
                "did:key:zTodo-1",
                "did:key:zTodo-2",
                "did:key:zTodo-3",
            ]))),
        );
    }

    #[dialog_common::test]
    fn it_preserves_first_seen_order_for_differing_values() {
        let rows = vec![
            conclusion("did:key:zX", &[("tag", json!("zebra"))]),
            conclusion("did:key:zX", &[("tag", json!("apple"))]),
            conclusion("did:key:zX", &[("tag", json!("mango"))]),
        ];
        let folded = fold_one(rows).expect("rows fold");
        assert_eq!(
            folded.fields.get("tag"),
            Some(&ipld(json!(["zebra", "apple", "mango"]))),
        );
    }

    #[dialog_common::test]
    fn it_deduplicates_repeated_values() {
        let rows = vec![
            conclusion("did:key:zX", &[("tag", json!("alpha"))]),
            conclusion("did:key:zX", &[("tag", json!("beta"))]),
            conclusion("did:key:zX", &[("tag", json!("alpha"))]),
        ];
        let folded = fold_one(rows).expect("rows fold");
        // `alpha` appears twice on the wire but is collapsed to
        // one entry in the folded array.
        assert_eq!(
            folded.fields.get("tag"),
            Some(&ipld(json!(["alpha", "beta"])))
        );
    }

    #[dialog_common::test]
    fn it_handles_a_field_missing_from_later_rows() {
        // Worker output for cardinality-many queries can leave a
        // field unbound on some rows; we just don't append it.
        let rows = vec![
            conclusion(
                "did:key:zX",
                &[("name", json!("Bag")), ("item", json!("did:key:zA"))],
            ),
            conclusion("did:key:zX", &[("name", json!("Bag"))]),
        ];
        let folded = fold_one(rows).expect("rows fold");
        // `item` saw one value, stays scalar.
        assert_eq!(folded.fields.get("item"), Some(&ipld(json!("did:key:zA"))));
        assert_eq!(folded.fields.get("name"), Some(&ipld(json!("Bag"))));
    }
}
