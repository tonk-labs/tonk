//! `<tonk-portal>` — an imperative rendering escape hatch.
//!
//! The declarative `<tonk-view>` / `<tonk-display>` stack paints a
//! template by interpolating an entity's fields into the page DOM.
//! That cannot express arbitrary imperative DOM work — canvas/WebGL
//! drawing, third-party widgets, custom state machines. `<tonk-portal>`
//! fills the gap: it writes an author-supplied HTML document (which
//! may run its own scripts) into a sandboxed `<iframe>`.
//!
//! Like `<tonk-view>`, the portal is a **painter, not a fetcher**. It
//! opens no subscription and resolves no descriptor; it receives an
//! already-fetched HTML string through its `content` attribute (and an
//! explicit `height`, since an iframe has no intrinsic content height)
//! and does one imperative thing — assign the iframe's `srcdoc`. The
//! `content` is itself first-class dialog data: the [`portal` concept]
//! holds it, and a nested `<tonk-display model=portal>` fetches it,
//! exactly as a board column fetches its tiles.
//!
//! This crate ships the element plus a [`BOOTSTRAP`] document seeding
//! the `portal` concept and its canonical view (resolved by model)
//! that bridges it to the element.
//!
//! [`portal` concept]: BOOTSTRAP

#![warn(missing_docs)]

use std::sync::LazyLock;

use tonk_core::claim::TransactRequest;

/// The `portal` concept and its canonical view as a typed transact
/// request — lowered from `bootstrap.yaml` at compile time by
/// `claim!`. The shell folds this into the default repository's `PUT`
/// body (chained after `tonk_board::BOOTSTRAP`) so the schema seeds
/// once at repo creation.
///
/// The document redeclares the `view` concept byte-identically to the
/// board's — same `this: tonk:view` pin and attribute set — so the
/// merged bootstrap seeds the one `tonk:view` entity and the claims
/// dedupe rather than minting a conflicting second concept.
pub static BOOTSTRAP: LazyLock<TransactRequest> =
    LazyLock::new(|| tonk_macros::claim!("bootstrap.yaml"));

#[cfg(target_arch = "wasm32")]
mod bridge;
#[cfg(target_arch = "wasm32")]
mod element;
#[cfg(target_arch = "wasm32")]
mod query;

#[cfg(target_arch = "wasm32")]
pub use element::register;

#[cfg(test)]
mod tests {
    use super::BOOTSTRAP;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn it_compiles_bootstrap_into_a_transact_request() {
        // `claim!` runs parse + local analysis + lowering at compile
        // time; the bundled bootstrap.yaml must produce a non-empty
        // claim set with no running system. A `view!` that could not
        // resolve the redeclared `view` concept would fail here.
        assert!(
            !BOOTSTRAP.claims.is_empty(),
            "bootstrap.yaml should lower to at least one claim",
        );
    }
}
