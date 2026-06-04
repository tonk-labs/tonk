//! `/space/:space/board/:board` route (`:space` is `{branch}@{name}`).
//!
//! Renders `<tonk-board name={board}>` inside the
//! `<tonk-repository>` / `<tonk-branch>` routing wrappers. The
//! element handles bootstrap seeding, name resolution, and
//! mounting the underlying `<tonk-display>` — see
//! `tonk_board::board` for the lifecycle.
//!
//! See `plan/tonk-board.md` for the design.

use leptos::prelude::*;
use leptos_router::hooks::use_params;
use leptos_router::params::Params;

#[derive(Params, PartialEq, Clone, Debug)]
pub struct TonkBoardParams {
    space: Option<String>,
    board: Option<String>,
}

/// Board route. Reads `:space` (`{branch}@{name}`) and `:board` from
/// the URL and hands the name off to `<tonk-board>`; everything else —
/// bootstrap, name resolve, display mount — happens inside the
/// element via the host event system.
#[component]
#[allow(clippy::unused_unit)]
pub fn TonkBoardView() -> impl IntoView {
    let params = use_params::<TonkBoardParams>();

    let space_ref = Signal::derive_local(move || {
        params
            .get()
            .ok()
            .and_then(|p| p.space)
            .filter(|s| !s.is_empty())
            .and_then(|s| crate::components::route::parse_space(&s))
    });
    let space_name =
        Signal::derive_local(move || space_ref.get().map(|s| s.name).unwrap_or_default());
    let branch_name =
        Signal::derive_local(move || space_ref.get().map(|s| s.branch).unwrap_or_default());
    let board_name = Signal::derive_local(move || {
        params
            .get()
            .ok()
            .and_then(|p| p.board)
            .filter(|s| !s.is_empty())
            .unwrap_or_default()
    });

    view! {
        <main class="board-view">
            <tonk-repository name=move || space_name.get()>
                <tonk-branch name=move || branch_name.get()>
                    <tonk-board source=move || board_name.get()></tonk-board>
                </tonk-branch>
            </tonk-repository>
        </main>
    }
}

#[cfg(all(
    test,
    not(any(target_arch = "wasm32", feature = "web-integration-tests"))
))]
mod integration_tests {
    #![allow(unexpected_cfgs)]

    #[cfg_attr(not(feature = "integration-tests"), allow(unused))]
    use crate::helpers::TestEnvironment;
    #[cfg_attr(not(feature = "integration-tests"), allow(unused))]
    use anyhow::Result;
    #[cfg_attr(not(feature = "integration-tests"), allow(unused))]
    use thirtyfour::prelude::*;

    /// The board route is self-bootstrapping: `<tonk-board>`'s
    /// `connectedCallback` POSTs the bundled schema + demo data
    /// before resolving the board name. Landing on
    /// `/space/home/board/demo` with a fresh home repo
    /// is expected to produce a rendered strip with two columns
    /// (the demo board's `col-a` and `col-b`) inside the page's
    /// `<tonk-strip>` container — no manual seeding step.
    ///
    /// The test exercises the full host abstraction round-trip:
    /// bootstrap via `tonk-evaluate`, name lookup via `tonk-query`,
    /// content subscription via `tonk-subscribe`, fan-out through
    /// the nested view templates (board-view → column-view → tile).
    #[dialog_common::test]
    async fn it_renders_a_board_strip(test_environment: TestEnvironment) -> Result<()> {
        let driver = test_environment.driver().await?;

        // Navigate to the board route. `<tonk-board>` seeds the
        // bootstrap on mount; the strip should appear once the
        // POST + name lookup + subscription frame have landed.
        let target = format!("{}space/home/board/demo", test_environment.tonk_web);
        driver.goto(&target).await?;

        // The strip element materializes once the view template
        // chain renders board-view → column-view. `query` polls
        // so this implicitly waits for the round-trip.
        driver.query(By::Css("tonk-strip")).first().await?;

        // One column with one tile is expected from the demo data.
        let columns = driver
            .query(By::Css("tonk-strip tonk-column"))
            .all_from_selector()
            .await?;
        assert!(
            !columns.is_empty(),
            "expected at least one column in the strip, got {}",
            columns.len(),
        );

        driver.quit().await?;
        Ok(())
    }
}
