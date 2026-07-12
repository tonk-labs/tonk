//! Resolving and stamping a member's display name.
//!
//! The effective display name is the durable `ProfileName` override on
//! the profile meta branch, or a deterministic `petname(profile_did)`
//! when no override exists. This is what every `MemberName` write uses.

use dialog_query::{Output as _, Query, Term};
use tonk_common::log;
use tonk_schema::prelude::DidExt as _;
use tonk_schema::{ProfileName, petname};

use crate::RepositoryError;
use crate::worker::TonkState;
#[cfg(target_arch = "wasm32")]
use tonk_schema::{MemberName, Membership};

// The profile repository lives on `main` (it has no content/meta
// split); spaces re-stamp member names on their own `main` content
// branch.
const PROFILE_BRANCH: &str = "main";
// Only the wasm-gated rename handler re-stamps member names, so this and
// `restamp_member_name` exist only on the wasm target (the worker's real
// runtime). Gating them keeps the native `clippy -D warnings` build clean.
#[cfg(target_arch = "wasm32")]
const CONTENT_BRANCH: &str = "main";

/// The member's effective display name: stored override, else the
/// deterministic default derived from the profile DID.
pub(crate) async fn resolve_display_name(tonk: &TonkState) -> String {
    let profile_entity = tonk.profile.did().this();

    let session = match tonk
        .reactor
        .profile_repository()
        .branch(PROFILE_BRANCH)
        .acquire(&tonk.operator)
        .await
    {
        Ok(s) => s,
        Err(e) => {
            log!("resolve_display_name: meta acquire failed: {e}");
            return petname(&tonk.profile.did());
        }
    };

    let rows: Vec<ProfileName> = session
        .handle()
        .query()
        .select(Query::<ProfileName> {
            this: Term::from(profile_entity),
            name: Term::var("name"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .unwrap_or_default();

    rows.into_iter()
        .next()
        .map(|pn| pn.name.0)
        .unwrap_or_else(|| petname(&tonk.profile.did()))
}

/// Ensure a durable `ProfileName` exists on the profile meta branch.
///
/// [`resolve_display_name`] falls back to the deterministic `petname` when
/// no override is stored, but that fallback is computed, never persisted.
/// The FAB chrome renders the member name through a sealed profile-branch
/// `<tonk-display model="tonk:profile/name">`, which can only read the
/// branch DB — it has no path to the petname fallback, so the name slot is
/// blank until a rename writes a row. Stamping the petname once at bootstrap
/// fills that slot for a never-renamed member.
///
/// Idempotent and rename-safe: skips the write whenever any `ProfileName`
/// row already exists, so it never clobbers a user-chosen override (and a
/// later rename overwrites the petname, `cardinality: one`).
pub(crate) async fn ensure_display_name(tonk: &TonkState) -> Result<(), RepositoryError> {
    let profile_entity = tonk.profile.did().this();

    let session = tonk
        .reactor
        .profile_repository()
        .branch(PROFILE_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|e| {
            RepositoryError::Internal(format!("ensure_display_name: meta acquire failed: {e}"))
        })?;

    let existing: Vec<ProfileName> = session
        .handle()
        .query()
        .select(Query::<ProfileName> {
            this: Term::from(profile_entity.clone()),
            name: Term::var("name"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .unwrap_or_default();

    if !existing.is_empty() {
        return Ok(());
    }

    let name = petname(&tonk.profile.did());
    tonk.reactor
        .profile_repository()
        .branch(PROFILE_BRANCH)
        .transaction()
        .assert(ProfileName::new(profile_entity, name))
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            RepositoryError::Internal(format!("ensure_display_name: stamp petname failed: {e}"))
        })?;

    Ok(())
}

/// The routing keys of every space the profile belongs to.
///
/// Reads the profile's replica index off the meta branch (the same query
/// `get_profile` runs) and projects each space's routing key. The
/// self-replica (`subject == profile`) carries no roster, so it's skipped.
/// A single unparseable subject is logged and dropped rather than failing
/// the whole list.
#[cfg(target_arch = "wasm32")]
async fn profile_space_keys(tonk: &TonkState) -> Vec<String> {
    use dialog_varsig::Did;
    use tonk_schema::{Replica, domain::replica::Profile as ProfileEntity};

    let profile_did = tonk.profile.did();
    let profile_entity = profile_did.this();

    let session = match tonk
        .reactor
        .profile_repository()
        .branch(PROFILE_BRANCH)
        .acquire(&tonk.operator)
        .await
    {
        Ok(s) => s,
        Err(e) => {
            log!("profile_space_keys: meta acquire failed: {e}");
            return Vec::new();
        }
    };

    let rows: Vec<Replica> = session
        .handle()
        .query()
        .select(Query::<Replica> {
            this: Term::var("this"),
            subject: Term::var("subject"),
            profile: Term::from(ProfileEntity(profile_entity.clone())),
            kind: Term::var("kind"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .unwrap_or_default();

    let mut keys = Vec::new();
    for replica in rows {
        if replica.subject.0 == profile_entity {
            continue;
        }
        match replica.subject.0.to_string().parse::<Did>() {
            Ok(did) => keys.push(did.repo_key().to_owned()),
            Err(e) => log!(
                "profile_space_keys: unparseable subject {:?}: {e:?}",
                replica.subject.0
            ),
        }
    }
    keys
}

/// Re-stamp the self member's `MemberName` on every space the profile
/// belongs to.
///
/// A rename changes the profile's effective name; each space's roster
/// (`MemberName` on its synced `main` branch) must reflect it so peers on
/// every space see the new name — not just the space in focus when the
/// rename happened. One space's failure is logged and skipped so a single
/// unreachable branch can't block the rest.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn restamp_member_name_all_spaces(tonk: &TonkState, name: &str) {
    for key in profile_space_keys(tonk).await {
        if let Err(e) = restamp_member_name(tonk, &key, name).await {
            log!("restamp MemberName for space '{key}' failed: {e}");
        }
    }
}

/// Re-stamp the self-identity overlay (`state:self`) on every space the
/// profile belongs to, so the topbar chip reflects a rename instantly on
/// whichever space is in view.
///
/// A profile rename is fired from the FAB on the PROFILE branch, so the
/// command carries no origin space — it can't target just the one in focus.
/// The overlay is per-space (the chip reads it without seeing the profile
/// branch), so re-stamp them all; each is overlay-only and one space's
/// failure is logged inside [`crate::router::sync::publish_self_identity`].
#[cfg(target_arch = "wasm32")]
pub(crate) async fn restamp_self_identity_all_spaces(tonk: &TonkState) {
    for key in profile_space_keys(tonk).await {
        crate::router::sync::publish_self_identity(tonk, &key, CONTENT_BRANCH).await;
    }
}

/// Re-stamp the self member's `MemberName` on a space's content branch.
/// Used by [`restamp_member_name_all_spaces`] to update one space's roster.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn restamp_member_name(
    tonk: &TonkState,
    key: &str,
    name: &str,
) -> Result<(), RepositoryError> {
    // The repo (subject) DID identifies the membership entity. Read it
    // from the acquired content-branch handle (`of()` is the subject),
    // matching how sync.rs derives the replica subject.
    let session = tonk
        .reactor
        .repository(key)
        .branch(CONTENT_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|e| RepositoryError::Internal(format!("acquire content branch '{key}': {e}")))?;
    let repo_did = session.handle().of().clone();
    let membership = Membership::new(tonk.profile.did(), repo_did);

    tonk.reactor
        .repository(key)
        .branch(CONTENT_BRANCH)
        .transaction()
        .assert(MemberName::new(membership.this().clone(), name.to_string()))
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(|e| RepositoryError::Internal(format!("restamp member name for '{key}': {e}")))?;
    Ok(())
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_service_worker);

    use crate::worker::{DefaultSpace, TonkState};
    use dialog_capability::Subject;
    use dialog_operator::Profile;
    use dialog_storage::provider::storage::Storage;
    use tonk_schema::petname;

    /// Build an isolated `TonkState` backed by a unique IDB namespace.
    /// Tests that write to the profile meta branch must not share storage
    /// with each other — `Profile::open(name)` keys the IDB by `name`.
    async fn isolated_state(name: &str) -> TonkState {
        crate::patch_idb_versionchange();
        let storage = Storage::<DefaultSpace>::default();
        let profile = Profile::open(name)
            .perform(&storage)
            .await
            .expect("profile opens");
        let operator = profile
            .derive(b"test-worker")
            .allow(Subject::any())
            .build(storage)
            .await
            .expect("operator builds");
        let reactor = crate::Reactor::new(profile.clone());
        TonkState {
            profile,
            operator,
            profile_name: name.to_string(),
            reactor,
            view_bindings: Default::default(),
            bridges: Default::default(),
            sync_queue: Default::default(),
            commands: crate::router::command_registry(),
            sites: Default::default(),
        }
    }

    #[dialog_common::test]
    async fn it_defaults_to_the_petname_when_no_override() {
        let tonk = isolated_state("profile-name-test-default").await;
        let expected = petname(&tonk.profile.did());
        assert_eq!(resolve_display_name(&tonk).await, expected);
    }

    #[dialog_common::test]
    async fn ensure_stamps_the_petname_when_absent() {
        let tonk = isolated_state("profile-name-test-ensure-stamp").await;
        let expected = petname(&tonk.profile.did());
        // Nothing stored yet → the FAB read would be blank.
        ensure_display_name(&tonk).await.unwrap();
        // Now a durable row exists, so the branch read resolves the petname.
        assert_eq!(resolve_display_name(&tonk).await, expected);
    }

    #[dialog_common::test]
    async fn ensure_does_not_clobber_an_existing_override() {
        let tonk = isolated_state("profile-name-test-ensure-keep").await;
        let profile_entity = tonk.profile.did().this();
        tonk.reactor
            .profile_repository()
            .branch(PROFILE_BRANCH)
            .transaction()
            .assert(ProfileName::new(profile_entity, "brave-lynx".into()))
            .commit()
            .perform(&tonk.operator)
            .await
            .unwrap();
        ensure_display_name(&tonk).await.unwrap();
        assert_eq!(resolve_display_name(&tonk).await, "brave-lynx");
    }

    #[dialog_common::test]
    async fn it_returns_the_stored_override() {
        let tonk = isolated_state("profile-name-test-override").await;
        let profile_entity = tonk.profile.did().this();
        tonk.reactor
            .profile_repository()
            .branch(PROFILE_BRANCH)
            .transaction()
            .assert(ProfileName::new(profile_entity, "brave-lynx".into()))
            .commit()
            .perform(&tonk.operator)
            .await
            .unwrap();
        assert_eq!(resolve_display_name(&tonk).await, "brave-lynx");
    }
}
