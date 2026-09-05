//! Event-handler bindings — DOM events on rendered markup become
//! transient concept assertions that cross the service-worker
//! boundary.
//!
//! See `plan/event-handling.md` for the design. The runtime is
//! split into four pieces:
//!
//! - [`path`] — parses `the:` identifiers in the `dom.event`
//!   namespace into structured paths and action names. Pure
//!   logic, native-testable.
//! - `preprocess` (wasm-only) — rewrites `on<event>=<concept>`
//!   attributes on a template fragment to `data-on<event>=<concept>`
//!   so the browser doesn't try to evaluate them as inline JS.
//! - `extract` (wasm-only) — walks a concept descriptor and a
//!   JS event, projecting field values via the `path` module.
//! - `delegate` (wasm-only) — installs per-event-type listeners
//!   on the host that route fires to the bound concept,
//!   extract, and POST the resulting `TransactRequest`.
//! - `binding` (wasm-only) — the `on:<name>=<command>` form: reads
//!   an `event!:` declaration's `where:` map against the live event
//!   instead of the command's own `dom.event.*` identifiers. The
//!   grammar, the dispatch table and the wire shape live in
//!   `tonk_template::event`, shared with any other host.

#[cfg(target_arch = "wasm32")]
pub mod binding;
#[cfg(target_arch = "wasm32")]
pub mod delegate;
#[cfg(target_arch = "wasm32")]
pub mod extract;
pub mod path;
#[cfg(target_arch = "wasm32")]
pub mod preprocess;
