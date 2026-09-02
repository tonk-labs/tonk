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
//! It renders one frame to stdout rather than taking over the terminal.
//! That keeps it runnable without a tty, and makes the same code path
//! the snapshot-test harness.
//!
//! ```text
//! tonk-tui-poc --template demo/todo.tui.html --data demo/todo.json --size 60x12
//! tonk-tui-poc --template demo/todo.tui.html --data demo/todo.json --explain
//! ```

#![forbid(unsafe_code)]

mod cli;
mod notation;
mod paint;
mod pipeline;
mod theme;
mod vocabulary;

use std::process::ExitCode;

fn main() -> ExitCode {
    match cli::run() {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("tonk-tui-poc: {error}");
            ExitCode::FAILURE
        }
    }
}
