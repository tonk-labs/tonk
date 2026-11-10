//! The hosted agent's `/mcp`: an agent session token in, JSON-RPC out
//! ([`mcp`]), over the spaces the agent reaches.
//!
//! Every write to a space goes through the writer of that space and the
//! caller's MCP session ([`SpaceWriter`]): the only place revisions for the
//! pair are signed. A writer drives the space's own kernel, which its host
//! boots acting as the writer key ([`Kernels`]): a Durable Object's isolate
//! in the Host Worker, Wasmtime in this crate's tests.

pub mod join;
pub mod mcp;
#[cfg(all(feature = "native-kernels", not(target_arch = "wasm32")))]
pub mod native;
pub mod telemetry;
pub mod view;
mod writer;

pub use writer::{
    ACCESS_DENIED, CREDENTIAL_REVOKED, Kernels, ReadCaps, SpaceKernel, SpaceWriter, SyncFailure,
    WriterError, WriterKey, WriterSession, writer_seed,
};
