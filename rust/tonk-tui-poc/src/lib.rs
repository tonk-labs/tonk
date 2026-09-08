//! `tonk-tui-poc` — render a `tui` view facet into terminal cells.
//!
//! The point of this binary is to run the **real** view pipeline, not a
//! mock of it: it calls `tonk_render`'s own `parse_fragment` /
//! `collect_bindings` and `tonk_template`'s own planner, exactly as
//! `tonk render` does, and only diverges at the seam
//! `render_nodes` -> `Vec<Node>` (`plan/tui-views.md` §1.4). Everything
//! after that seam — the terminal vocabulary, the elm-ui layout algebra,
//! the theme and the painter — is new.
//!
//! It renders one frame to stdout by default rather than taking over the
//! terminal. That keeps it runnable without a tty, and makes the same
//! code path the snapshot-test harness. `--interactive` runs the event
//! loop instead; `--keys` drives the same interaction model headlessly,
//! which is how it is tested.
//!
//! ```text
//! tonk-tui-poc --template demo/todo.tui.html --data demo/todo.json --size 60x12
//! tonk-tui-poc --template demo/todo.tui.html --data demo/todo.json --explain
//! tonk-tui-poc --template demo/todo-interactive.tui.html --data demo/todo.json \
//!   --bindings demo/todo.bindings.json --interactive
//! ```

#![forbid(unsafe_code)]

pub mod activate;
pub mod cli;
pub mod focus;
pub mod notation;
pub mod paint;
pub mod pipeline;
pub mod session;
pub mod terminal;
pub mod theme;
pub mod vocabulary;
pub mod wiring;
