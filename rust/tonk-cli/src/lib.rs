#![warn(missing_docs)]
//! Tonk — a local-only CLI for reading and writing tonk facts
//! via the asserted-notation DSL.
//!
//! The crate's public surface is small on purpose: it exists so
//! integration tests (which exercise commands without spawning
//! the binary) and a future SDK consumer can drive the same code
//! paths the CLI does.
//!
//! - [`site`] — repo+branch open/init; resolved via the space registry.
//! - [`space`] — space registry: named spaces, canonical storage, selection.
//! - [`account_spaces`] — account inventory, pull, and backup reconciliation.
//! - [`recovery`] — whether a space's data exists anywhere but this
//!   disk, so deleting it can say what it costs.
//! - [`identity`] — local profile management.
//! - [`agents`] — claim-backed space context and `AGENTS.md` projection data.
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
pub mod account_observability;
mod account_session;
pub mod account_spaces;
pub mod account_state;
pub mod agents;
pub mod authoring;
pub mod auto_sync;
pub mod blob;
/// A loopback callback server for browser authorization ceremonies.
pub mod callback;
pub mod context;
pub mod custody;
pub mod customer;
pub mod data;
pub mod data_ops;
pub mod deployment;
pub mod eval;
pub mod guide;
pub mod highlight;
pub mod identity;
pub mod inventory;
pub mod invite;
pub mod listing;
pub mod migrate;
pub mod onboarding;
pub mod output;
pub mod recovery;
pub mod remote;
pub mod render;
pub mod schema;
pub mod site;
pub mod space;
pub mod space_link;
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

/// The envelope every `--json` listing carries.
///
/// There used to be two conventions. `tonk status --json` carries a
/// top-level string `schemaVersion`; every listing emitted a bare array
/// whose rows each repeated a numeric `version: 1`. Both were versioned
/// and neither could be recognised from the other, and the per-row form
/// spent a field on every row to say something true of the whole
/// response.
///
/// One shape, named for the command that produced it, so a reader can
/// tell what it is holding from the document alone.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rows<T> {
    /// `tonk.<command>.v<n>`.
    pub schema_version: &'static str,
    /// The listed rows, empty rather than absent when there are none.
    pub rows: Vec<T>,
}

impl<T> Rows<T> {
    /// Wrap `rows` in the envelope for `schema_version`.
    pub fn new(schema_version: &'static str, rows: Vec<T>) -> Self {
        Self {
            schema_version,
            rows,
        }
    }
}

/// An error that knows which [`ExitCode`] it should produce.
///
/// One trait rather than an inherent method per error enum, so the binary
/// can have a single printer that both renders the error the way
/// `--verbose` calls for and returns the code the error carries. It used to
/// have to choose: the helper that honoured `--verbose` flattened every
/// failure to [`ExitCode::IoError`], and the two dozen call sites that
/// needed a real code printed the error directly and ignored `--verbose`.
pub trait Coded: std::error::Error + Send + Sync + 'static {
    /// The exit code this failure should produce.
    fn exit_code(&self) -> ExitCode;
}
