//! Built-in formula registry.
//!
//! dialog-query ships a fixed set of computation formulas
//! (`math/sum`, `boolean/and`, `text/concatenate`, …) behind its
//! `FormulaQuery` enum. They aren't concepts: they don't live on
//! the branch and they compute new bindings rather than querying
//! stored facts. The analyzer therefore can't resolve them
//! through the [`Resolver`](super::resolver::Resolver) — it
//! consults this registry instead.
//!
//! Each entry pairs the formal name (the same string
//! `FormulaQuery` serializes under) with the formula's static
//! [`Cells`] schema, taken straight from the formula type's
//! [`Formula::cells`] impl. Keeping the registry derived from
//! dialog-query's own types means the operand names the analyzer
//! validates against can never drift from what the formula
//! actually accepts.
//!
//! The registry powers two callers:
//!
//! - [`super::rule::lift_premise`] — recognises a formula name in
//!   premise head position and validates its `where:` operands.
//! - LSP completion — [`formula_names`] enumerates every formula
//!   for `assert:`-position suggestions.

use std::sync::OnceLock;

use dialog_query::formula::Formula;
use dialog_query::formula::cell::Cells;
use dialog_query::formula::conversions::{
    ParseFloat, ParseSignedInteger, ParseUnsignedInteger, ToString as ToStringFormula,
};
use dialog_query::formula::logic::{And, Not, Or};
use dialog_query::formula::math::{Difference, Modulo, Product, Quotient, Sum};
use dialog_query::formula::string::{Concatenate, Length, Like, Lowercase, Uppercase};

/// One built-in formula: its formal name plus its operand schema.
#[derive(Clone, Copy)]
pub(crate) struct FormulaInfo {
    /// The formal name — the string `FormulaQuery` serializes
    /// under and the name a premise's `assert:` must carry.
    pub name: &'static str,
    /// The formula's operand cells: required inputs and
    /// `#[output]` slots, keyed by operand name.
    pub cells: &'static Cells,
}

impl FormulaInfo {
    /// Operand names in this formula's schema (`of`, `with`,
    /// `is`, …), unordered.
    pub fn operands(&self) -> impl Iterator<Item = &'static str> {
        self.cells.keys()
    }
}

/// Look up a built-in formula by its formal name. Returns `None`
/// for names that aren't formulas — the caller then falls back to
/// concept resolution.
pub(crate) fn lookup_formula(name: &str) -> Option<FormulaInfo> {
    registry().iter().copied().find(|f| f.name == name)
}

/// A built-in formula surfaced for LSP completion: its name and
/// a one-line operand summary suitable for a documentation
/// tooltip.
#[derive(Clone)]
pub struct FormulaCompletion {
    /// The formal name the user types after `assert:`.
    pub name: &'static str,
    /// Human-readable operand summary, e.g.
    /// `math/sum — operands: of, with, is`.
    pub detail: String,
}

/// Every built-in formula, as completion candidates for an
/// `assert:`-position request. Sorted by name for stable
/// presentation.
pub fn formula_completions() -> Vec<FormulaCompletion> {
    let mut out: Vec<FormulaCompletion> = registry()
        .iter()
        .map(|f| {
            let mut operands: Vec<&str> = f.operands().collect();
            operands.sort_unstable();
            FormulaCompletion {
                name: f.name,
                detail: format!("{} — operands: {}", f.name, operands.join(", ")),
            }
        })
        .collect();
    out.sort_unstable_by_key(|c| c.name);
    out
}

/// Every built-in formula, name + schema.
pub(crate) fn registry() -> &'static [FormulaInfo] {
    static REGISTRY: OnceLock<Vec<FormulaInfo>> = OnceLock::new();
    REGISTRY.get_or_init(build_registry).as_slice()
}

/// Build the registry. The name/type pairing mirrors
/// dialog-query's own `define_formulas!` table — both must list
/// the same formulas under the same names.
fn build_registry() -> Vec<FormulaInfo> {
    fn info<F: Formula>(name: &'static str) -> FormulaInfo {
        FormulaInfo {
            name,
            cells: F::cells(),
        }
    }

    vec![
        info::<Sum>("math/sum"),
        info::<Difference>("math/difference"),
        info::<Product>("math/product"),
        info::<Quotient>("math/quotient"),
        info::<Modulo>("math/modulo"),
        info::<Concatenate>("text/concatenate"),
        info::<Length>("text/length"),
        info::<Uppercase>("text/upper-case"),
        info::<Lowercase>("text/lower-case"),
        info::<Like>("text/like"),
        info::<And>("boolean/and"),
        info::<Or>("boolean/or"),
        info::<Not>("boolean/not"),
        info::<ToStringFormula>("text/from"),
        info::<ParseUnsignedInteger>("unsigned-integer/parse"),
        info::<ParseSignedInteger>("signed-integer/parse"),
        info::<ParseFloat>("float/parse"),
    ]
}
