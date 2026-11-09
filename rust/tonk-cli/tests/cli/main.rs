//! The CLI's behavioural tests, built as one binary.
//!
//! Each other file in this directory is a module of this crate rather
//! than an integration test target of its own. A test target is a crate,
//! and a crate that drives a site through dialog's `perform(&env)`
//! entry points compiles its own copy of the engine those entry
//! points reach, because an optimized build shares no generic
//! instantiation across crates. Thirty-one such crates each spent
//! several minutes of release codegen on the same copy; one crate
//! spends it once. nextest still runs every test in a process of its
//! own, so the tests are as isolated as they were.
//!
//! The fixtures the files share live in `common`.

mod common;

#[cfg(feature = "integration-tests")]
mod account_authority;
#[cfg(feature = "integration-tests")]
mod account_spaces;
mod agents;
mod authoring;
mod blob;
mod cli_space;
mod connection_commands;
mod connection_compatibility;
mod connections;
mod context;
mod data_verbs;
mod eval_documents;
mod handoff;
mod include;
#[cfg(feature = "integration-tests")]
mod invitation_attribution;
mod invite_revocation;
mod join_profile;
mod join_routing;
mod live_migration;
mod notation;
mod render;
#[cfg(feature = "integration-tests")]
mod revoked_device;
mod schema_read;
mod site;
mod space;
mod space_access;
mod space_inventory;
#[cfg(feature = "integration-tests")]
mod space_link;
mod sync;
mod telemetry;
mod transfer;
mod update;
