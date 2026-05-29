//! Board layout custom elements.
//!
//! Three elements ship from this crate:
//!
//! - **`<tonk-board entity="…">`** — the outer wrapper. Takes a board
//!   entity URI and mounts a `<tonk-display>` against it. Resolution
//!   of `:board` name → entity URI happens at the route layer; this
//!   element receives the already-resolved URI.
//! - **`<tonk-strip>`** — horizontal scroll container; used inside
//!   the board view template as the host for column children.
//! - **`<tonk-column>`** — vertical scroll container with
//!   pull-to-reveal gesture; used inside the column view template
//!   as the host for tile children.
//!
//! All three are presentation containers. They do not subscribe to
//! data themselves — view templates (rendered by `<tonk-display>`)
//! supply children. Custom-element behavior is limited to layout
//! and gestures; data flows through the host abstraction defined
//! in `tonk-host`.
//!
//! See `plan/tonk-board.md` at the repository root for the design.

#![warn(missing_docs)]

use std::sync::LazyLock;

use tonk_core::claim::TransactRequest;

/// The board concepts, view templates, and `demo` board as a typed
/// transact request — lowered from `bootstrap.yaml` at compile time
/// by `claim!`. The shell folds this into the default repository's
/// `PUT` body so the schema seeds once at repo creation, rather
/// than re-evaluating the document on every board mount.
pub static BOOTSTRAP: LazyLock<TransactRequest> =
    LazyLock::new(|| tonk_macros::claim!("bootstrap.yaml"));

#[cfg(target_arch = "wasm32")]
mod board;
#[cfg(target_arch = "wasm32")]
mod column;
#[cfg(target_arch = "wasm32")]
mod strip;

/// Register `<tonk-board>`, `<tonk-strip>`, `<tonk-column>` with
/// the page. Idempotent — calling more than once is harmless.
#[cfg(target_arch = "wasm32")]
pub fn register() {
    board::register();
    strip::register();
    column::register();
}

#[cfg(test)]
mod tests {
    use super::BOOTSTRAP;

    // Run wasm32 tests in the browser (ChromeDriver), matching the
    // sibling tonk-* crates. Without this the default wasm-bindgen
    // runner is Node.js, which the CI web test leg does not provide
    // ("failed to find or execute Node.js").
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn it_compiles_bootstrap_into_a_transact_request() {
        // `claim!` runs parse + local analysis + lowering at
        // compile time; the bundled bootstrap.yaml must produce a
        // non-empty claim set with no running system.
        assert!(
            !BOOTSTRAP.claims.is_empty(),
            "bootstrap.yaml should lower to at least one claim",
        );
    }

    #[dialog_common::test]
    fn it_carries_the_demo_column_width() {
        use tonk_core::claim::Claim;
        // The demo `col-a` column authored `width: 12` (an
        // unsigned-integer field). Lowering must carry it as a
        // claim parameter; a dropped width breaks the board render.
        let has_width = BOOTSTRAP.claims.iter().any(|claim| {
            let app = match claim {
                Claim::Assert(a) | Claim::Retract(a) => a,
            };
            app.parameters.keys().any(|k| k == "width")
        });
        assert!(has_width, "lowering dropped the column width term");
    }
}
