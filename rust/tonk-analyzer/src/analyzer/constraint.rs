//! Built-in constraint registry.
//!
//! dialog-query ships a fixed set of pure variable constraints
//! (currently just `==`, equality) behind its `Constraint` enum.
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

use dialog_query::constraint::Equality;
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
    /// Human-readable operand summary, e.g. `== — operands: is, this`.
    pub detail: String,
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
    // schema's field names are used.
    let equality = Equality::new(Term::<Any>::unique(), Term::<Any>::unique());
    let mut operands: Vec<String> = equality.schema().iter().map(|(k, _)| k.clone()).collect();
    operands.sort_unstable();

    vec![ConstraintInfo {
        name: "==",
        operands,
    }]
}
