#![warn(missing_docs)]
//! Tonk — a local-only CLI for reading and writing tonk facts
//! via the asserted-notation DSL.
//!
//! The crate's public surface is small on purpose: it exists so
//! integration tests (which exercise commands without spawning
//! the binary) and a future SDK consumer can drive the same code
//! paths the CLI does.
//!
//! - [`site`] — `.tonk/` discovery, repo+branch open/init.
//! - [`identity`] — local profile management.
//! - [`authoring`] — pure notation builders (concept, view, and the
//!   space-home recipe) consumed by the noun-first authoring verbs.
//! - [`data`] — pure notation builders (value rendering + doc
//!   assembly) consumed by the argument-based data verbs.
//! - [`data_ops`] — the testable handlers behind the argument-based
//!   data verbs (`tonk assert`, `tonk query`, …); `bin/tonk.rs` is a thin
//!   parser→call shim over these.
//! - [`eval`] — read source, drive [`tonk_evaluator::evaluate`],
//!   render output.
//! - [`output`] — render an [`tonk_evaluator::evaluate::EvaluateResponse`]
//!   as YAML notation or JSON.
//! - [`schema`] — branch introspection: dump every named
//!   attribute and concept as a re-submittable notation document.
//! - [`migrate`] — copy a `.carry/` directory to `.tonk/`.
//! - [`guide`] — the asserted-notation reference, baked in.
//! - [`telemetry`] — anonymous usage telemetry (PostHog), opt-out.
//! - [`update`] — self-update: `tonk update` plus the release check.

pub mod authoring;
pub mod auto_sync;
pub mod blob;
pub mod data;
pub mod data_ops;
pub mod eval;
pub mod guide;
pub mod identity;
pub mod invite;
pub mod migrate;
pub mod output;
pub mod remote;
pub mod render;
pub mod schema;
pub mod share;
pub mod site;
pub mod sync;
pub mod telemetry;
pub mod transfer;
pub mod update;
pub mod views;

/// CLI exit codes.
///
/// Each is a small u8 so a [`std::process::exit`] call lands the
/// right value on the shell. Agents can branch on these without
/// parsing stderr.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitCode {
    /// All operations succeeded.
    Success = 0,
    /// Source failed to parse — diagnostics on stderr.
    ParseError = 1,
    /// Analyzer rejected the document — diagnostics on stderr.
    AnalyzeError = 2,
    /// Dialog rejected the transaction (planner / commit failure).
    CommitError = 3,
    /// I/O, repo-not-found, or identity error.
    IoError = 4,
}

impl ExitCode {
    /// Numeric value, ready for [`std::process::exit`].
    pub fn into_raw(self) -> i32 {
        self as i32
    }
}
