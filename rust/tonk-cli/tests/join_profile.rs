//! Accountless invite claims use the persistent local profile directly.

mod common;

use anyhow::Result;
use tonk_cli::inventory;
use tonk_cli::invite;
use tonk_cli::site::TonkSite;

/// An invite already carries the authority needed to join. Before this
/// profile is linked to an account, that authority terminates at the
/// persistent profile DID; linking later connects the same profile to an
/// account without changing the local membership identity.
#[dialog_common::test]
async fn it_claims_to_the_profile_without_a_linked_account() -> Result<()> {
    let inviter = common::TestSite::new().await?;
    let invite = invite::mint(&inviter.site, None, None).await?;

    let claimer_tmp = tempfile::tempdir()?;
    let claimer_parent = claimer_tmp.path().canonicalize()?;
    let claimer_root = claimer_parent.join("joined-site");
    let mut claimer_config = common::isolated_config(&claimer_parent)?;
    // Match production's account precondition while keeping every file in
    // the fixture. Claiming is not an account-owned creation operation.
    claimer_config.require_account = true;

    // SAFETY: this integration-test binary contains exactly one test, so no
    // other thread can observe the process-wide store override.
    unsafe {
        std::env::set_var(
            tonk_cli::space::STATE_ENV,
            claimer_config.account_store.root(),
        );
    }

    let claimed = invite::claim(&claimer_root, &invite.url, claimer_config.clone()).await;
    // SAFETY: paired with the single-test process override above.
    unsafe {
        std::env::remove_var(tonk_cli::space::STATE_ENV);
    }
    claimed?;

    let joined = TonkSite::open_with(&claimer_root, claimer_config.clone()).await?;
    assert_eq!(
        tonk_cli::site::member_did(&joined).await?,
        joined.profile.did(),
        "claiming must not fabricate an anonymous membership root"
    );
    let roster = inventory::read_roster(&joined).await?;
    assert_eq!(roster.members.len(), 1);
    assert_eq!(roster.members[0].did, joined.profile.did().to_string());
    Ok(())
}
