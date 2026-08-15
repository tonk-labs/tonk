//! Built-in resolver registry.
//!
//! dialog-query ships a fixed set of *resolvers* — moded premises the
//! evaluation environment answers by content address rather than by
//! scanning the mutable head. The `tree/*` family is the first of them:
//! `tree/node`, `tree/span`, `tree/key`, `tree/entry`, `tree/value`,
//! `tree/blob`, and `tree/manifest` describe the search tree's own
//! structure, so a document can inspect the store it queries.
//!
//! They are the replacement for tonk's hand-rolled `tree/*` formulas,
//! which the worker intercepted before the planner ever saw them (and
//! which therefore could not be subscribed to, joined against, or used
//! in a rule). As ordinary premises they compose like anything else.
//!
//! Each entry pairs the formal name (the string [`ResolverQuery`]
//! serializes under) with its operand names, read off the resolver's
//! own schema — so the operands the analyzer validates against cannot
//! drift from what the resolver actually deserializes from. This
//! mirrors [`super::constraint`], which does the same for constraints.
//!
//! The registry powers two callers:
//!
//! - [`super::rule::lift_premise`] — recognises a resolver name in
//!   premise head position and validates its `where:` operands.
//! - LSP completion — [`resolver_completions`] enumerates every
//!   resolver for `assert:`-position suggestions.

use std::sync::OnceLock;

use dialog_query::ResolverQuery;

/// One operand of a resolver, as dialog's own schema describes it.
#[derive(Clone)]
pub struct ResolverOperand {
    /// The operand name, written as a key under the premise's `where:`.
    pub name: String,
    /// Dialog's one-line description of what this operand carries.
    pub description: String,
    /// Whether the operand is a *required input* — a term that must
    /// already be bound when the premise runs, rather than one the
    /// resolver produces. `of` is required on every `tree/*` resolver:
    /// they select by content address, so there is nothing to scan.
    pub required: bool,
}

/// One built-in resolver: its formal name plus its operands.
pub(crate) struct ResolverInfo {
    /// The formal name — the string [`ResolverQuery`] serializes under
    /// and the name a premise's `assert:` must carry (e.g. `tree/node`).
    pub name: &'static str,
    /// The resolver's operands, read off the dialog type's schema.
    pub operands: Vec<ResolverOperand>,
}

impl ResolverInfo {
    /// Operand names in this resolver's schema, unordered.
    pub fn operands(&self) -> impl Iterator<Item = &str> {
        self.operands.iter().map(|o| o.name.as_str())
    }
}

/// The operands of a built-in resolver, for completing keys under a
/// resolver premise's `where:`. `None` for names that aren't resolvers.
///
/// Sorted with required inputs first, then alphabetically — the
/// required term is the one a document cannot omit, so it leads.
pub fn resolver_operands(name: &str) -> Option<Vec<ResolverOperand>> {
    let mut operands = lookup_resolver(name)?.operands.clone();
    operands.sort_by(|a, b| {
        b.required
            .cmp(&a.required)
            .then_with(|| a.name.cmp(&b.name))
    });
    Some(operands)
}

/// Look up a built-in resolver by its formal name. Returns `None` for
/// names that aren't resolvers — the caller then falls back to concept
/// resolution.
pub(crate) fn lookup_resolver(name: &str) -> Option<&'static ResolverInfo> {
    registry().iter().find(|r| r.name == name)
}

/// A built-in resolver surfaced for LSP completion: its name and a
/// one-line operand summary suitable for a documentation tooltip.
#[derive(Clone)]
pub struct ResolverCompletion {
    /// The formal name the user types after `assert:`.
    pub name: &'static str,
    /// Human-readable operand summary.
    pub detail: String,
}

/// Every built-in resolver, as completion candidates for an
/// `assert:`-position request. Sorted by name for stable presentation.
pub fn resolver_completions() -> Vec<ResolverCompletion> {
    let mut out: Vec<ResolverCompletion> = registry()
        .iter()
        .map(|r| {
            let mut operands: Vec<&str> = r.operands().collect();
            operands.sort_unstable();
            ResolverCompletion {
                name: r.name,
                detail: format!("{} — operands: {}", r.name, operands.join(", ")),
            }
        })
        .collect();
    out.sort_unstable_by_key(|r| r.name);
    out
}

/// Every built-in resolver, name + operands.
pub(crate) fn registry() -> &'static [ResolverInfo] {
    static REGISTRY: OnceLock<Vec<ResolverInfo>> = OnceLock::new();
    REGISTRY.get_or_init(build_registry).as_slice()
}

fn build_registry() -> Vec<ResolverInfo> {
    // Materialise one resolver per variant with default (blank) terms
    // and read its name and operand names off the live schema. Going
    // through serde rather than naming the variants keeps this in
    // lockstep with the wire format: a resolver dialog renames stops
    // deserializing here, loudly, instead of drifting.
    [
        "tree/node",
        "tree/span",
        "tree/key",
        "tree/entry",
        "tree/value",
        "tree/blob",
        "tree/manifest",
    ]
    .into_iter()
    .map(|name| {
        let query: ResolverQuery =
            serde_json::from_value(serde_json::json!({ "assert": name, "where": {} }))
                .unwrap_or_else(|e| panic!("dialog resolver {name:?} must deserialize: {e}"));
        let mut operands: Vec<ResolverOperand> = query
            .schema()
            .iter()
            .map(|(name, field)| ResolverOperand {
                name: name.clone(),
                description: field.description().to_owned(),
                required: field.requirement().is_required(),
            })
            .collect();
        operands.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        ResolverInfo {
            name: query.name(),
            operands,
        }
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test_configure!(run_in_browser);

    /// Every `tree/*` resolver is reachable from notation. These are
    /// what replaced the worker-intercepted `tree/*` formulas, so a
    /// name missing here is an inspector query no document can write.
    #[dialog_common::test]
    fn it_exposes_every_tree_resolver() {
        for name in [
            "tree/node",
            "tree/span",
            "tree/key",
            "tree/entry",
            "tree/value",
            "tree/blob",
            "tree/manifest",
        ] {
            let found = lookup_resolver(name)
                .unwrap_or_else(|| panic!("`{name}` must be resolvable as a premise name"));
            assert_eq!(found.name, name);
        }
    }

    /// Operand names come off dialog's own schema. `of` is the node
    /// reference every resolver keys on — the join point that makes
    /// them composable — so its absence would mean the registry read
    /// the wrong type.
    #[dialog_common::test]
    fn it_reads_resolver_operands_off_the_dialog_schema() {
        for name in ["tree/node", "tree/span", "tree/entry"] {
            let found = lookup_resolver(name).expect("registered");
            let operands: Vec<&str> = found.operands().collect();
            assert!(
                operands.contains(&"of"),
                "`{name}` keys on `of`; saw {operands:?}"
            );
        }
    }

    /// Operands carry dialog's own descriptions and requiredness, so
    /// the editor can tell an input that must be bound apart from a
    /// binding the resolver produces. `of` is the required one on
    /// every `tree/*` resolver, and it must sort first.
    #[dialog_common::test]
    fn it_describes_resolver_operands() {
        let operands = resolver_operands("tree/node").expect("`tree/node` is a resolver");

        let of = &operands[0];
        assert_eq!(of.name, "of", "the required input must lead");
        assert!(
            of.required,
            "`of` selects by content address, so it is an input"
        );
        assert!(
            !of.description.is_empty(),
            "operand descriptions come off dialog's schema"
        );

        let kind = operands
            .iter()
            .find(|o| o.name == "kind")
            .expect("`tree/node` reports a node kind");
        assert!(!kind.required, "`kind` is produced, not supplied");
        assert!(!kind.description.is_empty());
    }

    /// Names that are not resolvers have no operands to offer.
    #[dialog_common::test]
    fn it_offers_no_operands_for_non_resolvers() {
        assert!(resolver_operands("person").is_none());
    }

    /// A name that is not a resolver falls through, so the caller can
    /// go on to try concept resolution.
    #[dialog_common::test]
    fn it_declines_names_that_are_not_resolvers() {
        assert!(lookup_resolver("tree/nope").is_none());
        assert!(lookup_resolver("person").is_none());
    }
}
