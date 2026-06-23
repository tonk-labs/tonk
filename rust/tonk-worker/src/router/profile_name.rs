//! Resolving and stamping a member's display name.
//!
//! The effective display name is the durable `ProfileName` override on
//! the profile meta branch, or a deterministic `petname(profile_did)`
//! when no override exists. This is what every `MemberName` write uses.

use dialog_query::{Output as _, Query, Term};
use tonk_common::log;
use tonk_schema::prelude::DidExt as _;
use tonk_schema::{MemberName, Membership, ProfileName, petname};

use crate::RepositoryError;
use crate::worker::TonkState;

// The meta and content branch name constants, as used throughout router code.
const META_BRANCH: &str = "meta";
#[allow(dead_code)]
const CONTENT_BRANCH: &str = "main";

/// The member's effective display name: stored override, else the
/// deterministic default derived from the profile DID.
pub(crate) async fn resolve_display_name(tonk: &TonkState) -> String {
    let profile_entity = tonk.profile.did().this();

    let session = match tonk
        .reactor
        .profile_repository()
        .branch(META_BRANCH)
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

/// Re-stamp the self member's `MemberName` on a space's content branch.
/// Used by the rename handler so the current space's roster updates.
#[allow(dead_code)]
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
        .map_err(|e| {
            RepositoryError::Internal(format!("restamp member name for '{key}': {e}"))
        })?;
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
            commands: crate::router::command_registry(),
        }
    }

    #[dialog_common::test]
    async fn it_defaults_to_the_petname_when_no_override() {
        let tonk = isolated_state("profile-name-test-default").await;
        let expected = petname(&tonk.profile.did());
        assert_eq!(resolve_display_name(&tonk).await, expected);
    }

    #[dialog_common::test]
    async fn it_returns_the_stored_override() {
        let tonk = isolated_state("profile-name-test-override").await;
        let profile_entity = tonk.profile.did().this();
        tonk.reactor
            .profile_repository()
            .branch(META_BRANCH)
            .transaction()
            .assert(ProfileName::new(profile_entity, "brave-lynx".into()))
            .commit()
            .perform(&tonk.operator)
            .await
            .unwrap();
        assert_eq!(resolve_display_name(&tonk).await, "brave-lynx");
    }
}
