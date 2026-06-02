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

/// The `rule!:` installs from `bootstrap.yaml`, lifted at compile
/// time by `effects!`. Rules have no [`TransactRequest`] shape (the
/// `Claim` wire can't carry `dialog.effect/*` triples), so the shell
/// asserts these alongside the [`BOOTSTRAP`] claims when seeding the
/// repository — the seam that makes interactive tab selection live.
pub static RULES: LazyLock<Vec<tonk_schema::rule::Rule>> =
    LazyLock::new(|| tonk_macros::effects!("bootstrap.yaml"));

#[cfg(test)]
mod tests {
    use super::{BOOTSTRAP, RULES};

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
    fn it_lifts_the_activate_sheet_rule() {
        // The `rule!:` install is lifted out-of-band by `effects!`
        // (rules have no TransactRequest shape). Tab selection is
        // inert without it, so the bootstrap must carry the rule.
        assert_eq!(
            RULES.len(),
            1,
            "expected the activate-sheet rule to lift, got {}",
            RULES.len(),
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
    fn it_accumulates_every_stop_on_the_itinerary() {
        use tonk_core::claim::Claim;
        // The itinerary trip is asserted once per stop (a field value
        // can't be a list, and `stop` is cardinality-many). Counting
        // the `stop` values on the itinerary entity must total all
        // seven wireframe stops.
        let mut stops: Vec<String> = BOOTSTRAP
            .claims
            .iter()
            .filter_map(|claim| {
                let Claim::Assert(app) = claim else {
                    return None;
                };
                let this = app.parameters.get("this").map(|v| format!("{v:?}"))?;
                if !this.contains("tonk-workspace/itinerary") {
                    return None;
                }
                app.parameters.get("stop").map(|v| format!("{v:?}"))
            })
            .collect();
        stops.sort();
        stops.dedup();
        assert_eq!(
            stops.len(),
            7,
            "expected seven distinct stops on the itinerary, got {stops:?}",
        );
    }

    #[dialog_common::test]
    fn it_collects_the_workspace_sheets() {
        use tonk_core::claim::Claim;
        // The demo workspace is asserted once per sheet (cardinality
        // -many `sheet`), so the bootstrap must carry both distinct
        // sheet members for the tab strip to render.
        let mut sheets: Vec<String> = BOOTSTRAP
            .claims
            .iter()
            .filter_map(|claim| {
                let Claim::Assert(app) = claim else {
                    return None;
                };
                app.parameters.get("sheet").map(|v| format!("{v:?}"))
            })
            .collect();
        sheets.sort();
        sheets.dedup();
        assert_eq!(
            sheets.len(),
            2,
            "expected two workspace sheets, got {sheets:?}",
        );
    }
}
