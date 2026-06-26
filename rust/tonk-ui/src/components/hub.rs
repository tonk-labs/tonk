//! `/` route — the Tonk Hub.
//!
//! A repository picker: the set of spaces this profile can open,
//! rendered as cards that link to `/space/{name}`. The list lives on
//! the profile repository's meta branch (one `space`/replica record per
//! repository), so the Hub is a `<tonk-display concept=space>` directory
//! mounted under a `<tonk-repository profile>` — the `profile` flag
//! routes its queries to the profile-as-repository endpoint
//! (`/api/profile/branch/meta/query`) rather than a named repo.
//!
//! Like the bare display route, the Hub renders without shell chrome:
//! it is the page. The repository/branch elements are pure routing
//! context (transparent in layout); the cards and their styling come
//! from the `space` directory view in the standard library.

use leptos::prelude::*;

/// The Tonk Hub view at `/`. A thin shim that mounts the `<tonk-hub>`
/// custom element (see [`crate::components::route_views`]); the element
/// carries the routing-context markup and the `space` directory view does
/// the rest. This shim exists only while the router is still Leptos; once
/// the router mounts elements directly it goes away.
#[component]
pub fn TonkHub() -> impl IntoView {
    view! { <tonk-hub></tonk-hub> }
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

    /// On a fresh first load, the shell's `init()` creates the default
    /// `home` repository (`PUT /api/repository/home`), which records a
    /// replica for `home` on the profile's meta branch. The Hub at `/`
    /// lists that branch's replicas as cards — so a brand-new profile
    /// should show a `home` card linking to `/space/home`.
    ///
    /// Each test gets a fresh WebDriver session (its own browser
    /// profile and storage), so `driver().goto("/")` is a genuine
    /// first visit: no pre-existing `home`, no stale service worker.
    /// This is the case manual testing couldn't reproduce cleanly
    /// because cleared site-data left the worker's store intact.
    #[dialog_common::test]
    async fn it_lists_the_home_space_on_first_load(
        test_environment: TestEnvironment,
    ) -> Result<()> {
        let driver = test_environment.driver().await?;

        // First load of `/`. The shell's `init()` ensures the `home`
        // repo exists before the Hub mounts; the Hub then subscribes to
        // the profile meta branch and renders one card per replica.
        let target = test_environment.tonk_web.to_string();
        driver.goto(&target).await?;

        // The Hub renders a `.hub-card` per space. The profile's own
        // self-replica is hidden in CSS (`[data-kind="tonk:profile"]`);
        // the `home` repository card links to `/space/home`. `query`
        // polls, so this implicitly waits for init() to create `home`,
        // record its replica through the reactor, and the Hub's
        // subscription frame to land. Before the reactor-handle fix this
        // never appeared: the replica was written through a separate
        // profile handle the Hub's cached subscription never saw.
        let home_card = driver
            .query(By::Css("a.hub-card[href=\"/space/home\"]"))
            .first()
            .await?;

        // Sanity: it carries the repository kind, not the profile kind.
        let kind = home_card.attr("data-kind").await?;
        assert_eq!(
            kind.as_deref(),
            Some("tonk:repository"),
            "home card should be a repository replica, got data-kind={kind:?}",
        );

        driver.quit().await?;
        Ok(())
    }
}
