#![cfg(not(target_arch = "wasm32"))]

// ---------------------------------------------------------------------------
// New carry modules (Phase 1)
// ---------------------------------------------------------------------------
pub mod assert_cmd;
pub mod init;
pub mod query_cmd;
pub mod retract_cmd;
pub mod site;
pub mod status_cmd;
pub mod target;

// ---------------------------------------------------------------------------
// Retained internal library modules
// ---------------------------------------------------------------------------
pub mod schema;

// ---------------------------------------------------------------------------
// Legacy modules (kept for compilation; will be removed in step 11)
// ---------------------------------------------------------------------------
pub mod attribute;
pub mod authority;
pub mod batch;
pub mod concept;
pub mod crypto;
pub mod delegation;
pub mod did;
pub mod entity;
pub mod fact;
pub mod import;
pub mod inspect;
pub mod keystore;
pub mod login;
pub mod metadata;
pub mod operator;
pub mod remote;
pub mod rule;
pub mod session;
pub mod space;
pub mod state;
pub mod status;
pub mod util;
