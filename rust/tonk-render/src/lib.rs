//! Headless rendering of `tonk-display` view templates to HTML
//! strings, mirroring the browser renderer without a DOM.
//!
//! Two layers:
//!
//! - The **pure renderer** ([`parse`] + [`collect`] + [`render`] +
//!   [`serialize`]): [`parse`] an HTML template into the owned [`tree`]
//!   (`html5gum`-backed), [`collect`] bindings from it (splitting
//!   interpolated text nodes in place, exactly like the browser
//!   collector), feed them to the shared [`tonk_template`] planner,
//!   then render the plan against query conclusions to an HTML string.
//!   The planner is shared with `tonk-display`, so native plan ==
//!   browser plan.
//! - The **page orchestrator** ([`page`]): given a route and a
//!   [`page::QueryBackend`], it resolves which view and which data to
//!   render (by querying), then drives the pure renderer. This is the
//!   host-agnostic core both `slide render` and a worker SSR route
//!   share.

pub mod collect;
pub mod page;
pub mod parse;
pub mod render;
pub mod serialize;
pub mod tree;

pub use collect::collect_bindings;
pub use page::{QueryBackend, RenderError, RenderRoute, render as render_page};
pub use parse::parse_fragment;
pub use render::{Conclusion, render, render_nodes};
pub use serialize::serialize_nodes;
pub use tree::{Element, Node};
