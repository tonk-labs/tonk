//! Headless rendering of `tonk-display` view templates to HTML
//! strings, mirroring the browser renderer without a DOM.
//!
//! Pipeline: [`parse`] an HTML template into the owned [`tree`]
//! ([`tl`]-backed), [`collect`] bindings from it (splitting
//! interpolated text nodes in place, exactly like the browser
//! collector), feed them to the shared [`tonk_template`] planner,
//! then render the plan against query conclusions to an HTML
//! string. The planner is shared with `tonk-display`, so native
//! plan == browser plan.

pub mod collect;
pub mod parse;
pub mod render;
pub mod serialize;
pub mod tree;

pub use collect::collect_bindings;
pub use parse::parse_fragment;
pub use render::{Conclusion, render, render_nodes};
pub use serialize::serialize_nodes;
pub use tree::{Element, Node};
