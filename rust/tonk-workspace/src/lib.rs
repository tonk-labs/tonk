//! Viewer workspace bootstrap.
//!
//! Ships the `artifact` concept — an entity bound to a model and a
//! view plus tab metadata (title, icon), whose `(entity, model,
//! view)` map one-to-one onto `<tonk-display entity model view>`.
//!
//! The bootstrap (`bootstrap.yaml`) has two clearly-marked
//! sections: the built-in schema (the `view` and `artifact`
//! concepts) and a demo-content block to be deleted before
//! shipping (a `trip` concept, its view, and the artifacts that
//! render it).
//!
//! The `view` concept is kept identical to the one in
//! `tonk-board/bootstrap.yaml` so both crates seed the same `view`
//! concept entity — the `{model, display}` shape `<tonk-display>`
//! actually queries.
//!
//! See `plan/tonk-viewer.md` at the repository root for the design.

#![warn(missing_docs)]

use std::sync::LazyLock;

use tonk_core::claim::TransactRequest;

/// The `view` and `artifact` concepts plus demo content, as a typed
/// transact request — lowered from `bootstrap.yaml` at compile time
/// by `claim!`. The shell folds this into the default repository's
/// `PUT` body so it seeds once at repo creation.
pub static BOOTSTRAP: LazyLock<TransactRequest> =
    LazyLock::new(|| tonk_macros::claim!("bootstrap.yaml"));

#[cfg(test)]
mod tests {
    use super::BOOTSTRAP;

    // Run wasm32 tests in the browser (ChromeDriver), matching the
    // sibling tonk-* crates. Without this the default wasm-bindgen
    // runner is Node.js, which the CI web test leg does not provide.
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn it_compiles_bootstrap_into_a_transact_request() {
        // The document must lower with no running system.
        assert!(
            !BOOTSTRAP.claims.is_empty(),
            "bootstrap should lower to at least one claim",
        );
    }

    #[dialog_common::test]
    fn it_carries_the_artifact_title() {
        use tonk_core::claim::Claim;
        // Each demo artifact authored a `title` (a text field).
        // Lowering must carry it as a claim parameter; a dropped
        // title leaves the tab unnamed.
        let has_title = BOOTSTRAP.claims.iter().any(|claim| {
            let app = match claim {
                Claim::Assert(a) | Claim::Retract(a) => a,
            };
            app.parameters.keys().any(|k| k == "title")
        });
        assert!(has_title, "lowering dropped the artifact title term");
    }

    #[dialog_common::test]
    fn it_carries_the_demo_stop_rows() {
        use tonk_core::claim::Claim;
        // Each demo `stop` authored an `item` (a text field) — the
        // sheet rows the trip view iterates. Lowering must carry it.
        let has_item = BOOTSTRAP.claims.iter().any(|claim| {
            let app = match claim {
                Claim::Assert(a) | Claim::Retract(a) => a,
            };
            app.parameters.keys().any(|k| k == "item")
        });
        assert!(has_item, "demo content did not contribute stop rows");
    }

    #[dialog_common::test]
    fn it_accumulates_every_stop_on_the_trip() {
        use tonk_core::claim::Claim;
        // The trip is asserted once per stop (a field value can't be
        // a list in notation, and `stop` is cardinality-many). Each
        // assertion adds one stop, so the bootstrap must carry a
        // distinct `stop` value for all seven wireframe stops.
        let mut stops: Vec<String> = BOOTSTRAP
            .claims
            .iter()
            .filter_map(|claim| {
                let Claim::Assert(app) = claim else {
                    return None;
                };
                app.parameters.get("stop").map(|v| format!("{v:?}"))
            })
            .collect();
        stops.sort();
        stops.dedup();
        assert_eq!(
            stops.len(),
            7,
            "expected seven distinct stops on the trip, got {stops:?}",
        );
    }
}
