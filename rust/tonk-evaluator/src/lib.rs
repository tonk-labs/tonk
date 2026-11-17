#![warn(missing_docs)]
// Tests commit with the one-step publish, which fails on a lost race:
// a test's branch has no other writer to lose one to.
#![cfg_attr(test, allow(deprecated))]
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
//!
//! Rule induction is dialog's: installed `dialog.rule/*` rules fire
//! at commit time inside `TransactionCommit::perform`, so this crate
//! carries no fixpoint of its own.

pub mod evaluate;
