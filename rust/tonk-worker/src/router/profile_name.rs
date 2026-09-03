//! Resolving and stamping a member's display name.
//!
//! The effective display name is the durable `ProfileName` override on
//! the profile meta branch, or a deterministic `petname(profile_did)`
//! when no override exists. This is what every `MemberName` write uses.

use dialog_query::{Output as _, Query, Term};
use dialog_repository::Repository;
use tonk_common::log;
use tonk_schema::prelude::DidExt as _;
use tonk_schema::{ProfileName, petname};

use crate::RepositoryError;
use crate::worker::{DefaultOperator, TonkState};
use dialog_operator::Profile;
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
    resolve_display_name_from(&tonk.profile, &tonk.operator).await
}

/// Resolve the effective display name for an explicit profile without
/// booting it as the active worker state.
pub(crate) async fn resolve_display_name_from(
    profile: &Profile,
    operator: &DefaultOperator,
) -> String {
    let profile_entity = profile.did().this();

    let branch = match Repository::from(profile)
        .branch(PROFILE_BRANCH)
        .open()
        .perform(operator)
        .await
    {
        Ok(branch) => branch,
        Err(e) => {
            log!("resolve_display_name: meta acquire failed: {e}");
            return petname(&profile.did());
        }
    };

    let rows: Vec<ProfileName> = branch
        .query()
        .select(Query::<ProfileName> {
            this: Term::from(profile_entity),
            name: Term::var("name"),
        })
        .perform(operator)
        .try_vec()
        .await
        .unwrap_or_default();

    rows.into_iter()
        .next()
        .map(|pn| pn.name.0)
        .unwrap_or_else(|| petname(&profile.did()))
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

/// The routing keys of every real space the profile belongs to.
///
/// Reads the profile's replica index off the meta branch (the same query
/// `get_profile` runs) and projects only `tonk:repository` routing keys.
/// Profile and account system replicas carry no user-space roster.
/// A single unparseable subject is logged and dropped rather than failing
/// the whole list.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn real_space_keys(tonk: &TonkState) -> Vec<String> {
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
            log!("real_space_keys: meta acquire failed: {e}");
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
        if replica.kind != Replica::repository_kind() {
            continue;
        }
        match replica.subject.0.to_string().parse::<Did>() {
            Ok(did) => keys.push(did.repo_key().to_owned()),
            Err(e) => log!(
                "real_space_keys: unparseable subject {:?}: {e:?}",
                replica.subject.0
            ),
        }
    }
    keys
}

/// Project `name` onto `member`'s `MemberName` in one space's roster.
///
/// Idempotent: reads the roster first and commits only when the row is
/// missing or stale, so the sweep can run it on every pass without
/// touching the branch. A linked profile's rename also clears the
/// device-keyed row a pre-link join left behind (cardinality-one on a
/// different entity, so the assert alone would not overwrite it).
///
/// Returns whether anything was written, so the caller knows to queue
/// the space for sync.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn project_member_name(
    tonk: &TonkState,
    key: &str,
    member: &dialog_varsig::Did,
    name: &str,
) -> Result<bool, RepositoryError> {
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
    let membership = Membership::new(member.clone(), repo_did.clone());
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

    let stale = !names
        .iter()
        .any(|row| row.this == *membership.this() && row.name.0 == name);
    let device = tonk.profile.did();
    let obsolete: Vec<MemberName> = if *member == device {
        Vec::new()
    } else {
        let device_entity = Membership::new(device, repo_did).this().clone();
        names
            .into_iter()
            .filter(|row| row.this == device_entity)
            .collect()
    };
    if !stale && obsolete.is_empty() {
        return Ok(false);
    }

    let mut txn = tonk
        .reactor
        .repository(key)
        .branch(CONTENT_BRANCH)
        .transaction();
    if stale {
        txn = txn.assert(MemberName::new(membership.this().clone(), name.to_string()));
    }
    for row in obsolete {
        txn = txn.retract(row);
    }
    txn.commit()
        .perform(&tonk.operator)
        .await
        .map_err(|e| RepositoryError::Internal(format!("project member name to '{key}': {e}")))?;
    Ok(true)
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_service_worker);

    use crate::worker::{DefaultSpace, TonkState};
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
        let session = crate::session::open(&profile, &storage)
            .await
            .expect("signing session opens");
        let reactor = crate::Reactor::new(profile.clone());
        TonkState {
            profile,
            operator: session.operator,
            storage,
            session_expires_at: session.expires_at,
            profile_name: name.to_string(),
            reactor,
            retiring: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            view_bindings: Default::default(),
            bridges: Default::default(),
            sync_queue: Default::default(),
            commands: crate::router::command_registry(),
            clients: Default::default(),
            account_keys: Default::default(),
            registry: crate::device::Registry {
                profile: name.to_string(),
                directory: dialog_effects::storage::Directory::Profile,
            },
            profile_transition: Default::default(),
            context_generation: Default::default(),
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

    /// A rename after linking a device to an account root must move the
    /// space's roster row rather than duplicate it: the founder membership
    /// was stamped on the device DID (the account was unlinked when the
    /// space was created), so [`project_member_name`] has to key the new
    /// `MemberName` on the root and retract the now-orphaned device-keyed
    /// row.
    #[dialog_common::test]
    async fn it_rekeys_the_roster_name_to_the_root_and_retracts_the_device_row() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use dialog_varsig::Did;
        use tower::ServiceExt;

        let (app, state, _lsp) =
            crate::router::api_router_with_state(crate::router::tests::test_state().await);

        // Create the space while unlinked: the founder membership is
        // stamped on the device DID.
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/repository/rename-rekey-test")
                    .method("PUT")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let info: crate::router::RepositoryInfo = serde_json::from_slice(&body).unwrap();
        let key = info.name;
        let device_did = state.read().await.profile.did();

        let root_did_str = {
            let tonk = state.read().await;
            crate::router::identity::root_did(&tonk)
                .await
                .unwrap()
                .to_string()
        };

        let tonk = state.read().await;
        let root_did: Did = root_did_str.parse().unwrap();
        assert!(
            project_member_name(&tonk, &key, &root_did, "brave-lynx")
                .await
                .expect("projection succeeds"),
            "a rekey is a write"
        );

        let session = tonk
            .reactor
            .repository(&key)
            .branch(CONTENT_BRANCH)
            .acquire(&tonk.operator)
            .await
            .unwrap();
        let repo_did = session.handle().of().clone();
        let root_entity = Membership::new(root_did, repo_did.clone()).this().clone();
        let device_entity = Membership::new(device_did, repo_did).this().clone();

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
            .unwrap();

        assert_eq!(
            names
                .iter()
                .find(|n| n.this == root_entity)
                .map(|n| n.name.0.as_str()),
            Some("brave-lynx"),
            "the renamed name lands on the root-keyed roster row",
        );
        assert!(
            names.iter().all(|n| n.this != device_entity),
            "the stale device-keyed MemberName is retracted",
        );
    }
}
