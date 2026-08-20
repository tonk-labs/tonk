#![warn(missing_docs)]
//! Tonk — a local-only CLI for reading and writing tonk facts
//! via the asserted-notation DSL.
//!
//! The crate's public surface is small on purpose: it exists so
//! integration tests (which exercise commands without spawning
//! the binary) and a future SDK consumer can drive the same code
//! paths the CLI does.
//!
//! - [`site`] — repo+branch open/init; resolved via the spot registry.
//! - [`spot`] — spot registry: named spots, canonical storage, selection.
//! - [`account_spots`] — account inventory, pull, and backup reconciliation.
//! - [`recovery`] — whether a spot's data exists anywhere but this
//!   disk, so deleting it can say what it costs.
//! - [`identity`] — local profile management.
//! - [`agents`] — claim-backed spot context and `AGENTS.md` projection data.
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

pub mod account;
mod account_authority;
pub mod account_profiles;
mod account_session;
pub mod account_spots;
pub mod account_state;
pub mod account_sync;
pub mod agents;
pub mod authoring;
pub mod auto_sync;
pub mod blob;
/// A loopback callback server for browser authorization ceremonies.
pub mod callback;
pub mod context;
pub mod customer;
pub mod data;
pub mod data_ops;
pub mod deployment;
pub mod eval;
pub mod guide;
pub mod identity;
pub mod inventory;
pub mod invite;
/// Migrating a CSV export written by a pre-dialog-upgrade build.
pub mod legacy;
pub mod migrate;
pub mod output;
pub mod recovery;
pub mod remote;
pub mod render;
pub mod schema;
pub mod site;
pub mod spot;
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
