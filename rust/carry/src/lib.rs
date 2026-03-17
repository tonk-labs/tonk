#![cfg(not(target_arch = "wasm32"))]

// ---------------------------------------------------------------------------
// New carry modules (Phase 1)
// ---------------------------------------------------------------------------
pub mod assert_cmd;
pub mod help;
pub mod init;
pub mod query_cmd;
pub mod retract_cmd;
pub mod site;
pub mod space_cmd;
pub mod status_cmd;
pub mod target;

// ---------------------------------------------------------------------------
// Retained internal library modules
// ---------------------------------------------------------------------------
pub mod schema;
