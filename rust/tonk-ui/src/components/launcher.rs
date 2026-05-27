use crate::components::{
    LastJoinOutcome, TonkConceptView, TonkCreateSpace, TonkDisplayView, TonkInviteDialog, TonkJoin,
    TonkLayoutView, TonkProfile, TonkSpace, TonkSpaceViewer, TonkToolbar,
};
use leptos::prelude::*;
use leptos_router::{
    components::{Route, Router, Routes},
    path,
};

/// Main launcher view. `<wa-page>` provides the adaptive shell:
/// navigation sits in its own column on desktop and collapses into a
/// drawer (with a hamburger toggle) below the mobile breakpoint.
///
/// The create-space dialog is mounted here once (outside the
/// `<Routes>`) so it survives navigation and can be triggered from
/// any screen via the shared [`CreateSpaceOpen`] signal.
///
/// [`CreateSpaceOpen`]: super::CreateSpaceOpen
#[component]
pub fn TonkLauncher() -> impl IntoView {
    let last_join_outcome =
        use_context::<LastJoinOutcome>().expect("LastJoinOutcome provided by TonkShell");

    view! {
        <tonk-host>
            <Router>
                <wa-page
                    navigation-placement="end"
                    attr:data-last-join-outcome=move || last_join_outcome.get()
                >
                    <TonkToolbar />
                    <Routes fallback=move || view!{ <section class="not-found">"Nothing here ¯\\_(ツ)_/¯"</section> }>
                        // Order matters: the more specific viewer
                        // route is listed before the catch-all
                        // `space/:space?` so deep links like
                        // `/space/foo/branch/main/view/bar` don't
                        // get swallowed by the generic space route.
                        <Route
                            path=path!("space/:space/branch/:branch/view/:entity")
                            view=TonkSpaceViewer
                        />
                        <Route
                            path=path!("space/:space/branch/:branch/concept/:source")
                            view=TonkConceptView
                        />
                        <Route
                            path=path!("space/:space/branch/:branch/display/:subject")
                            view=TonkDisplayView
                        />
                        <Route
                            path=path!("space/:space/branch/:branch/layout/:workspace")
                            view=TonkLayoutView
                        />
                        <Route path=path!("space/:space?") view=TonkSpace />
                        <Route path=path!("profile") view=TonkProfile />
                        <Route path=path!("join") view=TonkJoin />
                    </Routes>
                </wa-page>
                <TonkCreateSpace />
                <TonkInviteDialog />
            </Router>
        </tonk-host>
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
    use dialog_credentials::Ed25519Signer;
    #[cfg_attr(not(feature = "integration-tests"), allow(unused))]
    use dialog_ucan_core::{DelegationBuilder, DelegationChain, subject::Subject as UcanSubject};
    #[cfg_attr(not(feature = "integration-tests"), allow(unused))]
    use dialog_varsig::Principal as _;
    #[cfg_attr(not(feature = "integration-tests"), allow(unused))]
    use thirtyfour::prelude::*;
    #[cfg_attr(not(feature = "integration-tests"), allow(unused))]
    use tonk_invite::{Invite, InviteAudience};

    /// Selector for the profile footer tile. Keyed by its
    /// `aria-label` so the test doesn't depend on the footer's
    /// internal structure.
    #[cfg_attr(not(feature = "integration-tests"), allow(dead_code))]
    const PROFILE_TILE: &str = r#"wa-button[aria-label="Profile"]"#;

    /// Selector for the default-space tile in the sidebar.
    #[cfg_attr(not(feature = "integration-tests"), allow(dead_code))]
    const HOME_TILE: &str = r#"wa-button[aria-label="Open home space"]"#;

    #[dialog_common::test]
    async fn it_navigates_to_the_default_space(test_environment: TestEnvironment) -> Result<()> {
        let driver = test_environment.driver().await?;

        // Wait for the shell's `PUT /api/repository/home` to complete
        // and the redirect to land on `/space/home`. `.space-banner-title`
        // is only rendered after `repository_view` resolves, so it
        // doubles as the "page is ready" signal. Its `title`
        // attribute carries the repository's subject DID.
        let title = driver.query(By::Css(".space-banner-title")).first().await?;
        assert_eq!(title.text().await?, "home");
        let subject = title.attr("title").await?.unwrap_or_default();
        assert!(
            subject.starts_with("did:key:"),
            "expected banner title attribute to be a did:key, got: {subject}",
        );

        // The meta branch is bootstrapped by the worker on create
        // and the `main` branch is declared by `api::init`, so both
        // should appear as cards.
        let name_elements = driver
            .query(By::Css(".branch-card .branch-card-name"))
            .all_from_selector()
            .await?;
        let mut branch_names = Vec::with_capacity(name_elements.len());
        for element in name_elements {
            branch_names.push(element.text().await?);
        }
        assert!(
            branch_names.iter().any(|n| n == "main"),
            "expected a `main` branch card, got: {branch_names:?}",
        );
        assert!(
            branch_names.iter().any(|n| n == "meta"),
            "expected a `meta` branch card, got: {branch_names:?}",
        );

        // `api::init` wires up an `origin` remote; at least one
        // remote tile should render.
        assert!(
            driver.query(By::Css(".remote-card")).exists().await?,
            "expected at least one remote card to render",
        );

        driver.quit().await?;

        Ok(())
    }

    /// Clicking the profile tile in the sidebar footer should
    /// switch the route to `/profile` and swap the banner from
    /// the home space's view to the profile's. We identify the
    /// swap by the `title` attribute on `.space-banner-title`:
    /// `repository_view` writes the subject DID there, and the
    /// profile repo's DID differs from any space's.
    #[dialog_common::test]
    async fn it_navigates_to_the_profile_via_sidebar(env: TestEnvironment) -> Result<()> {
        let driver = env.driver().await?;

        // Wait for the home space to finish loading and capture
        // its DID so we can tell when the banner swaps.
        let home_banner = driver.query(By::Css(".space-banner-title")).first().await?;
        assert_eq!(home_banner.text().await?, "home");
        let home_did = home_banner.attr("title").await?.unwrap_or_default();
        assert!(
            home_did.starts_with("did:key:"),
            "expected home banner title to be a did:key, got: {home_did}",
        );

        // Sanity-check the active-state wiring: the home tile
        // should be marked active while we're on `/space/home`,
        // the profile tile shouldn't.
        let home_tile = driver.query(By::Css(HOME_TILE)).first().await?;
        assert!(
            home_tile
                .class_name()
                .await?
                .unwrap_or_default()
                .contains("is-active"),
            "expected home tile to carry `is-active` while on /space/home",
        );
        let profile_tile = driver.query(By::Css(PROFILE_TILE)).first().await?;
        assert!(
            !profile_tile
                .class_name()
                .await?
                .unwrap_or_default()
                .contains("is-active"),
            "expected profile tile to not be active on /space/home",
        );

        // `wa-button` with `href` renders an anchor inside its
        // shadow root. WebDriver's `click()` targets the host
        // element, which Chrome reports as "not interactable"
        // because the event target lives inside the shadow
        // boundary. Dispatching the click from JS hits the host
        // directly and bubbles normally, which is what a real
        // user's click does after the browser resolves the
        // composed path.
        driver
            .execute(
                &format!("document.querySelector({PROFILE_TILE:?}).click();"),
                vec![],
            )
            .await?;

        // Wait for the banner to swap. The profile's `title`
        // attribute differs from the home space's, so polling on
        // inequality is a race-free signal that the route
        // transition completed.
        let home_did_for_filter = home_did.clone();
        let profile_banner = driver
            .query(By::Css(".space-banner-title"))
            .with_filter(move |element: WebElement| {
                let home_did = home_did_for_filter.clone();
                async move {
                    let current = element.attr("title").await?.unwrap_or_default();
                    Ok(current.starts_with("did:key:") && current != home_did)
                }
            })
            .first()
            .await?;

        // URL reflects the navigation.
        let url = driver.current_url().await?;
        assert_eq!(
            url.path(),
            "/profile",
            "expected URL to be /profile after clicking profile tile, got: {url}",
        );

        // Banner shows a non-empty name and a distinct did:key.
        let profile_name = profile_banner.text().await?;
        assert!(
            !profile_name.trim().is_empty(),
            "expected profile banner to have non-empty text",
        );
        let profile_did = profile_banner.attr("title").await?.unwrap_or_default();
        assert!(
            profile_did.starts_with("did:key:"),
            "expected profile banner title attribute to be a did:key, got: {profile_did}",
        );
        assert_ne!(
            profile_did, home_did,
            "expected profile DID to differ from home DID",
        );

        // Active-state flipped: profile tile is now active, home
        // tile is not.
        let profile_tile = driver.query(By::Css(PROFILE_TILE)).first().await?;
        assert!(
            profile_tile
                .class_name()
                .await?
                .unwrap_or_default()
                .contains("is-active"),
            "expected profile tile to carry `is-active` on /profile",
        );
        let home_tile = driver.query(By::Css(HOME_TILE)).first().await?;
        assert!(
            !home_tile
                .class_name()
                .await?
                .unwrap_or_default()
                .contains("is-active"),
            "expected home tile to drop `is-active` on /profile",
        );

        driver.quit().await?;
        Ok(())
    }

    /// A repository created out-of-band (direct fetch, not via the
    /// dialog) should show up in the sidebar without a reload:
    /// the worker broadcasts on `/api/profile` after recording the
    /// replica, and the shell's listener refetches the profile in
    /// response. Clicking the freshly appeared tile should navigate
    /// to `/space/{name}` like any other sidebar tile.
    #[dialog_common::test]
    async fn it_surfaces_externally_created_space_in_sidebar(env: TestEnvironment) -> Result<()> {
        let driver = env.driver().await?;

        // Wait for the home space so we know the shell is live and
        // the broadcast subscription is in place.
        let home_banner = driver.query(By::Css(".space-banner-title")).first().await?;
        assert_eq!(home_banner.text().await?, "home");
        let home_did = home_banner.attr("title").await?.unwrap_or_default();
        assert!(
            home_did.starts_with("did:key:"),
            "expected home banner title to be a did:key, got: {home_did}",
        );

        // Sanity check: no `pictures` tile exists yet.
        assert!(
            !driver
                .query(By::Css(r#"wa-button[aria-label="Open pictures space"]"#))
                .nowait()
                .exists()
                .await?,
            "expected no `pictures` tile before creating one",
        );

        // Create the repo via a direct fetch — this is the path a
        // non-dialog caller (another tab, a CLI wrapper, a future
        // flow) would hit. The dialog's explicit refetch is gone,
        // so if the sidebar updates it's only because the worker's
        // broadcast reached the shell's listener.
        let create_result = driver
            .execute(
                r#"
                const response = await fetch('/api/repository/pictures', {
                    method: 'PUT',
                    headers: {
                        'Content-Type': 'application/json',
                        'If-None-Match': '*',
                    },
                    body: JSON.stringify({ branch: { main: {} } }),
                });
                return { status: response.status };
                "#,
                vec![],
            )
            .await?;
        let status = create_result.json()["status"].as_u64().unwrap_or(0);
        assert_eq!(
            status, 201,
            "expected PUT /api/repository/pictures to create (201), got {status}",
        );

        // Wait for the new tile to appear. `query` polls by default,
        // so this doubles as our proof that the broadcast round-trip
        // (worker → /api/profile channel → shell listener →
        // refetch → sidebar re-render) actually completed.
        let pictures_tile = driver
            .query(By::Css(r#"wa-button[aria-label="Open pictures space"]"#))
            .first()
            .await?;
        assert!(
            !pictures_tile
                .class_name()
                .await?
                .unwrap_or_default()
                .contains("is-active"),
            "new tile should not be active before we navigate to it",
        );

        // Click through the tile (shadow-root anchor, so dispatch
        // via JS — see `it_navigates_to_the_profile_via_sidebar`).
        driver
            .execute(
                "document.querySelector('wa-button[aria-label=\"Open pictures space\"]').click();",
                vec![],
            )
            .await?;

        // Wait for the banner to swap to the new space. Polling
        // on title inequality handles the transition race.
        let home_did_for_filter = home_did.clone();
        let pictures_banner = driver
            .query(By::Css(".space-banner-title"))
            .with_filter(move |element: WebElement| {
                let home_did = home_did_for_filter.clone();
                async move {
                    let current = element.attr("title").await?.unwrap_or_default();
                    Ok(current.starts_with("did:key:") && current != home_did)
                }
            })
            .first()
            .await?;

        let url = driver.current_url().await?;
        assert_eq!(
            url.path(),
            "/space/pictures",
            "expected URL to be /space/pictures after clicking the tile, got: {url}",
        );
        assert_eq!(pictures_banner.text().await?, "pictures");
        let pictures_did = pictures_banner.attr("title").await?.unwrap_or_default();
        assert!(
            pictures_did.starts_with("did:key:"),
            "expected pictures banner title to be a did:key, got: {pictures_did}",
        );
        assert_ne!(
            pictures_did, home_did,
            "expected pictures DID to differ from home DID",
        );

        driver.quit().await?;
        Ok(())
    }

    /// Land on `/join?...&name=<chosen>#<seed>` carrying an
    /// audience-open invite, click Join, and verify the
    /// recipient lands on `/space/<chosen>` with a banner whose
    /// title attribute matches the invited subject DID.
    ///
    /// Sigils across users converge because the local replica's
    /// DID equals the invited subject DID — the banner's `title`
    /// attribute (which `repository_view` writes from
    /// `info.subject`) is the cleanest invariant to assert.
    #[dialog_common::test]
    async fn it_joins_an_open_invite_via_the_join_route(
        test_environment: TestEnvironment,
    ) -> Result<()> {
        const SUBJECT_SEED: [u8; 32] = [11u8; 32];
        const ISSUER_SEED: [u8; 32] = [12u8; 32];
        const EPHEMERAL_SEED: [u8; 32] = [13u8; 32];
        const LOCAL_NAME: &str = "shared-pictures";

        let subject_did = Ed25519Signer::import(&SUBJECT_SEED).await?.did();
        let ephemeral_did = Ed25519Signer::import(&EPHEMERAL_SEED).await?.did();
        let issuer = Ed25519Signer::import(&ISSUER_SEED).await?;

        let delegation = DelegationBuilder::new()
            .issuer(issuer)
            .audience(&ephemeral_did)
            .subject(UcanSubject::Specific(subject_did.clone()))
            .command(vec![])
            .try_build()
            .await?;
        let chain = DelegationChain::new(delegation);

        let remote_url = test_environment.tonk_web.join("/ucan/")?;
        let invite = Invite::new(
            chain,
            InviteAudience::Open {
                seed: EPHEMERAL_SEED,
            },
            Some(remote_url),
        )
        .await?;
        let join_base = format!("{}join", test_environment.tonk_web);
        let mut invite_url = invite.to_url(&join_base)?;
        // Inviter's suggested name; the recipient's `/join` form
        // pre-fills with this. Avoid `Url::query_pairs_mut`
        // here because that re-orders existing params and we
        // want to leave the rest of the invite URL untouched —
        // appending is safe since we know `to_url` already
        // emitted at least one query parameter.
        invite_url.push_str(&format!("&name={}", LOCAL_NAME));
        // The fragment in `to_url`'s output sits at the end of
        // the URL string; the `&name=` we just spliced lands
        // *before* the `#` because `to_url` reorders during
        // round-trip. Sanity: re-parse to put the params/fragment
        // back in the right slots.
        let invite_url = url::Url::parse(&invite_url)?.to_string();

        let driver = test_environment.driver().await?;
        driver.goto(&invite_url).await?;

        // Click "Join" once the form is interactive.
        // `wa-button` renders an internal anchor/button inside its
        // shadow root — dispatch the click via JS for the same
        // reason the profile-tile test does (the host element is
        // not directly interactable from WebDriver).
        driver
            .query(By::Css(r#"wa-button[type="submit"]"#))
            .first()
            .await?;
        driver
            .execute(
                r#"document.querySelector('wa-button[type="submit"]').click();"#,
                vec![],
            )
            .await?;

        // Wait for the redirect to land on `/space/<LOCAL_NAME>`.
        // Polling on the URL avoids racing the post-claim
        // navigation; once we see it, the banner title is the
        // proof that the local replica's DID matches the invited
        // subject (the test's load-bearing invariant).
        let mut current_url = String::new();
        let mut found = false;
        for _ in 0..100 {
            current_url = driver
                .current_url()
                .await
                .map(|u| u.to_string())
                .unwrap_or_default();
            if current_url.contains(&format!("/space/{}", LOCAL_NAME)) {
                found = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert!(
            found,
            "join never redirected to /space/{LOCAL_NAME}; last URL: {current_url}",
        );

        let banner = driver.query(By::Css(".space-banner-title")).first().await?;
        assert_eq!(banner.text().await?, LOCAL_NAME);
        let banner_did = banner.attr("title").await?.unwrap_or_default();
        assert_eq!(
            banner_did,
            subject_did.to_string(),
            "expected banner title to carry the invited subject DID, \
             confirming the local replica was created with the \
             subject's verifier credential",
        );

        // `<wa-page data-last-join-outcome=...>` is the wire for
        // tests (and future UX) to distinguish the two join
        // outcomes without parsing an HTTP response. A fresh
        // invite for a previously-unseen subject should be
        // "joined".
        let page = driver.query(By::Css("wa-page")).first().await?;
        let outcome = page
            .attr("data-last-join-outcome")
            .await?
            .unwrap_or_default();
        assert_eq!(
            outcome, "joined",
            "expected fresh invite to land on the `joined` outcome",
        );

        driver.quit().await?;
        Ok(())
    }
}
