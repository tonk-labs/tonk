//! On-the-wire shape for query results — a serializable projection of
//! [`ConceptConclusion`] that workers emit and browser clients
//! deserialize.
//!
//! [`Conclusion`] and [`Frame`] are engine-free data types and live in
//! `tonk-worker-api` so the page can name them without linking the
//! datalog engine; they are re-exported here at their historical
//! `tonk_core::conclusion::*` paths. Only [`project`] — which reads a
//! raw engine [`ConceptConclusion`] — needs the engine, so it stays
//! here as a free function.

use std::collections::BTreeMap;

use dialog_artifacts::Value;
use dialog_query::{Any, ConceptConclusion, Parameters, Term};
use ipld_core::ipld::Ipld;
use ipld_core::serde::to_ipld;

pub use tonk_worker_api::{Conclusion, Frame};

/// Project a [`ConceptConclusion`] using the query's `terms` to
/// discover which field names were bound.
///
/// Variable terms read their value from the match's bindings; constant
/// terms emit the constant directly. Constants matter for filter
/// use-cases — `terms.name = "Alice"` means the caller already knows
/// the value and the engine won't bind a variable for it; without
/// surfacing the constant here, `fields["name"]` would be missing for
/// every row even though every row's `name` is, by construction,
/// `"Alice"`.
pub fn project(conclusion: &ConceptConclusion, terms: &Parameters) -> Conclusion {
    let mut fields = BTreeMap::new();
    for (name, term) in terms.iter() {
        let value = match term {
            Term::Constant(value) => value_to_ipld(value),
            // `lookup` yields a `Binding`: a `Present` value, or
            // `Absent` for an optional field this entity lacks.
            // Absent fields are simply omitted from the wire
            // projection.
            Term::Variable { .. } => conclusion
                .source()
                .lookup(&Term::<Any>::var(name.clone()))
                .ok()
                .and_then(|binding| binding.as_value().and_then(value_to_ipld)),
        };
        if let Some(value) = value {
            fields.insert(name.clone(), value);
        }
    }
    // A keyed-collection entry arrives as two terms, the field and its
    // key operand (`block`, `block/key`). On the wire the field holds
    // the entry as a one-key map, `{key: value}`, so the fold across
    // rows merges entries by key and a sequence's map comes out in
    // position order.
    let keys: Vec<String> = fields
        .keys()
        .filter(|name| name.ends_with("/key"))
        .cloned()
        .collect();
    for key_operand in keys {
        let field = key_operand.trim_end_matches("/key").to_owned();
        let Some(Ipld::String(key)) = fields.remove(&key_operand) else {
            continue;
        };
        if let Some(value) = fields.remove(&field) {
            fields.insert(field, Ipld::Map(BTreeMap::from([(key, value)])));
        }
    }
    Conclusion {
        this: conclusion.entity().to_string(),
        fields,
    }
}

/// Convert a [`Value`] into [`Ipld`] for the wire projection.
///
/// `ipld_core`'s serde serializer rejects `u128` outright, so the
/// derived [`Value`] serialization drops every
/// [`Value::UnsignedInt`] field on the floor (it serializes as
/// `serialize_u128`, which errors). Map unsigned integers that fit
/// in `i128` to [`Ipld::Integer`] directly; values past `i128::MAX`
/// fall back to a decimal string so they survive the hop rather
/// than vanishing. Every other variant routes through the derived
/// serialization, which handles them.
fn value_to_ipld(value: &Value) -> Option<Ipld> {
    match value {
        Value::UnsignedInt(u) => Some(match i128::try_from(*u) {
            Ok(i) => Ipld::Integer(i),
            Err(_) => Ipld::String(u.to_string()),
        }),
        other => to_ipld(other).ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    /// An unsigned-integer field (the column `width`) must survive
    /// projection. `ipld_core`'s serde path errors on `u128`, so
    /// the derived `Value` serialization silently drops it — which
    /// is why a seeded `width` never reached the board renderer.
    #[dialog_common::test]
    fn it_projects_an_unsigned_integer_value() {
        assert_eq!(
            value_to_ipld(&Value::UnsignedInt(40)),
            Some(Ipld::Integer(40)),
        );
    }

    /// A signed integer round-trips through the derived path
    /// unchanged — the regression was unsigned-only.
    #[dialog_common::test]
    fn it_projects_a_signed_integer_value() {
        assert_eq!(
            value_to_ipld(&Value::SignedInt(-7)),
            Some(Ipld::Integer(-7)),
        );
    }
}
