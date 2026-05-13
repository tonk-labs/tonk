//! Transport-agnostic language server for `tonk-notation` documents.
//!
//! This crate sits between [`tonk_notation`] (the pure language model)
//! and whatever runtime hosts the server (the in-browser service
//! worker today; a stdio-mode CLI tomorrow). It owns:
//!
//! - **JSON-RPC dispatch.** Incoming messages are parsed, routed to a
//!   typed handler, and the handler's reply (if any) is serialized
//!   back. Hosts pump bytes; this crate does the routing.
//! - **Document state.** Per-`Uri` text buffers, populated by
//!   `textDocument/didOpen` and updated by `textDocument/didChange`.
//!   The state is the *source of truth* the server validates against;
//!   clients don't re-send the document on every diagnostic poll.
//! - **Capability negotiation.** A static set of advertised
//!   capabilities (just text-document sync + push diagnostics for now)
//!   returned from `initialize`.
//! - **A push channel.** `Server::take_outbound` drains any
//!   server-initiated notifications (most importantly
//!   `textDocument/publishDiagnostics`) the host should forward to the
//!   client. Hosts decide *how* to forward; this crate just queues.
//!
//! The crate is **runtime-agnostic**: synchronous, no `tokio`, no I/O.
//! Hosts integrate it however their environment fits — the SW dispatch
//! is a single `axum` handler that calls [`Server::handle_message`]
//! and serializes the reply; the CLI mode (future) wraps the same
//! method behind a stdio framing loop.
//!
//! # Currently supported
//!
//! - `initialize`, `initialized`, `shutdown`, `exit`
//! - `textDocument/didOpen`, `textDocument/didChange`, `textDocument/didClose`
//! - On every change: validate via [`tonk_notation::document_diagnostics`]
//!   and queue a `textDocument/publishDiagnostics` notification.
//!
//! Completion, hover, signature help, and code actions are
//! orthogonal additions on top — each grows a new `handle_*`
//! method and an entry in the capability advertisement, without
//! touching the dispatch layer.

mod jsonrpc;
mod server;

pub use server::{IntrospectionFactory, Server};
