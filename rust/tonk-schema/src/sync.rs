//! Sync-state classification.
//!
//! The types and the classifier moved to `tonk-worker-api` (the
//! engine-free wire crate) so the web page can name `SyncState` and
//! classify without linking the datalog engine. They only need a
//! [`Revision`](dialog_artifacts::Revision), which is itself
//! engine-free. Re-exported here at their historical
//! `tonk_schema::sync::*` paths for the native CLI, the worker, and
//! the workspace.

pub use tonk_worker_api::{Comparison, SyncState, classify};
