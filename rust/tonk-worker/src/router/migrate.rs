//! Converge a linked device's existing device-keyed spaces onto the
//! account root DID.

#[cfg(target_arch = "wasm32")]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(target_arch = "wasm32")]
use dialog_query::{Output as _, Query, Term};
#[cfg(target_arch = "wasm32")]
use dialog_repository::RepositoryExt as _;
#[cfg(target_arch = "wasm32")]
use tonk_common::log;
#[cfg(target_arch = "wasm32")]
use tonk_schema::prelude::DidExt as _;
#[cfg(target_arch = "wasm32")]
use tonk_schema::{InvitedVia, MemberName, MemberRole, Membership};

#[cfg(target_arch = "wasm32")]
use crate::RepositoryError;
#[cfg(target_arch = "wasm32")]
use crate::router::account;
#[cfg(target_arch = "wasm32")]
use crate::worker::TonkState;

/// Only the wasm-gated migration path re-keys a roster, so this
/// (mirroring `profile_name.rs`) exists only on the wasm target — gating
/// it keeps the native `clippy -D warnings` build clean.
#[cfg(target_arch = "wasm32")]
const CONTENT_BRANCH: &str = "main";

/// Guards against concurrent migration sweeps. A device-link response
/// dispatches the sweep fire-and-forget; a second concurrent trigger
/// would only race the first through the same set of spaces. Mirrors
/// `restore.rs`'s `RESTORE_IN_FLIGHT` guard.
#[cfg(target_arch = "wasm32")]
static MIGRATE_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// Converge every existing device-keyed space onto the account root.
/// Best-effort: no-op when unlinked; one space's failure is logged and
/// skipped. A concurrent run is skipped (the guard), since both would
/// migrate the same spaces.
///
/// A space whose roster migration returns `Ok(true)` (a device-keyed row
/// was actually re-keyed) still has its capability chain anchored to this
/// device — [`reanchor_space`] re-anchors it to the root and backs it up
/// for the account's other devices. `Ok(false)` means the space was
/// already root-keyed (migrated on an earlier link, or claimed
/// post-account), so its chain already terminates at root and needs no
/// re-anchor.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn migrate_rosters(tonk: &TonkState) {
    if account::account_link(tonk).await.is_none() {
        return; // unlinked
    }
    if MIGRATE_IN_FLIGHT.swap(true, Ordering::SeqCst) {
        return;
    }
    for key in crate::router::profile_name::profile_space_keys(tonk).await {
        match migrate_space_roster(tonk, &key).await {
            Ok(true) => reanchor_space(tonk, &key).await,
            Ok(false) => {}
            Err(error) => log!("roster migration for space '{key}' skipped: {error}"),
        }
    }
    MIGRATE_IN_FLIGHT.store(false, Ordering::SeqCst);
}

/// Re-key one space's roster from the device DID to the account root DID,
/// atomically. Returns `Ok(true)` when a device-keyed row was migrated,
/// `Ok(false)` when the space is already root-keyed, the profile isn't a
/// member, or the profile is unlinked.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn migrate_space_roster(
    tonk: &TonkState,
    key: &str,
) -> Result<bool, RepositoryError> {
    let member = account::member_did(tonk).await;
    let device = tonk.profile.did();
    // Unlinked: no root to migrate to. (member_did == device DID.)
    if member == device {
        return Ok(false);
    }

    let session = tonk
        .reactor
        .repository(key)
        .branch(CONTENT_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|e| RepositoryError::Internal(format!("acquire content '{key}': {e}")))?;
    let subject = session.handle().of().clone();
    let subject_entity = subject.this();

    let device_membership = Membership::new(device.clone(), subject.clone());
    let device_entity = device_membership.this().clone();

    let root_membership = Membership::new(member.clone(), subject.clone());
    let root_entity = root_membership.this().clone();

    // Is there a device-keyed row to migrate?
    let memberships: Vec<Membership> = session
        .handle()
        .query()
        .select(Query::<Membership> {
            this: Term::var("this"),
            subject: Term::from(subject_entity.clone()),
            member: Term::var("member"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|e| RepositoryError::Internal(format!("membership query '{key}': {e:?}")))?;
    let Some(device_row) = memberships.into_iter().find(|m| m.this == device_entity) else {
        return Ok(false);
    };

    // Read the device row's stamps so they can be copied and retracted.
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
        .map_err(|e| RepositoryError::Internal(format!("role query '{key}': {e:?}")))?;
    let device_role = roles.iter().find(|r| r.this == device_entity).cloned();

    let names: Vec<MemberName> = session
        .handle()
        .query()
        .select(Query::<MemberName> {
            this: Term::var("this"),
            name: Term::var("name"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|e| RepositoryError::Internal(format!("name query '{key}': {e:?}")))?;
    let device_name = names.into_iter().find(|n| n.this == device_entity);

    let stamps: Vec<InvitedVia> = session
        .handle()
        .query()
        .select(Query::<InvitedVia> {
            this: Term::var("this"),
            invitation: Term::var("invitation"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|e| RepositoryError::Internal(format!("invited-via query '{key}': {e:?}")))?;
    // Same first-wins reasoning as the role guard above.
    let root_stamp_exists = stamps.iter().any(|s| s.this == root_entity);
    let device_stamp = stamps.into_iter().find(|s| s.this == device_entity);

    // Build the root-keyed rows and one atomic assert+retract transaction.
    let mut txn = tonk
        .reactor
        .repository(key)
        .branch(CONTENT_BRANCH)
        .transaction()
        .assert(root_membership.clone())
        .retract(device_row);

    if let Some(role) = device_role {
        // Founder-wins convergence, order-independent: a founder assertion
        // wins over an existing member, a member never demotes an existing
        // founder, and a member is stamped only when the root has no role
        // yet. The device row is retracted regardless of which role wins.
        let device_is_founder = role.role.0.to_string() == MemberRole::FOUNDER;
        let root_is_founder = roles
            .iter()
            .any(|r| r.this == root_entity && r.role.0.to_string() == MemberRole::FOUNDER);
        let root_has_role = roles.iter().any(|r| r.this == root_entity);

        if device_is_founder && !root_is_founder {
            txn = txn.assert(MemberRole::founder(root_entity.clone())); // upgrade / first founder
        } else if !device_is_founder && !root_has_role {
            txn = txn.assert(MemberRole::member(root_entity.clone())); // first member
        }
        txn = txn.retract(role);
    }
    if let Some(name) = device_name {
        // Last-wins: unlike role/provenance, a display name has no
        // "first stamp wins" ownership concern, so this device's name
        // simply overwrites whatever the root entity carried before.
        txn = txn
            .assert(MemberName::new(root_entity.clone(), name.name.0.clone()))
            .retract(name);
    }
    if let Some(stamp) = device_stamp {
        // First-wins, same reasoning as the role guard above: a prior
        // migration's invitation provenance for the root entity must not
        // be overwritten by a later device's migration.
        if !root_stamp_exists {
            txn = txn.assert(InvitedVia::new(
                root_entity.clone(),
                stamp.invitation.0.clone(),
            ));
        }
        txn = txn.retract(stamp);
    }

    txn.commit()
        .perform(&tonk.operator)
        .await
        .map_err(|e| RepositoryError::Internal(format!("commit migration '{key}': {e}")))?;
    log!("migrated roster for space '{key}' to the account root");
    Ok(true)
}

/// Re-anchor a just-migrated space's capability chain to the account root
/// and back it up so the account's other devices can restore it.
/// Best-effort, fire-and-forget: any failure is logged and swallowed —
/// the roster is already root-keyed regardless of whether this succeeds.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn reanchor_space(tonk: &TonkState, key: &str) {
    if let Err(error) = try_reanchor_space(tonk, key).await {
        log!("re-anchor of space '{key}' skipped: {error}");
    }
}

#[cfg(target_arch = "wasm32")]
async fn try_reanchor_space(tonk: &TonkState, key: &str) -> Result<(), RepositoryError> {
    let Some(root_did) = account::account_root_did(tonk).await else {
        return Ok(());
    };
    let repository = tonk
        .profile
        .repository(key)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|e| RepositoryError::Internal(format!("load space '{key}': {e}")))?;

    // Recover the sync URL off the content branch's (`main`) own stored
    // upstream remote config — the same recovery
    // `create_invite::resolve_remote_url` already does for the
    // invite-mint path.
    let remote_url = match crate::router::create_invite::resolve_remote_url(tonk, &repository)
        .await
        .map_err(|e| RepositoryError::Internal(format!("resolve remote for '{key}': {e}")))?
    {
        crate::router::create_invite::RemoteRequirement::Ready(url) => url.to_string(),
        // No shareable remote: no other device could restore this space
        // from a backup anyway, so there is nothing to back up.
        crate::router::create_invite::RemoteRequirement::Refused(_) => return Ok(()),
    };

    match repository.try_access() {
        // Created/owned: mint the one-hop space -> root delegation and
        // back it up (reuses the restore branch's helper).
        Some(_) => {
            crate::router::account_backup::back_up_owned_space(tonk, &repository, &remote_url)
                .await;
        }
        // Claimed: the profile re-delegates its held capability to the
        // root, composing space -> eph -> device -> root. Save it to the
        // access store, then back it up.
        None => {
            let chain: dialog_ucan::UcanDelegation = tonk
                .profile
                .access()
                .claim(&repository)
                .delegate(root_did)
                .perform(&tonk.operator)
                .await
                .map_err(|e| RepositoryError::Internal(format!("re-anchor '{key}': {e}")))?;
            tonk.profile
                .access()
                .save(chain.clone())
                .perform(&tonk.operator)
                .await
                .map_err(|e| RepositoryError::Internal(format!("save re-anchor '{key}': {e}")))?;
            crate::router::account_backup::back_up_reanchored(
                tonk,
                chain.into_chain(),
                &remote_url,
            )
            .await;
        }
    }
    Ok(())
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use axum::Json;
    use axum::extract::State;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_service_worker);

    use dialog_artifacts::Entity;
    use tonk_schema::prelude::{DidExt as _, EntityExt as _};
    use tonk_schema::{InvitedVia, MemberName, MemberRole, Membership};

    use super::migrate_space_roster;
    use crate::router::api_router_with_state;
    use crate::router::tests::{
        content_invited_via, content_member_names, content_member_roles, content_memberships,
        put_repo, test_state,
    };

    /// Link the given (already-open) state's profile to a fresh account
    /// root, mirroring `join.rs`'s
    /// `it_keys_membership_on_the_root_did_for_an_account_holder` setup.
    /// Returns the minted root DID.
    async fn link_account(state: &crate::router::AppState) -> dialog_varsig::Did {
        use dialog_varsig::Principal as _;
        let device_did = state.read().await.profile.did();
        let root = tonk_identity::derive::derive_root_signer(&[9u8; 32])
            .await
            .unwrap();
        let root_did = root.did();
        let delegation = tonk_identity::delegation::mint_device_delegation(root, &device_did)
            .await
            .unwrap();
        let request = tonk_worker_api::AccountLinkRequest {
            root_did: root_did.to_string(),
            delegation_hex: hex::encode(delegation.to_bytes().unwrap()),
        };
        let _ = crate::router::account::link(State(state.clone()), Json(request))
            .await
            .unwrap();
        root_did
    }

    /// Seed a device-keyed founder membership directly on `key`'s content
    /// branch — the state migration is meant to converge away from.
    ///
    /// The `InvitedVia` stamp points at a bare stable `Entity` rather
    /// than a full `Invitation` — mirroring `membership.rs`'s own
    /// `it_stamps_the_membership_entity` test — since the migration
    /// only ever copies the invitation reference, never dereferences
    /// it.
    async fn seed_device_membership(
        state: &crate::router::AppState,
        key: &str,
        device_did: &dialog_varsig::Did,
        subject_did: &dialog_varsig::Did,
    ) {
        let tonk = state.read().await;
        let membership = Membership::new(device_did.clone(), subject_did.clone());
        let entity = membership.this().clone();
        let invitation_entity = Entity::of(&"migrate-roster-invitation");
        tonk.reactor
            .repository(key)
            .branch("main")
            .transaction()
            .assert(membership)
            .assert(MemberRole::founder(entity.clone()))
            .assert(MemberName::new(entity.clone(), "Device Member".to_string()))
            .assert(InvitedVia::new(entity, invitation_entity))
            .commit()
            .perform(&tonk.operator)
            .await
            .unwrap();
    }

    #[dialog_common::test]
    async fn it_rekeys_a_device_membership_onto_the_account_root() {
        let state = test_state().await;
        let (app, state, _lsp) = api_router_with_state(state);

        let device_did = state.read().await.profile.did();
        let key = put_repo(&app, "migrate-roster-founder").await;
        let subject_did = {
            let tonk = state.read().await;
            tonk.reactor
                .repository(&key)
                .branch("main")
                .acquire(&tonk.operator)
                .await
                .unwrap()
                .handle()
                .of()
                .clone()
        };
        let device_membership = Membership::new(device_did.clone(), subject_did.clone());
        let device_entity = device_membership.this().clone();
        seed_device_membership(&state, &key, &device_did, &subject_did).await;

        let root_did = link_account(&state).await;

        let migrated = {
            let tonk = state.read().await;
            migrate_space_roster(&tonk, &key).await.unwrap()
        };
        assert!(migrated, "expected a device-keyed row to be migrated");

        let memberships = content_memberships(&state, &key).await;
        assert!(
            memberships.iter().any(|m| m.member.0 == root_did.this()),
            "root-keyed membership must exist after migration",
        );
        assert!(
            !memberships.iter().any(|m| m.member.0 == device_did.this()),
            "device-keyed membership must be gone after migration",
        );

        let roles = content_member_roles(&state, &key).await;
        let root_membership = memberships
            .iter()
            .find(|m| m.member.0 == root_did.this())
            .expect("root membership present");
        assert!(
            roles
                .iter()
                .any(|r| r.this == root_membership.this
                    && r.role.0.to_string() == MemberRole::FOUNDER),
            "founder role must carry over onto the root-keyed entity",
        );

        let names = content_member_names(&state, &key).await;
        assert!(
            names
                .iter()
                .any(|n| n.this == root_membership.this && n.name.0 == "Device Member"),
            "member name must carry over onto the root-keyed entity",
        );
        assert!(
            !names.iter().any(|n| n.this == device_entity),
            "member name must be gone from the device-keyed entity",
        );

        let stamps = content_invited_via(&state, &key).await;
        let root_stamp = stamps
            .iter()
            .find(|s| s.this == root_membership.this)
            .expect("invited-via stamp must carry over onto the root-keyed entity");
        assert!(
            !stamps.iter().any(|s| s.this == device_entity),
            "invited-via stamp must be gone from the device-keyed entity",
        );
        assert_eq!(
            root_stamp.invitation.0,
            Entity::of(&"migrate-roster-invitation"),
            "invited-via must reference the same invitation entity as before migration",
        );

        // Idempotent: a second call finds no device-keyed row left to migrate.
        let migrated_again = {
            let tonk = state.read().await;
            migrate_space_roster(&tonk, &key).await.unwrap()
        };
        assert!(!migrated_again, "second migration call must be a no-op");
    }

    #[dialog_common::test]
    async fn it_is_a_noop_when_unlinked() {
        let state = test_state().await;
        let (app, state, _lsp) = api_router_with_state(state);

        let device_did = state.read().await.profile.did();
        let key = put_repo(&app, "migrate-roster-unlinked").await;
        let subject_did = {
            let tonk = state.read().await;
            tonk.reactor
                .repository(&key)
                .branch("main")
                .acquire(&tonk.operator)
                .await
                .unwrap()
                .handle()
                .of()
                .clone()
        };
        seed_device_membership(&state, &key, &device_did, &subject_did).await;

        // No account link — member_did == device DID, so there is
        // nothing to converge.
        let migrated = {
            let tonk = state.read().await;
            migrate_space_roster(&tonk, &key).await.unwrap()
        };
        assert!(!migrated, "unlinked profile has no root to migrate to");
    }

    /// First-wins guard: a second linked device's migration must not
    /// demote a founder role another migration (or a direct root-keyed
    /// join) has already stamped onto the root entity.
    #[dialog_common::test]
    async fn it_does_not_demote_an_existing_root_founder_role() {
        let state = test_state().await;
        let (app, state, _lsp) = api_router_with_state(state);

        let device_did = state.read().await.profile.did();
        let key = put_repo(&app, "migrate-roster-first-wins").await;
        let subject_did = {
            let tonk = state.read().await;
            tonk.reactor
                .repository(&key)
                .branch("main")
                .acquire(&tonk.operator)
                .await
                .unwrap()
                .handle()
                .of()
                .clone()
        };

        let root_did = link_account(&state).await;

        // Simulate a prior migration (or a direct root-keyed join): the
        // root entity already carries a founder role.
        let root_membership = Membership::new(root_did.clone(), subject_did.clone());
        let root_entity = root_membership.this().clone();
        {
            let tonk = state.read().await;
            tonk.reactor
                .repository(&key)
                .branch("main")
                .transaction()
                .assert(root_membership)
                .assert(MemberRole::founder(root_entity.clone()))
                .commit()
                .perform(&tonk.operator)
                .await
                .unwrap();
        }

        // This device still has its own device-keyed *member* row to
        // migrate — a lesser role than the root's existing founder.
        let device_membership = Membership::new(device_did.clone(), subject_did.clone());
        let device_entity = device_membership.this().clone();
        {
            let tonk = state.read().await;
            tonk.reactor
                .repository(&key)
                .branch("main")
                .transaction()
                .assert(device_membership)
                .assert(MemberRole::member(device_entity.clone()))
                .commit()
                .perform(&tonk.operator)
                .await
                .unwrap();
        }

        let migrated = {
            let tonk = state.read().await;
            migrate_space_roster(&tonk, &key).await.unwrap()
        };
        assert!(migrated, "expected the device-keyed row to be migrated");

        let roles = content_member_roles(&state, &key).await;
        assert!(
            roles
                .iter()
                .any(|r| r.this == root_entity && r.role.0.to_string() == MemberRole::FOUNDER),
            "root founder role must survive a later device's migration unchanged",
        );
        assert!(
            !roles.iter().any(|r| r.this == device_entity),
            "device-keyed role must be gone after migration",
        );

        let memberships = content_memberships(&state, &key).await;
        assert!(
            !memberships.iter().any(|m| m.this == device_entity),
            "device-keyed membership must be gone after migration",
        );
    }

    /// Founder-wins upgrade: a linked device's *founder* row must upgrade
    /// the root entity's role even when another device already stamped a
    /// lesser *member* role there first (self-healing, order-independent
    /// convergence — the mirror image of the demotion-guard test above).
    #[dialog_common::test]
    async fn it_upgrades_an_existing_root_member_role_to_founder() {
        let state = test_state().await;
        let (app, state, _lsp) = api_router_with_state(state);

        let device_did = state.read().await.profile.did();
        let key = put_repo(&app, "migrate-roster-upgrade").await;
        let subject_did = {
            let tonk = state.read().await;
            tonk.reactor
                .repository(&key)
                .branch("main")
                .acquire(&tonk.operator)
                .await
                .unwrap()
                .handle()
                .of()
                .clone()
        };

        let root_did = link_account(&state).await;

        // Simulate a prior migration (or a direct root-keyed join) that
        // only ever recorded a plain member role on the root entity.
        let root_membership = Membership::new(root_did.clone(), subject_did.clone());
        let root_entity = root_membership.this().clone();
        {
            let tonk = state.read().await;
            tonk.reactor
                .repository(&key)
                .branch("main")
                .transaction()
                .assert(root_membership)
                .assert(MemberRole::member(root_entity.clone()))
                .commit()
                .perform(&tonk.operator)
                .await
                .unwrap();
        }

        // This device holds the true founder row for the space — its
        // migration must upgrade the root to founder, not be blocked by
        // the root's existing (lesser) member role.
        let device_membership = Membership::new(device_did.clone(), subject_did.clone());
        let device_entity = device_membership.this().clone();
        {
            let tonk = state.read().await;
            tonk.reactor
                .repository(&key)
                .branch("main")
                .transaction()
                .assert(device_membership)
                .assert(MemberRole::founder(device_entity.clone()))
                .commit()
                .perform(&tonk.operator)
                .await
                .unwrap();
        }

        let migrated = {
            let tonk = state.read().await;
            migrate_space_roster(&tonk, &key).await.unwrap()
        };
        assert!(migrated, "expected the device-keyed row to be migrated");

        let roles = content_member_roles(&state, &key).await;
        assert!(
            roles
                .iter()
                .any(|r| r.this == root_entity && r.role.0.to_string() == MemberRole::FOUNDER),
            "root role must be upgraded to founder by the later device's migration",
        );
        assert!(
            !roles.iter().any(|r| r.this == device_entity),
            "device-keyed role must be gone after migration",
        );

        let memberships = content_memberships(&state, &key).await;
        assert!(
            !memberships.iter().any(|m| m.this == device_entity),
            "device-keyed membership must be gone after migration",
        );
    }
}
