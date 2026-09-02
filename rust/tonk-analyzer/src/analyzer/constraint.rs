//! Built-in constraint registry.
//!
//! dialog-query ships a fixed set of pure variable constraints
//! (equality, the range predicates, and prefix matching) behind its
//! `Constraint` enum.
//! Like formulas they aren't concepts: they don't live on the
//! branch and they filter/infer bindings rather than querying
//! stored facts. They're also a *separate* enum from `FormulaQuery`,
//! so the [`super::formula`] registry doesn't cover them — the
//! analyzer consults this one instead.
//!
//! Each entry pairs the formal name (the string `Constraint`
//! serializes under) with its operand names. Unlike a formula —
//! whose operand schema is a `&'static Cells` exposed by
//! [`Formula::cells`] — a constraint's operands come from a
//! `&self` `schema()` method, so the registry materialises one
//! instance per constraint at build time and reads its operand
//! names off the live schema. Deriving the names from dialog's own
//! type (rather than hardcoding them) means they can't drift from
//! what the constraint actually deserializes from.
//!
//! The registry powers two callers:
//!
//! - [`super::rule::lift_premise`] — recognises a constraint name in
//!   premise head position and validates its `where:` operands.
//! - LSP completion — [`constraint_completions`] enumerates every
//!   constraint for `assert:`-position suggestions, alongside
//!   formulas.

use std::sync::OnceLock;

use dialog_query::constraint::{
    AtLeast, AtMost, Coalesce, Equality, GreaterThan, LessThan, StartsWith,
};
use dialog_query::{Any, Term};

/// One built-in constraint: its formal name plus its operand names.
pub(crate) struct ConstraintInfo {
    /// The formal name — the string `Constraint` serializes under
    /// and the name a premise's `assert:` must carry (e.g. `==`).
    pub name: &'static str,
    /// The constraint's operand names, read off the dialog type's
    /// schema. All operands are required: a constraint relates the
    /// terms it's given, with nothing to auto-fill (unlike a
    /// formula's `#[output]` slots).
    pub operands: Vec<String>,
}

impl ConstraintInfo {
    /// Operand names in this constraint's schema, unordered.
    pub fn operands(&self) -> impl Iterator<Item = &str> {
        self.operands.iter().map(String::as_str)
    }
}

/// Look up a built-in constraint by its formal name. Returns `None`
/// for names that aren't constraints — the caller then falls back
/// to concept resolution.
pub(crate) fn lookup_constraint(name: &str) -> Option<&'static ConstraintInfo> {
    registry().iter().find(|c| c.name == name)
}

/// A built-in constraint surfaced for LSP completion: its name and
/// a one-line operand summary suitable for a documentation tooltip.
#[derive(Clone)]
pub struct ConstraintCompletion {
    /// The formal name the user types after `assert:`.
    pub name: &'static str,
    /// The name as it must appear in notation — quoted when the bare
    /// form is not a plain YAML scalar. See [`notation_form`].
    pub insert: String,
    /// Human-readable operand summary, e.g. `== — operands: is, this`.
    pub detail: String,
}

/// A constraint name as it must be written in notation.
///
/// `>` opens a folded scalar in YAML and `>=` is not a plain scalar
/// either, so both have to be quoted: an unquoted `assert: >` parses
/// as an EMPTY name and surfaces as "unknown concept" — a diagnostic
/// that says nothing about quoting. Completion inserts the quoted form
/// so the trap is unreachable from the editor.
pub fn notation_form(name: &str) -> String {
    if name.starts_with('>') {
        format!("\"{name}\"")
    } else {
        name.to_owned()
    }
}

/// Every built-in constraint, as completion candidates for an
/// `assert:`-position request. Sorted by name for stable
/// presentation.
pub fn constraint_completions() -> Vec<ConstraintCompletion> {
    let mut out: Vec<ConstraintCompletion> = registry()
        .iter()
        .map(|c| {
            let mut operands: Vec<&str> = c.operands().collect();
            operands.sort_unstable();
            ConstraintCompletion {
                name: c.name,
                insert: notation_form(c.name),
                detail: format!("{} — operands: {}", c.name, operands.join(", ")),
            }
        })
        .collect();
    out.sort_unstable_by_key(|c| c.name);
    out
}

/// Every built-in constraint, name + operands. The name/operand
/// pairing mirrors dialog-query's `Constraint` enum; the operand
/// names are read off each constraint type's own schema so they
/// stay in lockstep with the wire format.
pub(crate) fn registry() -> &'static [ConstraintInfo] {
    static REGISTRY: OnceLock<Vec<ConstraintInfo>> = OnceLock::new();
    REGISTRY.get_or_init(build_registry).as_slice()
}

fn build_registry() -> Vec<ConstraintInfo> {
    // Materialise an instance per constraint and read its operand
    // names off the live schema. The terms are throwaway — only the
    // schema's field names are used, so the registry cannot drift
    // from what each constraint actually deserializes from.
    fn operands_of(schema: dialog_query::Schema) -> Vec<String> {
        let mut operands: Vec<String> = schema.iter().map(|(k, _)| k.clone()).collect();
        operands.sort_unstable();
        operands
    }
    let term = Term::<Any>::unique;

    vec![
        ConstraintInfo {
            name: "==",
            operands: operands_of(Equality::new(term(), term()).schema()),
        },
        // The range predicates compare `of` against `with`, in that
        // order: `{ of: ?age, with: 30 }` under `<` reads `?age < 30`.
        // A constant side additionally proves an interval bound on the
        // other, which dialog pushes into the value scan as an index
        // range — so these narrow what is read, not just what is kept.
        ConstraintInfo {
            name: "<",
            operands: operands_of(LessThan::new(term(), term()).schema()),
        },
        ConstraintInfo {
            name: "<=",
            operands: operands_of(AtMost::new(term(), term()).schema()),
        },
        ConstraintInfo {
            name: ">",
            operands: operands_of(GreaterThan::new(term(), term()).schema()),
        },
        ConstraintInfo {
            name: ">=",
            operands: operands_of(AtLeast::new(term(), term()).schema()),
        },
        ConstraintInfo {
            name: "starts-with",
            operands: operands_of(StartsWith::new(term(), Term::<String>::unique()).schema()),
        },
        // `is` takes `source` when it is present and `fallback` when it is
        // absent — the only way notation can turn a `maybe:` field into a
        // bound one. Without it a rule needing an optional bound has to be
        // written once per present/absent combination, because a rule body is
        // a conjunction and a premise cannot be skipped. The constraint has
        // always existed on the wire (`{assert: coalesce, where: {...}}`) and
        // `DeductiveRule::new` validates it however it was built; only this
        // registry was withholding the name.
        ConstraintInfo {
            name: "coalesce",
            operands: operands_of(Coalesce::new(term(), term(), term()).schema()),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test_configure!(run_in_browser);

    /// Every constraint dialog serializes is reachable from notation.
    ///
    /// The registry is what the analyzer resolves premise names
    /// against, so a name missing here is a predicate no document can
    /// use — which is exactly how the range predicates went unexposed
    /// after dialog added them.
    #[dialog_common::test]
    fn it_exposes_every_range_predicate() {
        for name in ["==", "<", "<=", ">", ">=", "starts-with"] {
            let found = lookup_constraint(name)
                .unwrap_or_else(|| panic!("`{name}` must be resolvable as a premise name"));
            assert_eq!(found.name, name);
        }
    }

    /// The range predicates compare `of` against `with`, and the
    /// operand names come off dialog's own schema — so this fails if
    /// dialog renames a slot, rather than drifting silently.
    #[dialog_common::test]
    fn it_reads_range_operands_off_the_dialog_schema() {
        for name in ["<", "<=", ">", ">="] {
            let found = lookup_constraint(name).expect("registered");
            let mut operands: Vec<&str> = found.operands().collect();
            operands.sort_unstable();
            assert_eq!(
                operands,
                vec!["of", "with"],
                "`{name}` compares `of` against `with`"
            );
        }
    }

    /// Completion inserts a notation-safe name. `>` and `>=` open a
    /// YAML folded scalar unquoted, so inserting the bare name would
    /// produce a document that parses as an empty predicate name.
    #[dialog_common::test]
    fn it_quotes_the_yaml_hostile_names_for_insertion() {
        let by_name = |name: &str| {
            constraint_completions()
                .into_iter()
                .find(|c| c.name == name)
                .unwrap_or_else(|| panic!("`{name}` must be offered as a completion"))
        };

        assert_eq!(by_name(">").insert, "\">\"");
        assert_eq!(by_name(">=").insert, "\">=\"");
        // `<` has no YAML meaning, so it stays bare.
        assert_eq!(by_name("<").insert, "<");
        assert_eq!(by_name("==").insert, "==");
    }
}
