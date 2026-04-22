use crate::api;
use crate::components::{
    NewRepoVisible, RepoListResource, Status, TonkJoin, TonkNewRepo, TonkSpace, TonkToolbar,
};
use leptos::prelude::*;
use leptos_router::{
    components::{Route, Router, Routes},
    path,
};

/// Main launcher view: toolbar column + routed workspace + overlays.
///
/// Owns the repo-list resource and the new-repo overlay visibility
/// signal, and provides both via context so the toolbar (reader +
/// writer) and overlay components can share one source of truth.
#[component]
pub fn TonkLauncher() -> impl IntoView {
    let status = use_context::<Signal<Status, LocalStorage>>().expect("Missing status");

    // The list fetch is gated on `Status::Ready` — the worker's
    // `GET /api/repositories` opens home, so it 500s until
    // TonkShell's init has PUT home first. Tracking status here
    // means the resource re-runs when init completes. Before then
    // it resolves to an empty list, which is invisible anyway
    // since the toolbar is translated off-screen until Ready.
    let repos = LocalResource::new(move || {
        let ready = matches!(status.get(), Status::Ready);
        async move {
            if !ready {
                return Ok::<_, String>(Vec::new());
            }
            api::list_repositories().await.map_err(|e| format!("{e}"))
        }
    });
    provide_context(RepoListResource(repos));

    let new_repo_visible = RwSignal::new(false);
    provide_context(NewRepoVisible(new_repo_visible));

    view! {
        <Router>
            <section class="launcher">
                <TonkToolbar />
                <Routes fallback=move || view!{ <section class="404">"Nothing here ¯\\_(ツ)_/¯"</section> }>
                    <Route path=path!("space/:space?") view=TonkSpace />
                    <Route path=path!("join") view=TonkJoin />
                </Routes>
                <TonkNewRepo />
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

        // Wait for the shell's `PUT /api/repository/home` to complete
        // and the redirect to land on `/space/home`; `section.repository`
        // appears when the fetch succeeds.
        let repository = driver.query(By::Css("section.repository")).first().await?;

        // The rendered summary includes the subject DID in a `<code>`
        // under the identifiers list; a `did:key:` substring is
        // enough evidence that a real response came back.
        let text = repository.text().await?;
        assert!(
            text.contains("Subject"),
            "expected repository summary to include a Subject row, got: {text}",
        );
        assert!(
            text.contains("did:key:"),
            "expected repository summary to include a did:key value, got: {text}",
        );

        driver.quit().await?;

        Ok(())
    }

    /// Mint an open invite, navigate to `/join?...#...`, and verify the
    /// service worker claims it end-to-end.
    ///
    /// Successful claim ends at `/space/repo-<ts>-<seq>` with a
    /// `pre.repository` block whose JSON includes the invite's subject
    /// DID. That's the deterministic signal we wait on — the
    /// intermediate `.status.ok` state is too short-lived (the
    /// redirect fires in the same spawn_local block).
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

        // Success signal: a repository JSON block appears, and we're
        // no longer on /join. `pre.repository` can render briefly for
        // home before the claim-redirect lands on the new space, so
        // we poll until the rendered JSON contains the invited
        // subject DID.
        let subject_str = subject_did.to_string();
        let mut last_text = String::new();
        let mut last_url = String::new();
        let mut ok = false;
        for _ in 0..100 {
            if let Ok(url) = driver.current_url().await {
                last_url = url.to_string();
            }
            if let Ok(elem) = driver.find(By::Css("section.repository")).await {
                last_text = elem.text().await.unwrap_or_default();
                if last_text.contains(&subject_str) {
                    ok = true;
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        if !ok {
            let status_text = driver
                .find(By::Css(".auth .status"))
                .await
                .ok()
                .map(|e| async move { e.text().await.unwrap_or_default() });
            let status_text = match status_text {
                Some(fut) => fut.await,
                None => String::new(),
            };
            panic!(
                "claim did not land on /space/repo-<...> within timeout.\n\
                 last URL: {last_url}\n\
                 last section.repository text: {last_text}\n\
                 last .auth .status text: {status_text}\n\
                 expected subject DID: {subject_str}"
            );
        }

        let url = driver.current_url().await?;
        assert!(
            url.path().starts_with("/space/repo-"),
            "expected to land on /space/repo-<generated>, got {url}",
        );

        driver.quit().await?;
        Ok(())
    }
}
