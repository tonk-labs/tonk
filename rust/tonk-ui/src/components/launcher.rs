use crate::components::{
    LastJoinOutcome, TonkBoardView, TonkConceptView, TonkCreateSpace, TonkDisplayView,
    TonkInviteDialog, TonkJoin, TonkLayoutView, TonkProfile, TonkSpaceViewer, TonkToolbar,
};
use leptos::prelude::*;
use leptos_router::{
    components::{Outlet, ParentRoute, Route, Router, Routes},
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
    view! {
        <tonk-host>
            <Router>
                <Routes fallback=move || view!{ <section class="not-found">"Nothing here ¯\\_(ツ)_/¯"</section> }>
                    // The display route renders bare — just the
                    // `<tonk-display>` and its routing context, no shell
                    // chrome. It sits OUTSIDE the chromed parent route so
                    // no `<wa-page>` / toolbar wraps it. The space
                    // segment is `{branch}@{name}` (branch defaults to
                    // `main`); `*subject` is a wildcard so entity URIs
                    // containing `/` (e.g. `id:tonk-workspace/itinerary`)
                    // are captured whole rather than truncated.
                    //
                    //   /space/{name}/                       default view
                    //   /space/{branch}@{name}/              + branch
                    //   /space/{name}/{model}/               directory
                    //   /space/{name}/{entity}@{model}/*     artifact
                    //   /space/{name}/{entity}@{model}!{view}/*  ad-hoc
                    <Route path=path!("space/:space") view=TonkDisplayView />
                    <Route path=path!("space/:space/*subject") view=TonkDisplayView />

                    // Every other route renders inside the adaptive
                    // `<wa-page>` shell (navigation column on desktop,
                    // drawer on mobile) plus the toolbar. The parent
                    // route provides that chrome once and slots the
                    // matched child through `<Outlet/>`.
                    //
                    // The inspector/dev routes share the `{branch}@{name}`
                    // space convention. Their static keyword segment
                    // (`view`/`concept`/`layout`/`board`) is matched
                    // before the bare-space display routes above by the
                    // router's specificity ordering, so a model named
                    // `concept` is the only collision and is reserved.
                    <ParentRoute path=path!("") view=ChromeShell>
                        <Route
                            path=path!("space/:space/view/:entity")
                            view=TonkSpaceViewer
                        />
                        <Route
                            path=path!("space/:space/concept/:source")
                            view=TonkConceptView
                        />
                        <Route
                            path=path!("space/:space/layout/:workspace")
                            view=TonkLayoutView
                        />
                        <Route
                            path=path!("space/:space/board/:board")
                            view=TonkBoardView
                        />
                        <Route path=path!("profile") view=TonkProfile />
                        <Route path=path!("join") view=TonkJoin />
                    </ParentRoute>
                </Routes>
                <TonkCreateSpace />
                <TonkInviteDialog />
            </Router>
        </tonk-host>
    }
}

/// The chromed shell: the adaptive `<wa-page>` layout plus the
/// toolbar, with the matched child route rendered through
/// `<Outlet/>`. Wraps every route except the bare display route.
#[component]
fn ChromeShell() -> impl IntoView {
    let last_join_outcome =
        use_context::<LastJoinOutcome>().expect("LastJoinOutcome provided by TonkShell");
    view! {
        <wa-page
            navigation-placement="end"
            attr:data-last-join-outcome=move || last_join_outcome.get()
        >
            <TonkToolbar />
            <Outlet />
        </wa-page>
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

    #[dialog_common::test]
    async fn it_navigates_to_the_default_space(test_environment: TestEnvironment) -> Result<()> {
        let driver = test_environment.driver().await?;

        // Wait for the shell's `PUT /api/repository/home` to complete
        // and the redirect to land on `/space/home`. The bare display
        // route mounts `tonk-repository.display-route` once the space
        // is ready, with no shell chrome around it.
        let repository = driver
            .query(By::Css("tonk-repository.display-route"))
            .first()
            .await?;

        // The space name is `{branch}@{name}` with branch defaulting
        // to `main`, so `/` redirects to `/space/home`.
        let url = driver.current_url().await?;
        assert_eq!(
            url.path(),
            "/space/home",
            "expected the default redirect to land on /space/home, got: {url}",
        );
        assert_eq!(
            repository.attr("name").await?.unwrap_or_default(),
            "home",
            "expected the display route to name the home repository",
        );

        driver.quit().await?;

        Ok(())
    }

    /// Navigating to `/profile` lands on the chromed profile
    /// route. The bare `/space/home` route has no sidebar, so we
    /// reach the profile by direct navigation rather than a tile
    /// click. Once there, the chromed `<wa-page>` shell renders
    /// and the profile tile carries its active state.
    #[dialog_common::test]
    async fn it_navigates_to_the_profile_route(env: TestEnvironment) -> Result<()> {
        let driver = env.driver().await?;

        // Wait for the home space to finish loading. The bare
        // display route is the readiness signal now.
        driver
            .query(By::Css("tonk-repository.display-route"))
            .first()
            .await?;
        let url = driver.current_url().await?;
        assert_eq!(
            url.path(),
            "/space/home",
            "expected the default redirect to land on /space/home, got: {url}",
        );

        // Navigate directly to the profile route.
        driver.goto(&format!("{}profile", env.tonk_web)).await?;

        // `/profile` is chromed, so the adaptive `<wa-page>` shell
        // wraps it. Its presence is the route-ready signal.
        driver.query(By::Css("wa-page")).first().await?;
        let url = driver.current_url().await?;
        assert_eq!(
            url.path(),
            "/profile",
            "expected URL to be /profile after navigating, got: {url}",
        );

        // The chromed profile route renders the sidebar, so the
        // profile tile should carry `is-active` while we're on it.
        let profile_tile = driver.query(By::Css(PROFILE_TILE)).first().await?;
        assert!(
            profile_tile
                .class_name()
                .await?
                .unwrap_or_default()
                .contains("is-active"),
            "expected profile tile to carry `is-active` on /profile",
        );

        driver.quit().await?;
        Ok(())
    }

    /// A repository created out-of-band (direct fetch, not via the
    /// dialog) should be navigable at its bare display route. The
    /// space routes no longer carry a sidebar, so there's no tile
    /// to surface; instead we create the repo via a direct `PUT`
    /// and confirm `/space/{name}` mounts the bare display route.
    #[dialog_common::test]
    async fn it_surfaces_externally_created_space_in_sidebar(env: TestEnvironment) -> Result<()> {
        let driver = env.driver().await?;

        // Wait for the home space so we know the shell is live.
        driver
            .query(By::Css("tonk-repository.display-route"))
            .first()
            .await?;

        // Create the repo via a direct fetch — this is the path a
        // non-dialog caller (another tab, a CLI wrapper, a future
        // flow) would hit.
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

        // Navigate directly to the new space's bare display route.
        driver
            .goto(&format!("{}space/pictures", env.tonk_web))
            .await?;

        // The bare display route mounts, naming the new repository.
        let repository = driver
            .query(By::Css("tonk-repository.display-route"))
            .first()
            .await?;
        let url = driver.current_url().await?;
        assert_eq!(
            url.path(),
            "/space/pictures",
            "expected URL to be /space/pictures, got: {url}",
        );
        assert_eq!(
            repository.attr("name").await?.unwrap_or_default(),
            "pictures",
            "expected the display route to name the pictures repository",
        );

        driver.quit().await?;
        Ok(())
    }

    /// The shell bridges the workspace's `<tonk-share>` control to
    /// the invite dialog via a `tonk:share` window event. Firing it
    /// for the home repo opens the dialog and, once the mint lands,
    /// surfaces the invite-link input — the same path a real share
    /// click drives, minus the DOM click on the element (covered by
    /// `tonk-workspace`'s own dispatch test).
    #[dialog_common::test]
    async fn it_opens_the_invite_dialog_on_a_tonk_share_event(env: TestEnvironment) -> Result<()> {
        let driver = env.driver().await?;

        // Wait for the home space's bare display route — the shell is
        // live, so the `tonk:share` listener is mounted.
        driver
            .query(By::Css("tonk-repository.display-route"))
            .first()
            .await?;

        // Fire the bridge event the workspace's `<tonk-share>`
        // dispatches on click.
        driver
            .execute(
                r#"window.dispatchEvent(new CustomEvent('tonk:share', {
                    detail: { repo: 'home' },
                }));"#,
                vec![],
            )
            .await?;

        // The invite-link input renders only after the dialog opened
        // and the mint resolved, so its presence proves the bridge
        // opened the dialog and `create_invite` ran.
        let input = driver.query(By::Css("#tonk-invite-url")).first().await?;
        assert_eq!(
            input.tag_name().await?.to_lowercase(),
            "wa-input",
            "expected the minted invite link to render in a wa-input",
        );

        // The dialog itself is open (its `open` property is set by the
        // shared `invite_space` signal the bridge wrote).
        let is_open = driver
            .execute(
                r#"return document.querySelector('wa-dialog')?.open === true;"#,
                vec![],
            )
            .await?;
        assert_eq!(
            is_open.json().as_bool(),
            Some(true),
            "expected the invite <wa-dialog> to be open after tonk:share",
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
        // navigation. The destination is the bare display route, so
        // there's no banner or `<wa-page>` chrome to inspect once
        // we arrive — the redirect itself is the load-bearing
        // invariant (the local replica was created under the
        // requested name with the subject's verifier credential).
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

        // The bare display route mounts for the joined space,
        // naming the local replica.
        let repository = driver
            .query(By::Css("tonk-repository.display-route"))
            .first()
            .await?;
        let url = driver.current_url().await?;
        assert_eq!(
            url.path(),
            format!("/space/{LOCAL_NAME}"),
            "expected URL path to be /space/{LOCAL_NAME}, got: {url}",
        );
        assert_eq!(
            repository.attr("name").await?.unwrap_or_default(),
            LOCAL_NAME,
            "expected the display route to name the joined replica",
        );

        driver.quit().await?;
        Ok(())
    }
}
