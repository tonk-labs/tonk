//! Fold a multi-row conclusion frame into a single conclusion
//! with array-valued fields where the rows disagree.
//!
//! The dialog query engine emits one row per tuple — for a
//! cardinality-many attribute on a single entity, each value
//! comes back as its own [`Conclusion`] sharing the same `this`
//! and differing only in the projected field. `<tonk-display>`
//! is a *single-entity* element, so it needs to collapse those
//! rows into one conclusion the template renderer can iterate
//! over.
//!
//! The fold preserves order: distinct values for the same field
//! appear in the array in the order they first appeared across
//! the rows. Identical values across all rows are kept as a
//! scalar (no needless array wrapping).
//!
//! Target-independent so the fold logic can be unit-tested
//! natively without spinning up the orchestrator.

use std::collections::BTreeMap;

use ipld_core::ipld::Ipld;
use tonk_schema::conclusion::Conclusion;

/// Fold a list of conclusions sharing the same `this` into one.
/// If `conclusions` is empty, returns `None`. If all rows agree
/// on every field, returns the first row unchanged. Differing
/// rows collapse the disagreeing fields into `Array` values
/// preserving first-seen order; identical values are
/// deduplicated.
///
/// Mixed-`this` input is **not** an error — the function uses
/// the first row's `this` and folds across every row. Callers
/// who care about identity should group upstream; in practice
/// `<tonk-display>` only ever feeds rows for a single entity.
pub fn fold_rows(conclusions: Vec<Conclusion>) -> Option<Conclusion> {
    let mut iter = conclusions.into_iter();
    let first = iter.next()?;

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

    Some(Conclusion {
        this: first.this,
        fields: folded,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipld_core::serde::to_ipld;
    use serde_json::{Value, json};

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

    #[test]
    fn it_returns_none_for_empty_input() {
        assert!(fold_rows(vec![]).is_none());
    }

    #[test]
    fn it_returns_a_single_row_unchanged() {
        let c = conclusion("did:key:zX", &[("name", json!("Alice"))]);
        let folded = fold_rows(vec![c.clone()]).expect("single row folds");
        assert_eq!(folded.this, "did:key:zX");
        assert_eq!(folded.fields.get("name"), Some(&ipld(json!("Alice"))));
    }

    #[test]
    fn it_keeps_scalar_when_every_row_agrees() {
        let rows = vec![
            conclusion("did:key:zX", &[("name", json!("Alice"))]),
            conclusion("did:key:zX", &[("name", json!("Alice"))]),
        ];
        let folded = fold_rows(rows).expect("rows fold");
        // Both agree on `name` — should not become an array.
        assert_eq!(folded.fields.get("name"), Some(&ipld(json!("Alice"))));
    }

    #[test]
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
        let folded = fold_rows(rows).expect("rows fold");
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

    #[test]
    fn it_preserves_first_seen_order_for_differing_values() {
        let rows = vec![
            conclusion("did:key:zX", &[("tag", json!("zebra"))]),
            conclusion("did:key:zX", &[("tag", json!("apple"))]),
            conclusion("did:key:zX", &[("tag", json!("mango"))]),
        ];
        let folded = fold_rows(rows).expect("rows fold");
        assert_eq!(
            folded.fields.get("tag"),
            Some(&ipld(json!(["zebra", "apple", "mango"]))),
        );
    }

    #[test]
    fn it_deduplicates_repeated_values() {
        let rows = vec![
            conclusion("did:key:zX", &[("tag", json!("alpha"))]),
            conclusion("did:key:zX", &[("tag", json!("beta"))]),
            conclusion("did:key:zX", &[("tag", json!("alpha"))]),
        ];
        let folded = fold_rows(rows).expect("rows fold");
        // `alpha` appears twice on the wire but is collapsed to
        // one entry in the folded array.
        assert_eq!(
            folded.fields.get("tag"),
            Some(&ipld(json!(["alpha", "beta"])))
        );
    }

    #[test]
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
        let folded = fold_rows(rows).expect("rows fold");
        // `item` saw one value, stays scalar.
        assert_eq!(folded.fields.get("item"), Some(&ipld(json!("did:key:zA"))));
        assert_eq!(folded.fields.get("name"), Some(&ipld(json!("Bag"))));
    }
}
