//! The access service's integration tests, built as one binary.
//!
//! Each other file in this directory is a module of this crate rather
//! than a test target of its own, for the reason tonk-cli's
//! `tests/cli/main.rs` gives: a test target is a crate, and every crate that
//! reaches dialog's `perform(&env)` entry points compiles its own copy
//! of the engine behind them in an optimized build. Each file keeps the
//! `#![cfg(..)]` it had as a target, so what runs where is unchanged;
//! nextest still runs every test in a process of its own.

mod account_remote;
mod connections;
mod deployment_config;
mod lookup;
mod object;
mod purger;
mod registration;
mod schema_doc;
mod shortcut;
mod ticket_claim;
mod ucan_integration;
