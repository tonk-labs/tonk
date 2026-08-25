//! Accreditation: moving everything the onboarding account custodies to
//! the passkey account, then retiring the onboarding account.
//!
//! A device holds an onboarding account from first boot. Spaces it
//! creates delegate to that account; invites it redeems terminate there;
//! both seeds are sealed to its encryption key on profile `main`. When a
//! passkey account is created (or unlocked) on the device, every one of
//! those seeds is opened with the onboarding secret, which is local, and
//! used to mint a fresh `subject -> root` directly. Nothing is appended
//! below the onboarding account, so it can be retired without leaving a
//! hop in any chain. Design: `plan/join-under-custody.md`, Stage 3.
//!
//! Resumable: each seed is independent, and a seed still sealed only to
//! the onboarding recipient is simply picked up on the next attempt. The
//! onboarding account is retired only once nothing is sealed to it.

use dialog_credentials::{Ed25519Signer, Signer};
use dialog_repository::Repository;
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::subject::Subject as UcanSubject;
use dialog_ucan_core::{DelegationBuilder, DelegationChain};
use dialog_varsig::{Did, Principal as _};
use tonk_account::prefix::SPACE_ROOT_SITE_PREFIX;
use tonk_common::log;
use tonk_identity::sealed::{EncryptionKey, RecipientKey, Sealed};
use tonk_schema::{CustodiedSeed, SeedKind, prelude::DidExt as _};

use crate::TonkWorkerError;
use crate::worker::TonkState;

/// Bring everything custodied under the onboarding account under the
/// passkey root, then retire the onboarding account. Best effort per
/// seed: a seed that fails to rotate is logged and left sealed to the
/// onboarding recipient, and the retirement waits for it.
pub(crate) async fn rotate_from_onboarding(tonk: &TonkState) {
    let Ok(root) = super::identity::local_root(tonk).await else {
        return;
    };
    let onboarding = match crate::onboarding::did(tonk).await {
        Ok(Some(did)) => did,
        // Nothing to rotate: no onboarding account, or one already retired.
        _ => return,
    };
    let Ok(secret) = crate::onboarding::account(tonk).await else {
        return;
    };
    let old_key = secret.encryption_key();
    let old_recipient = old_key.recipient().did();
    let new_recipient =
        match super::account_state::published_encryption_key(tonk, &root.root_did).await {
            Ok(Some(recipient)) => recipient,
            Ok(None) => {
                log!("accreditation deferred: the account has not published its encryption key");
                return;
            }
            Err(error) => {
                log!("accreditation deferred: {error}");
                return;
            }
        };
    let new_key = match RecipientKey::from_did(&new_recipient) {
        Ok(key) => key,
        Err(error) => {
            log!("accreditation deferred: {error}");
            return;
        }
    };

    let rows = match sealed_to(tonk, &old_recipient).await {
        Ok(rows) => rows,
        Err(error) => {
            log!("accreditation deferred: {error}");
            return;
        }
    };
    let mut remaining = 0;
    for row in rows {
        match rotate_seed(tonk, &root.root_did, &onboarding, &old_key, &new_key, &row).await {
            Ok(subject) => log!("accreditation: {subject} re-issued to the account"),
            Err(error) => {
                remaining += 1;
                log!("accreditation: a seed was not rotated: {error}");
            }
        }
    }
    if remaining > 0 {
        log!("accreditation: {remaining} seed(s) still under the onboarding account");
        return;
    }
    retire_onboarding(tonk, &onboarding).await;
}

/// Every custodied seed sealed to `recipient`.
async fn sealed_to(
    tonk: &TonkState,
    recipient: &Did,
) -> Result<Vec<CustodiedSeed>, TonkWorkerError> {
    use dialog_query::{Output as _, Query, Term};
    let branch = tonk
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("open profile main: {error}")))?;
    branch
        .handle()
        .query()
        .select(Query::<CustodiedSeed> {
            this: Term::var("this"),
            subject: Term::var("subject"),
            kind: Term::var("kind"),
            recipient: Term::from(tonk_schema::domain::custody::Recipient(recipient.this())),
            sealed: Term::var("sealed"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("read custodied seeds: {error:?}")))
}

/// Open one seed with the onboarding key, re-issue what it derives to
/// the root, re-seal it to the account, and replace the row.
async fn rotate_seed(
    tonk: &TonkState,
    root: &Did,
    onboarding: &Did,
    old_key: &EncryptionKey,
    new_key: &RecipientKey,
    row: &CustodiedSeed,
) -> Result<Did, TonkWorkerError> {
    let subject: Did = row
        .subject
        .0
        .to_string()
        .parse()
        .map_err(|error| TonkWorkerError::Internal(format!("custodied subject: {error}")))?;
    let sealed = Sealed::decode(&row.sealed.0)
        .map_err(|error| TonkWorkerError::Internal(format!("{subject}: {error}")))?;
    let seed = old_key
        .open(&sealed, &subject)
        .map_err(|error| TonkWorkerError::Internal(format!("{subject}: {error}")))?;
    let signer = Ed25519Signer::import(&*seed)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("{subject}: {error}")))?;
    if signer.did() != subject {
        return Err(TonkWorkerError::Internal(format!(
            "{subject}: the custodied seed derives {}",
            signer.did()
        )));
    }

    let kind = row.kind.0.to_string();
    if kind == SeedKind::SPACE {
        reissue_space(tonk, root, signer).await?;
    } else if kind == SeedKind::INVITE {
        reissue_membership(tonk, root, onboarding, signer).await?;
    } else {
        return Err(TonkWorkerError::Internal(format!(
            "{subject}: unknown seed kind {kind}"
        )));
    }

    let resealed = new_key
        .seal(&seed, &subject)
        .map_err(|error| TonkWorkerError::Internal(format!("{subject}: reseal: {error}")))?
        .encode();
    let kind = if kind == SeedKind::SPACE {
        SeedKind::Space
    } else {
        SeedKind::Invite
    };
    tonk.reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .transaction()
        .retract(row.clone())
        .assert(CustodiedSeed::new(
            subject.clone(),
            kind,
            new_key.did(),
            resealed,
        ))
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("{subject}: reseal commit: {error}")))?;
    Ok(subject)
}

/// Mint `space -> root` from the space's own signer and install it the
/// way creation does: access branch, retained into the account, the
/// persisted prefix, and consumer provisioning.
async fn reissue_space(
    tonk: &TonkState,
    root: &Did,
    signer: Ed25519Signer,
) -> Result<(), TonkWorkerError> {
    let subject = signer.did();
    let minter = Repository::from(signer);
    let chain = minter
        .access()
        .claim(&minter)
        .delegate(root.clone())
        .perform(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("{subject}: delegate: {error}")))?
        .into_chain();
    install_prefix(tonk, &subject, &chain).await?;
    super::account_state::retain_space_delegation(tonk, &chain).await;
    if let Err(error) = super::customer::provision_or_defer(tonk, &subject, &chain, None).await {
        log!("{subject}: provisioning skipped: {error}");
    }
    Ok(())
}

/// Re-root a joined membership: the stored chain for the space ends
/// `principal -> onboarding`; replace that last hop with `principal ->
/// root`, minted from the principal's own signer, and install the result.
async fn reissue_membership(
    tonk: &TonkState,
    root: &Did,
    onboarding: &Did,
    principal: Ed25519Signer,
) -> Result<(), TonkWorkerError> {
    let principal_did = principal.did();
    let mut found = None;
    for key in super::profile_name::real_space_keys(tonk).await {
        let Ok(space) = key.parse::<Did>() else {
            continue;
        };
        let Ok(prefix) = super::repository::space_root_prefix(tonk, &space).await else {
            continue;
        };
        let last_issuer = prefix.proofs().last().map(|hop| hop.issuer().clone());
        if prefix.audience() == onboarding && last_issuer.as_ref() == Some(&principal_did) {
            found = Some((space, prefix));
            break;
        }
    }
    let Some((space, prefix)) = found else {
        return Err(TonkWorkerError::NotFound(format!(
            "{principal_did}: no membership chain ends at this principal"
        )));
    };

    let hop = DelegationBuilder::new()
        .issuer(Signer::from(principal))
        .audience(root)
        .subject(UcanSubject::Specific(space.clone()))
        .command(vec![])
        .try_build()
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("{space}: mint hop: {error}")))?;
    let mut hops: Vec<_> = prefix.proofs().cloned().collect();
    hops.pop();
    let mut hops = hops.into_iter();
    let first = hops
        .next()
        .ok_or_else(|| TonkWorkerError::Internal(format!("{space}: chain has one hop")))?;
    let mut chain = DelegationChain::new(first);
    for delegation in hops {
        chain = chain
            .push(delegation)
            .map_err(|error| TonkWorkerError::Internal(format!("{space}: rebuild: {error}")))?;
    }
    let chain = chain
        .push(hop)
        .map_err(|error| TonkWorkerError::Internal(format!("{space}: rebuild: {error}")))?;
    install_prefix(tonk, &space, &chain).await?;
    super::join::retain_claim_authority(tonk, space.repo_key(), &chain).await;
    Ok(())
}

/// Save a re-issued chain to the access branch and as the persisted
/// prefix for `subject`.
async fn install_prefix(
    tonk: &TonkState,
    subject: &Did,
    chain: &DelegationChain,
) -> Result<(), TonkWorkerError> {
    tonk.profile
        .access()
        .save(UcanDelegation(chain.clone()))
        .perform(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("{subject}: save: {error}")))?;
    let bytes = chain
        .to_bytes()
        .map_err(|error| TonkWorkerError::Internal(format!("{subject}: serialize: {error}")))?;
    tonk.profile
        .credential()
        .site(format!("{SPACE_ROOT_SITE_PREFIX}{subject}"))
        .save(bytes)
        .perform(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("{subject}: prefix: {error}")))?;
    Ok(())
}

/// Revoke the onboarding account's grant to this device and destroy the
/// onboarding custodian. Revocation is minted by the onboarding account
/// itself and published best-effort: the grant was only ever presented
/// to services the account space already lists, and a service that
/// never saw it has nothing to revoke.
async fn retire_onboarding(tonk: &TonkState, onboarding: &Did) {
    match crate::onboarding::grant_device(tonk).await {
        Ok(grant) => match crate::onboarding::signer(tonk).await {
            Ok(Some(signer)) => {
                let target = grant.proof_cids()[0];
                match tonk_identity::revocation::mint_root_revocation(signer, &grant, &target).await
                {
                    Ok(artifact) => {
                        if let Err(error) =
                            super::account_devices::publish_revocation(tonk, &artifact).await
                        {
                            log!("onboarding grant revocation not published: {error}");
                        }
                    }
                    Err(error) => log!("onboarding grant revocation not minted: {error}"),
                }
            }
            Ok(None) => log!("onboarding account already retired"),
            Err(error) => log!("onboarding signer unavailable: {error}"),
        },
        Err(error) => log!("onboarding grant unavailable: {error}"),
    }
    match crate::onboarding::retire(tonk).await {
        Ok(()) => log!("accreditation complete: onboarding account {onboarding} retired"),
        Err(error) => log!("onboarding account {onboarding} not retired: {error}"),
    }
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use crate::router::api_router_with_state;
    use crate::router::join::tests::{handcrafted_invite_url, post_join};
    use crate::router::tests::{persist_test_root, put_repo, test_state_without_root};
    use axum::http::StatusCode;
    use dialog_capability::Subject;
    use dialog_effects::Use;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_service_worker);

    /// A space created and a space joined under the onboarding account
    /// both end up rooted at the passkey account, their seeds re-sealed
    /// to it, and the onboarding account retired; both still prove.
    #[dialog_common::test]
    async fn it_rotates_created_and_joined_spaces_to_the_account() {
        let (app, state, _lsp) = api_router_with_state(test_state_without_root().await);
        let created_key = put_repo(&app, "rotated-space").await;
        let created: Did = created_key.parse().unwrap();
        let (url, joined_key) = handcrafted_invite_url(96, 97).await;
        assert_eq!(post_join(&app, &url).await, StatusCode::CREATED);
        let joined: Did = joined_key.parse().unwrap();

        let tonk = state.read().await;
        let onboarding = crate::onboarding::did(&tonk).await.unwrap().unwrap();
        let old_recipient = crate::onboarding::account(&tonk)
            .await
            .unwrap()
            .encryption_key()
            .recipient()
            .did();
        assert_eq!(sealed_to(&tonk, &old_recipient).await.unwrap().len(), 2);

        let root_did = persist_test_root(&tonk).await;
        rotate_from_onboarding(&tonk).await;

        for subject in [&created, &joined] {
            let prefix = super::super::repository::space_root_prefix(&tonk, subject)
                .await
                .unwrap();
            assert_eq!(prefix.audience(), &root_did, "{subject} re-rooted");
            assert!(
                prefix.proofs().all(|hop| hop.audience() != &onboarding),
                "{subject}: no hop is left at the onboarding account",
            );
            tonk.profile
                .access()
                .prove(Subject::from(subject.clone()).attenuate(Use))
                .audience(&tonk.operator)
                .perform(&tonk.operator)
                .await
                .expect("the re-issued chain proves");
        }

        assert!(
            sealed_to(&tonk, &old_recipient).await.unwrap().is_empty(),
            "nothing stays sealed to the onboarding account",
        );
        let new_recipient = super::super::account_state::published_encryption_key(&tonk, &root_did)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(sealed_to(&tonk, &new_recipient).await.unwrap().len(), 2);
        assert!(
            crate::onboarding::account(&tonk).await.is_err(),
            "the onboarding account can no longer be opened",
        );
    }
}
