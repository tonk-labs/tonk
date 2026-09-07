//! Account rotation: moving everything the onboarding account custodies to
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
use dialog_query::{Output as _, Query, Term};
use dialog_repository::Repository;
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::subject::Subject as UcanSubject;
use dialog_ucan_core::{DelegationBuilder, DelegationChain};
use dialog_varsig::{Did, Principal as _};
use tonk_account::prefix::SPACE_ROOT_SITE_PREFIX;
use tonk_common::log;
use tonk_identity::sealed::RecipientKey;
#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
use tonk_schema::SecretMessage;
use tonk_schema::{InvitedVia, MemberName, MemberRole, Membership, SeedKind, prelude::DidExt as _};

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
    // The published fact is the account's word; the root record is the
    // ceremony's. Either names the same recipient, and the record is
    // available even while the account repository is still unhydrated
    // (a pending email activation blocks the sweep that publishes the
    // fact), so rotation must not wait on the publish.
    let new_recipient =
        match super::account_state::published_sealed_inbox(tonk, &root.root_did).await {
            Ok(Some(recipient)) => recipient,
            Ok(None) => match root.encryption_key.clone() {
                Some(recipient) => recipient,
                None => {
                    log!("account rotation deferred: the account has no encryption key");
                    return;
                }
            },
            Err(error) => {
                log!("account rotation deferred: {error}");
                return;
            }
        };
    let new_key = match RecipientKey::try_from(&new_recipient) {
        Ok(key) => key,
        Err(error) => {
            log!("account rotation deferred: {error}");
            return;
        }
    };

    let branch = match tonk
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
    {
        Ok(branch) => branch,
        Err(error) => {
            log!("account rotation deferred: open profile main: {error}");
            return;
        }
    };
    // The rotation itself is the shared core (`tonk_schema::custody::
    // rotate`), the same one the CLI runs at sign-in; only the re-issue
    // half — chains, prefixes, retention, provisioning — is this
    // adapter's.
    let outcome = match tonk_schema::custody::rotate(
        branch.handle(),
        secret.secret(),
        new_key,
        &tonk.operator,
        async |kind, signer, row, replacement| {
            match kind {
                SeedKind::Space => reissue_space(tonk, &root.root_did, &onboarding, signer)
                    .await
                    .map_err(|error| error.to_string())?,
                SeedKind::Invite => reissue_membership(tonk, &root.root_did, &onboarding, signer)
                    .await
                    .map_err(|error| error.to_string())?,
            }
            // The replacement commits through a fresh handle: the
            // re-issue writes above advanced the branch underneath any
            // handle held across them.
            tonk.reactor
                .profile_repository()
                .branch(tonk_account::MAIN_BRANCH)
                .transaction()
                .retract(row.clone())
                .assert(replacement.message)
                .assert(replacement.principal)
                .commit()
                .perform(&tonk.operator)
                .await
                .map(|_| ())
                .map_err(|error| format!("reseal commit: {error}"))
        },
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(error) => {
            log!("account rotation deferred: {error}");
            return;
        }
    };
    for subject in &outcome.rotated {
        log!("rotation: {subject} re-issued to the account");
    }
    for (subject, reason) in &outcome.failures {
        log!("rotation: {subject} was not rotated: {reason}");
    }
    if !outcome.failures.is_empty() {
        log!(
            "rotation: {} seed(s) still under the onboarding account",
            outcome.failures.len()
        );
        return;
    }
    retire_onboarding(tonk, &onboarding).await;
}

/// Every sealed message addressed to `recipient`.
#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
async fn sealed_to(
    tonk: &TonkState,
    recipient: &Did,
) -> Result<Vec<SecretMessage>, TonkWorkerError> {
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
        .select(Query::<SecretMessage> {
            this: Term::var("this"),
            to: Term::from(tonk_schema::domain::custody::To(recipient.this())),
            message: Term::var("message"),
            from: Term::var("from"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("read sealed messages: {error:?}")))
}

/// Mint `space -> root` from the space's own signer and install it the
/// way creation does: access branch, retained into the account, the
/// persisted prefix, and consumer provisioning.
async fn reissue_space(
    tonk: &TonkState,
    root: &Did,
    onboarding: &Did,
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
    migrate_membership_rows(tonk, &subject, onboarding, root).await?;
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
        if (prefix.audience() == onboarding || prefix.audience() == root)
            && last_issuer.as_ref() == Some(&principal_did)
        {
            found = Some((space, prefix));
            break;
        }
    }
    let Some((space, prefix)) = found else {
        return Err(TonkWorkerError::NotFound(format!(
            "{principal_did}: no membership chain ends at this principal"
        )));
    };

    let chain = if prefix.audience() == root {
        // A previous attempt installed the new prefix but stopped before
        // moving the roster or custody row. Reuse it and finish the commit.
        prefix.clone()
    } else {
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
        chain
            .push(hop)
            .map_err(|error| TonkWorkerError::Internal(format!("{space}: rebuild: {error}")))?
    };

    replace_retained_membership(tonk, &space, &prefix, &chain).await?;
    install_prefix(tonk, &space, &chain).await?;
    migrate_membership_rows(tonk, &space, onboarding, root).await?;
    Ok(())
}

/// Put the account-rooted membership proof in the shared space before
/// removing the onboarding leaf it replaces.
///
/// The ordering keeps at least one retained path at every step. The old leaf
/// is removed before the persisted prefix changes, so an interrupted attempt
/// can still identify and retry that cleanup from the onboarding prefix.
async fn replace_retained_membership(
    tonk: &TonkState,
    space: &Did,
    previous: &DelegationChain,
    replacement: &DelegationChain,
) -> Result<(), TonkWorkerError> {
    let branch = tonk
        .reactor
        .repository(space.repo_key())
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("{space}: open content: {error}")))?;
    branch
        .handle()
        .delegations()
        .retain(UcanDelegation(replacement.clone()))
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("{space}: retain account membership: {error}"))
        })?;

    if previous.audience() != replacement.audience() {
        let leaf = previous.proofs().last().cloned().ok_or_else(|| {
            TonkWorkerError::Internal(format!("{space}: membership chain has no leaf"))
        })?;
        branch
            .handle()
            .delegations()
            .retract(UcanDelegation(DelegationChain::new(leaf)))
            .perform(&tonk.operator)
            .await
            .map_err(|error| {
                TonkWorkerError::Internal(format!(
                    "{space}: retract onboarding membership: {error}"
                ))
            })?;
    }
    Ok(())
}

/// Repair a founder left behind by older created-space rotations. The
/// current direct space grant and the old account's direct grant to this
/// device establish the identity association; a role or petname alone does
/// not. Retained grants remain readable after the onboarding key is retired.
/// This changes descriptive roster facts only, never delegations.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(super) async fn reconcile_founder_membership(
    tonk: &TonkState,
    space: &Did,
    root: &Did,
) -> Result<bool, TonkWorkerError> {
    if super::identity::root_did(tonk).await.as_ref().ok() != Some(root) {
        return Ok(false);
    }
    let prefix = match super::repository::space_root_prefix(tonk, space).await {
        Ok(prefix) => prefix,
        Err(TonkWorkerError::NotFound(_)) => return Ok(false),
        Err(error) => return Err(error),
    };
    if prefix.issuer() != space || prefix.audience() != root || prefix.proofs().count() != 1 {
        return Ok(false);
    }
    let session = tonk
        .reactor
        .repository(space.repo_key())
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("{space}: open founder roster: {error}"))
        })?;
    let memberships: Vec<Membership> = session
        .handle()
        .query()
        .select(Query::<Membership> {
            this: Term::var("this"),
            subject: Term::from(space.this()),
            member: Term::var("member"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("{space}: read founder memberships: {error:?}"))
        })?;
    let roles: Vec<MemberRole> = session
        .handle()
        .query()
        .select(Query::<MemberRole> {
            this: Term::var("this"),
            role: Term::var("role"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("{space}: read founder roles: {error:?}"))
        })?;
    let profile = tonk
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("open historical account grants: {error}"))
        })?;
    let mut changed = false;
    for membership in memberships {
        if membership.member.0 == root.this()
            || !roles.iter().any(|role| {
                role.this == *membership.this() && role.role.0.to_string() == MemberRole::FOUNDER
            })
        {
            continue;
        }
        let Ok(previous) = membership.member.0.to_string().parse::<Did>() else {
            continue;
        };
        let Ok(grant) = super::revoke_invite::prove_path(
            profile.handle(),
            tonk,
            &previous,
            &tonk.profile.did(),
        )
        .await
        else {
            continue;
        };
        if grant.issuer() != &previous
            || grant.audience() != &tonk.profile.did()
            || grant.proofs().count() != 1
        {
            continue;
        }
        migrate_membership_rows(tonk, space, &previous, root).await?;
        changed = true;
    }
    Ok(changed)
}

/// Move the shared roster bundle from the onboarding account to the full
/// account in one content commit.
///
/// Membership entities are derived from `(space, member)`, so changing the
/// member DID necessarily changes the entity every role, name, and provenance
/// stamp addresses. Existing account-side stamps win on a resumed or repeated
/// migration; onboarding values only fill fields the account row lacks.
async fn migrate_membership_rows(
    tonk: &TonkState,
    space: &Did,
    onboarding: &Did,
    root: &Did,
) -> Result<(), TonkWorkerError> {
    let repo = space.repo_key();
    let session = tonk
        .reactor
        .repository(repo)
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("{space}: open roster: {error}")))?;
    let branch = session.handle();
    let previous = Membership::new(onboarding.clone(), space.clone());
    let replacement = Membership::new(root.clone(), space.clone());

    let previous_rows: Vec<Membership> = branch
        .query()
        .select(Query::<Membership> {
            this: Term::from(previous.this().clone()),
            subject: Term::var("subject"),
            member: Term::var("member"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("{space}: read onboarding membership: {error:?}"))
        })?;
    if previous_rows.is_empty() {
        let replacement_rows: Vec<Membership> = branch
            .query()
            .select(Query::<Membership> {
                this: Term::from(replacement.this().clone()),
                subject: Term::var("subject"),
                member: Term::var("member"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .map_err(|error| {
                TonkWorkerError::Internal(format!("{space}: read account membership: {error:?}"))
            })?;
        return if replacement_rows.is_empty() {
            Err(TonkWorkerError::NotFound(format!(
                "{space}: no onboarding membership to accredit"
            )))
        } else {
            Ok(())
        };
    }

    let previous_entity = previous.this().clone();
    let replacement_entity = replacement.this().clone();
    let previous_roles: Vec<MemberRole> = branch
        .query()
        .select(Query::<MemberRole> {
            this: Term::from(previous_entity.clone()),
            role: Term::var("role"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("{space}: read role: {error:?}")))?;
    let replacement_roles: Vec<MemberRole> = branch
        .query()
        .select(Query::<MemberRole> {
            this: Term::from(replacement_entity.clone()),
            role: Term::var("role"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("{space}: read account role: {error:?}"))
        })?;
    let previous_names: Vec<MemberName> = branch
        .query()
        .select(Query::<MemberName> {
            this: Term::from(previous_entity.clone()),
            name: Term::var("name"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("{space}: read name: {error:?}")))?;
    let replacement_names: Vec<MemberName> = branch
        .query()
        .select(Query::<MemberName> {
            this: Term::from(replacement_entity.clone()),
            name: Term::var("name"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("{space}: read account name: {error:?}"))
        })?;
    let previous_provenance: Vec<InvitedVia> = branch
        .query()
        .select(Query::<InvitedVia> {
            this: Term::from(previous_entity.clone()),
            invitation: Term::var("invitation"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("{space}: read invite provenance: {error:?}"))
        })?;
    let replacement_provenance: Vec<InvitedVia> = branch
        .query()
        .select(Query::<InvitedVia> {
            this: Term::from(replacement_entity.clone()),
            invitation: Term::var("invitation"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("{space}: read account provenance: {error:?}"))
        })?;

    let mut transaction = tonk
        .reactor
        .repository(repo)
        .branch(tonk_account::MAIN_BRANCH)
        .transaction()
        .assert(replacement);
    for row in previous_rows {
        transaction = transaction.retract(row);
    }
    for row in previous_roles {
        if replacement_roles.is_empty() {
            transaction = transaction.assert(MemberRole {
                this: replacement_entity.clone(),
                role: row.role.clone(),
            });
        }
        transaction = transaction.retract(row);
    }
    for row in previous_names {
        if replacement_names.is_empty() {
            transaction = transaction.assert(MemberName {
                this: replacement_entity.clone(),
                name: row.name.clone(),
            });
        }
        transaction = transaction.retract(row);
    }
    for row in previous_provenance {
        if replacement_provenance.is_empty() {
            transaction = transaction.assert(InvitedVia {
                this: replacement_entity.clone(),
                invitation: row.invitation.clone(),
            });
        }
        transaction = transaction.retract(row);
    }
    transaction
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("{space}: migrate membership roster: {error}"))
        })?;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    tonk.sync_queue.mark_dirty(repo, js_sys::Date::now());
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
    if let Err(error) = retract_onboarding_facts(tonk, onboarding).await {
        log!("onboarding facts not retracted: {error}");
    }
    match crate::onboarding::retire(tonk).await {
        Ok(()) => log!("account rotation complete: onboarding account {onboarding} retired"),
        Err(error) => log!("onboarding account {onboarding} not retired: {error}"),
    }
}

/// Clear what the onboarding account leaves behind on profile `main`:
/// its published encryption key — nothing is sealed to it once rotation
/// finished, so the fact only invites sealing to an unopenable
/// recipient — and the device-link row describing its just-revoked
/// grant, so the devices panel stops listing a retired account.
async fn retract_onboarding_facts(
    tonk: &TonkState,
    onboarding: &Did,
) -> Result<(), TonkWorkerError> {
    use dialog_query::{Output as _, Query, Term};
    use tonk_schema::{AccountSealedInbox, DeviceLink};

    let branch = tonk
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("open profile main: {error}")))?;
    let keys: Vec<AccountSealedInbox> = branch
        .handle()
        .query()
        .select(Query::<AccountSealedInbox> {
            this: Term::from(onboarding.this()),
            address: Term::var("address"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("read the onboarding key fact: {error:?}"))
        })?;
    let mut links: Vec<DeviceLink> = Vec::new();
    for entity in issued_link_entities(tonk, branch.handle(), onboarding).await? {
        let rows: Vec<DeviceLink> = branch
            .handle()
            .query()
            .select(Query::<DeviceLink> {
                this: Term::from(entity),
                created_at: Term::var("created_at"),
                title: Term::var("title"),
                reason: Term::var("reason"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .map_err(|error| {
                TonkWorkerError::Internal(format!("read the onboarding link row: {error:?}"))
            })?;
        links.extend(rows);
    }
    if keys.is_empty() && links.is_empty() {
        return Ok(());
    }
    let mut transaction = tonk
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .transaction();
    for row in keys {
        transaction = transaction.retract(row);
    }
    for row in links {
        transaction = transaction.retract(row);
    }
    transaction
        .commit()
        .perform(&tonk.operator)
        .await
        .map(|_| ())
        .map_err(|error| {
            TonkWorkerError::Internal(format!("retract the onboarding facts: {error}"))
        })
}

/// Entities of every retained delegation issued by `issuer` — for the
/// onboarding account, exactly its device grant.
async fn issued_link_entities(
    tonk: &TonkState,
    branch: &dialog_repository::Branch,
    issuer: &Did,
) -> Result<Vec<dialog_artifacts::Entity>, TonkWorkerError> {
    use dialog_artifacts::{ArtifactSelector, Value};
    use futures_util::StreamExt as _;

    let selector = ArtifactSelector::new()
        .the(
            dialog_repository::DELEGATION_ISSUER
                .parse()
                .map_err(|error| {
                    TonkWorkerError::Internal(format!("issuer attribute: {error:?}"))
                })?,
        )
        .is(Value::String(issuer.to_string()));
    let facts = branch
        .claims()
        .select(selector)
        .perform(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("select issued grants: {error}")))?
        .collect::<Vec<_>>()
        .await;
    let mut entities = Vec::new();
    for fact in facts.into_iter().flatten() {
        let bytes = fact.of_bytes().map_err(|error| {
            TonkWorkerError::Internal(format!("read an issued grant's entity: {error}"))
        })?;
        let entity: dialog_artifacts::Entity =
            String::from_utf8_lossy(&bytes).parse().map_err(|error| {
                TonkWorkerError::Internal(format!("parse an issued grant's entity: {error:?}"))
            })?;
        entities.push(entity);
    }
    Ok(entities)
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use crate::router::api_router_with_state;
    use crate::router::join::tests::{handcrafted_invite_url, post_join};
    use crate::router::tests::{
        content_invited_via, content_member_names, content_member_roles, content_memberships,
        persist_test_root, put_repo, test_state_without_root,
    };
    use axum::http::StatusCode;
    use dialog_capability::Subject;
    use dialog_effects::Use;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_service_worker);

    async fn retained_audience_count(
        state: &crate::router::AppState,
        repo: &str,
        audience: &Did,
    ) -> usize {
        use dialog_artifacts::{ArtifactSelector, Value};
        use futures_util::StreamExt as _;

        let tonk = state.read().await;
        let branch = tonk
            .reactor
            .repository(repo)
            .branch(tonk_account::MAIN_BRANCH)
            .acquire(&tonk.operator)
            .await
            .expect("the content branch opens");
        let selector = ArtifactSelector::new()
            .the(
                dialog_repository::DELEGATION_AUDIENCE
                    .parse()
                    .expect("the audience attribute parses"),
            )
            .is(Value::String(audience.to_string()));
        branch
            .handle()
            .claims()
            .select(selector)
            .perform(&tonk.operator)
            .await
            .expect("retained audiences read")
            .filter_map(|row| async move { row.ok() })
            .count()
            .await
    }

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
            .secret()
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
        let new_recipient = super::super::account_state::published_sealed_inbox(&tonk, &root_did)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(sealed_to(&tonk, &new_recipient).await.unwrap().len(), 2);
        assert!(
            crate::onboarding::account(&tonk).await.is_err(),
            "the onboarding account can no longer be opened",
        );

        // Nothing on profile main keeps naming the retired account: its
        // published key is retracted, and so is the device-link row
        // describing its revoked grant.
        use dialog_query::{Output as _, Query, Term};
        let branch = tonk
            .reactor
            .profile_repository()
            .branch(tonk_account::MAIN_BRANCH)
            .acquire(&tonk.operator)
            .await
            .unwrap();
        let keys: Vec<tonk_schema::AccountSealedInbox> = branch
            .handle()
            .query()
            .select(Query::<tonk_schema::AccountSealedInbox> {
                this: Term::from(onboarding.this()),
                address: Term::var("address"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .unwrap();
        assert!(keys.is_empty(), "the onboarding key fact is retracted");
        for entity in issued_link_entities(&tonk, branch.handle(), &onboarding)
            .await
            .unwrap()
        {
            let links: Vec<tonk_schema::DeviceLink> = branch
                .handle()
                .query()
                .select(Query::<tonk_schema::DeviceLink> {
                    this: Term::from(entity),
                    created_at: Term::var("created_at"),
                    title: Term::var("title"),
                    reason: Term::var("reason"),
                })
                .perform(&tonk.operator)
                .try_vec()
                .await
                .unwrap();
            assert!(links.is_empty(), "the onboarding link row is retracted");
        }
    }

    /// COLLAB-05 / B-07: creating before linking must preserve a complete
    /// founder row when the account changes and its name is projected.
    #[dialog_common::test]
    async fn it_rotates_the_created_space_founder_roster() {
        let (app, state, _lsp) = api_router_with_state(test_state_without_root().await);
        let key = put_repo(&app, "founder-roster").await;
        let root = {
            let tonk = state.read().await;
            let root = persist_test_root(&tonk).await;
            rotate_from_onboarding(&tonk).await;
            super::super::profile_name::project_member_name(&tonk, &key, &root, "jack")
                .await
                .unwrap();
            root
        };
        let memberships = content_memberships(&state, &key).await;
        assert_eq!(memberships.len(), 1);
        assert_eq!(
            memberships[0].member.0,
            root.this(),
            "founder must name the current account"
        );
        let roles = content_member_roles(&state, &key).await;
        let names = content_member_names(&state, &key).await;
        assert_eq!(roles.len(), 1);
        assert_eq!(roles[0].this, *memberships[0].this());
        assert_eq!(roles[0].role.0.to_string(), MemberRole::FOUNDER);
        assert_eq!(names.len(), 1);
        assert_eq!(names[0].this, *memberships[0].this());
        assert_eq!(names[0].name.0, "jack");
    }

    /// Older workers retired custody after rotating authority but left the
    /// founder bundle behind. Repair must also work with no onboarding secret.
    #[dialog_common::test]
    async fn it_repairs_a_retired_founder_roster_on_name_projection() {
        let (app, state, _lsp) = api_router_with_state(test_state_without_root().await);
        let key = put_repo(&app, "retired-founder-roster").await;
        let old = content_memberships(&state, &key).await.remove(0);
        let old_role = content_member_roles(&state, &key).await.remove(0);
        let old_name = content_member_names(&state, &key).await.remove(0);
        let tonk = state.read().await;
        let root = persist_test_root(&tonk).await;
        rotate_from_onboarding(&tonk).await;
        assert!(crate::onboarding::account(&tonk).await.is_err());
        let current = Membership::new(root.clone(), key.parse().unwrap());
        let foreign = Membership::new(
            Ed25519Signer::import(&[113; 32]).await.unwrap().did(),
            key.parse().unwrap(),
        );
        let foreign_role = MemberRole::founder(foreign.this().clone());
        let foreign_name = MemberName::new(foreign.this().clone(), "unrelated".into());
        // Restore exactly the old worker's durable shape: root authority,
        // old founder bundle, and the otherwise-orphaned account name.
        tonk.reactor
            .repository(&key)
            .branch("main")
            .transaction()
            .retract(current.clone())
            .retract(MemberRole::founder(current.this().clone()))
            .assert(foreign.clone())
            .assert(foreign_role.clone())
            .assert(foreign_name.clone())
            .assert(old.clone())
            .assert(old_role)
            .assert(old_name)
            .assert(MemberName::new(current.this().clone(), "jack".into()))
            .commit()
            .perform(&tonk.operator)
            .await
            .unwrap();
        // A sibling branch stands in for a replica: pull the broken state,
        // then the repair, proving that the bundle is durable sync data.
        let source = tonk
            .reactor
            .repository(&key)
            .branch("main")
            .acquire(&tonk.operator)
            .await
            .unwrap();
        let replica = tonk
            .reactor
            .repository(&key)
            .branch("roster-replica")
            .acquire(&tonk.operator)
            .await
            .unwrap();
        replica
            .handle()
            .set_upstream(source.handle())
            .perform(&tonk.operator)
            .await
            .unwrap();
        tonk.reactor
            .repository(&key)
            .branch("roster-replica")
            .pull()
            .perform(&tonk.operator)
            .await
            .unwrap();
        let wire: crate::reactor::Query =
            serde_json::from_str(&tonk_fab::logic::member_roster_query_body()).unwrap();
        let roster_query = wire.into_concept_query().unwrap();
        let mut subscription = tonk
            .reactor
            .repository(&key)
            .branch("main")
            .subscribe(roster_query.clone())
            .perform(&tonk.operator)
            .await
            .unwrap();
        let initial = subscription.receiver.recv().await.unwrap();
        assert!(String::from_utf8_lossy(&initial).contains(&old.member.0.to_string()));
        assert!(
            super::super::profile_name::project_member_name(&tonk, &key, &root, "jack")
                .await
                .unwrap(),
            "repair is a write even when the name is current"
        );
        tonk.reactor.run_scheduled_polls(&tonk.operator).await;
        let update = subscription
            .receiver
            .try_recv()
            .expect("live roster receives repair");
        let update = String::from_utf8_lossy(&update);
        assert!(update.contains(&root.to_string()), "{update}");
        assert!(update.contains("jack"), "{update}");
        let branch = tonk
            .reactor
            .repository(&key)
            .branch("main")
            .acquire(&tonk.operator)
            .await
            .unwrap();
        let before = branch.handle().revision();
        assert!(
            !super::super::profile_name::project_member_name(&tonk, &key, &root, "jack")
                .await
                .unwrap(),
            "repeat projection is a no-op"
        );
        assert_eq!(branch.handle().revision(), before);
        tonk.reactor
            .repository(&key)
            .branch("roster-replica")
            .pull()
            .perform(&tonk.operator)
            .await
            .unwrap();
        let local_rows = tonk
            .reactor
            .repository(&key)
            .branch("main")
            .query(roster_query.clone())
            .perform(&tonk.operator)
            .await
            .unwrap();
        let replicated_rows = tonk
            .reactor
            .repository(&key)
            .branch("roster-replica")
            .query(roster_query)
            .perform(&tonk.operator)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(local_rows).unwrap(),
            serde_json::to_value(replicated_rows).unwrap()
        );
        drop(tonk);
        let memberships = content_memberships(&state, &key).await;
        assert_eq!(memberships.len(), 2);
        assert!(memberships.contains(&current));
        assert!(
            memberships.contains(&foreign),
            "an unrelated founder is preserved"
        );
        let roles = content_member_roles(&state, &key).await;
        assert_eq!(roles.len(), 2);
        assert!(roles.contains(&MemberRole::founder(current.this().clone())));
        assert!(roles.contains(&foreign_role));
        let names = content_member_names(&state, &key).await;
        assert_eq!(names.len(), 2);
        assert!(names.contains(&MemberName::new(current.this().clone(), "jack".into())));
        assert!(names.contains(&foreign_name));
    }

    /// Accreditation replaces the onboarding account in the shared roster;
    /// it must not leave a second, retired identity behind in the space.
    #[dialog_common::test]
    async fn it_replaces_the_onboarding_membership_with_the_account() {
        let (app, state, _lsp) = api_router_with_state(test_state_without_root().await);
        let (url, joined_key) = handcrafted_invite_url(98, 99).await;
        assert_eq!(post_join(&app, &url).await, StatusCode::CREATED);

        let onboarding = crate::onboarding::did(&*state.read().await)
            .await
            .unwrap()
            .expect("the join minted the onboarding account");
        let before_memberships = content_memberships(&state, &joined_key).await;
        let before_membership = before_memberships
            .iter()
            .find(|row| row.member.0 == onboarding.this())
            .expect("the onboarding membership is recorded")
            .clone();
        let before_role = content_member_roles(&state, &joined_key)
            .await
            .into_iter()
            .find(|row| row.this == *before_membership.this())
            .expect("the onboarding membership has a role");
        let before_name = content_member_names(&state, &joined_key)
            .await
            .into_iter()
            .find(|row| row.this == *before_membership.this())
            .expect("the onboarding membership has a display name");
        let before_provenance = content_invited_via(&state, &joined_key)
            .await
            .into_iter()
            .find(|row| row.this == *before_membership.this())
            .expect("the onboarding membership has invite provenance");
        assert_eq!(
            retained_audience_count(&state, &joined_key, &onboarding).await,
            1,
            "the joined space retains its onboarding membership leaf",
        );

        let root = {
            let tonk = state.read().await;
            let root = persist_test_root(&tonk).await;
            rotate_from_onboarding(&tonk).await;
            root
        };

        let memberships = content_memberships(&state, &joined_key).await;
        assert_eq!(memberships.len(), 1, "accreditation leaves one member row");
        let membership = &memberships[0];
        assert_eq!(
            membership.member.0,
            root.this(),
            "the account is the member"
        );
        assert_ne!(
            membership.this(),
            before_membership.this(),
            "the membership entity is derived from the new account",
        );

        let roles = content_member_roles(&state, &joined_key).await;
        assert_eq!(
            roles.len(),
            1,
            "the retired membership leaves no role stamp"
        );
        assert_eq!(roles[0].this, *membership.this());
        assert_eq!(roles[0].role, before_role.role);

        let names = content_member_names(&state, &joined_key).await;
        assert_eq!(
            names.len(),
            1,
            "the retired membership leaves no name stamp"
        );
        assert_eq!(names[0].this, *membership.this());
        assert_eq!(names[0].name, before_name.name);

        let provenance = content_invited_via(&state, &joined_key).await;
        assert_eq!(
            provenance.len(),
            1,
            "the retired membership leaves no provenance stamp",
        );
        assert_eq!(provenance[0].this, *membership.this());
        assert_eq!(provenance[0].invitation, before_provenance.invitation);
        assert_eq!(
            retained_audience_count(&state, &joined_key, &onboarding).await,
            0,
            "no retained proof still names the onboarding account",
        );
    }

    /// A link whose account sweep failed (a pending email activation
    /// blocks hydration) has the recipient only in the root record, not
    /// as a published fact. Rotation still runs from the record instead
    /// of deferring until a publish that nothing retries.
    #[dialog_common::test]
    async fn it_rotates_from_the_root_record_before_the_key_is_published() {
        use crate::router::tests::test_root_seed;
        use dialog_credentials::Ed25519Signer;
        use dialog_varsig::Principal as _;

        let (app, state, _lsp) = api_router_with_state(test_state_without_root().await);
        let created_key = put_repo(&app, "unpublished-key-space").await;
        let created: Did = created_key.parse().unwrap();

        let tonk = state.read().await;
        let old_recipient = crate::onboarding::account(&tonk)
            .await
            .unwrap()
            .secret()
            .did();

        // Save the root record with its recipient, without asserting the
        // `AccountSealedInbox` fact `persist_test_root` would publish.
        let root = Ed25519Signer::import(&test_root_seed(&tonk.profile_name))
            .await
            .unwrap();
        let root_did = root.did();
        let grant = tonk_identity::delegation::mint_device_delegation(root, &tonk.profile.did())
            .await
            .unwrap();
        let recipient = tonk_identity::envelope::AccountSecret::from_bytes(
            zeroize::Zeroizing::new(test_root_seed(&tonk.profile_name)),
        )
        .secret()
        .did();
        super::super::identity::persist_root(
            &tonk,
            tonk_worker_api::SaveRootRequest {
                credential_id: "test-credential".to_string(),
                delegation_hex: hex::encode(grant.to_bytes().unwrap()),
                passkey: None,
                encryption_key: Some(recipient.to_string()),
            },
        )
        .await
        .unwrap();

        rotate_from_onboarding(&tonk).await;

        let prefix = super::super::repository::space_root_prefix(&tonk, &created)
            .await
            .unwrap();
        assert_eq!(prefix.audience(), &root_did, "the space is re-rooted");
        assert!(
            sealed_to(&tonk, &old_recipient).await.unwrap().is_empty(),
            "nothing stays sealed to the onboarding account",
        );
        assert_eq!(sealed_to(&tonk, &recipient).await.unwrap().len(), 1);
    }
}
