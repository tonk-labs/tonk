#![warn(missing_docs)]
//! Evaluation of analyzed Tonk notation documents.
//!
//! This crate sits at the top of the dependency graph:
//! `tonk-evaluator → tonk-analyzer → tonk-schema → tonk-core`. It takes
//! the [`Analysis<Syntax>`][tonk_analyzer::analysis::Analysis] tree the
//! analyzer produces and drives it against a repository — running the
//! synthesized queries, applying mutations, and committing.
//!
//! - [`evaluate`] — the evaluate chain (`syntax.evaluate`, `Evaluate`,
//!   `Evaluated`, `EvaluatedCommit`).
//! - [`effects`] — the `induce` fixpoint that fires installed effects.
//! - [`effect_query`] — effect storage, lookup, and validation.

pub mod evaluate;

pub mod effect_query;

pub mod effects;
