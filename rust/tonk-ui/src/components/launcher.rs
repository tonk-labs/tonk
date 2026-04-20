use crate::components::{TonkEmpty, TonkJoin, TonkRepo, TonkSidebar};
use leptos::prelude::*;
use leptos_router::{
    components::{Route, Router, Routes},
    path,
};

/// Main launcher view: persistent sidebar and the routed view.
#[component]
pub fn TonkLauncher() -> impl IntoView {
    view! {
        <Router>
            <section class="launcher">
                <TonkSidebar />
                <Routes fallback=move || view!{ <section class="404">"Nothing here ¯\\_(ツ)_/¯"</section> }>
                    <Route path=path!("") view=TonkEmpty />
                    <Route path=path!("join") view=TonkJoin />
                    <Route path=path!("repo/:name?") view=TonkRepo />
                </Routes>
            </section>
        </Router>
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
    use dialog_credentials::ed25519::Ed25519Signer;
    #[cfg_attr(not(feature = "integration-tests"), allow(unused))]
    use dialog_ucan_core::{
        DelegationBuilder, DelegationChain, principal::Principal, subject::Subject as UcanSubject,
    };
    #[cfg_attr(not(feature = "integration-tests"), allow(unused))]
    use thirtyfour::prelude::*;
    #[cfg_attr(not(feature = "integration-tests"), allow(unused))]
    use tonk_invite::{Invite, InviteAudience};

    #[dialog_common::test]
    async fn it_navigates_to_the_default_space(test_environment: TestEnvironment) -> Result<()> {
        let driver = test_environment.driver().await?;

        let _launcher = driver
            .query(By::Css(".launcher"))
            .with_text("Nothing here ¯\\_(ツ)_/¯")
            .first()
            .await?;

        let space = driver.query(By::Css(".space")).first().await?;

        assert!(space.text().await?.starts_with("did:key:"));

        driver.quit().await?;

        Ok(())
    }

    /// Navigate to `/join?...#...` with a freshly-minted open invite and
    /// assert the worker claims it, opens a local repo, configures the
    /// remote against the test access service, and redirects the user to
    /// `/repo/<local_name>`.
    ///
    /// The test access service boots as part of [`TestEnvironment`] and
    /// is proxied at `/ucan/`, which is what we embed in the invite's
    /// `remote_url`.
    #[dialog_common::test]
    async fn it_claims_an_open_invite_via_the_join_route(
        test_environment: TestEnvironment,
    ) -> Result<()> {
        const SUBJECT_SEED: [u8; 32] = [11u8; 32];
        const ISSUER_SEED: [u8; 32] = [12u8; 32];
        const EPHEMERAL_SEED: [u8; 32] = [13u8; 32];

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
        )?;
        let join_base = format!("{}join", test_environment.tonk_web);
        let invite_url = invite.to_url(&join_base)?;

        let driver = test_environment.driver().await?;
        driver.goto(&invite_url).await?;

        // The success path redirects to `/repo/<local_name>`; the failure
        // path keeps us on `/join` with `.status.error`. Wait for either.
        let landed = driver
            .query(By::Css("section.repo, .join .status.error"))
            .first()
            .await?;
        let class = landed.attr("class").await?.unwrap_or_default();
        assert!(
            class.contains("repo") && !class.contains("status"),
            "claim flow did not reach repo view; class={class:?}"
        );

        let url = driver.current_url().await?;
        assert!(
            url.path().starts_with("/repo/"),
            "expected redirect to /repo/<name>, got {}",
            url.path()
        );
        let local_name = url
            .path()
            .trim_start_matches("/repo/")
            .trim_end_matches('/')
            .to_string();
        assert!(!local_name.is_empty(), "local_name must be non-empty");

        // Confirm the remote was configured by asking the worker directly.
        let status_url = format!(
            "{}api/repository/{}/status",
            test_environment.tonk_web, local_name
        );
        let status_json = driver
            .execute(
                &format!(
                    r#"
                    const response = await fetch("{status_url}");
                    return await response.json();
                "#
                ),
                vec![],
            )
            .await?;
        let status: serde_json::Value = status_json.json().clone();
        assert_eq!(
            status["has_upstream"].as_bool(),
            Some(true),
            "claim should configure upstream on main; status={status}"
        );

        driver.quit().await?;
        Ok(())
    }
}
