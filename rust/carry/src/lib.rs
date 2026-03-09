// ---------------------------------------------------------------------------
// Native-only modules (CLI, filesystem)
// ---------------------------------------------------------------------------
#[cfg(not(target_arch = "wasm32"))]
pub mod assert_cmd;
#[cfg(not(target_arch = "wasm32"))]
pub mod help;
#[cfg(not(target_arch = "wasm32"))]
pub mod identity_cmd;
#[cfg(not(target_arch = "wasm32"))]
pub mod init;
#[cfg(not(target_arch = "wasm32"))]
pub mod invite_cmd;
#[cfg(not(target_arch = "wasm32"))]
pub mod join_cmd;
#[cfg(not(target_arch = "wasm32"))]
pub mod query_cmd;
#[cfg(not(target_arch = "wasm32"))]
pub mod retract_cmd;
#[cfg(not(target_arch = "wasm32"))]
pub mod site;
#[cfg(not(target_arch = "wasm32"))]
pub mod status_cmd;
#[cfg(not(target_arch = "wasm32"))]
pub mod target;

// ---------------------------------------------------------------------------
// Retained internal library modules
// ---------------------------------------------------------------------------
#[cfg(not(target_arch = "wasm32"))]
pub mod schema;
