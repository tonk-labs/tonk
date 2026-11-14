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

// Only `project_member_name` needs it now, and that is wasm-only: nothing
// on the native target writes a name any more.
#[cfg(target_arch = "wasm32")]
use crate::RepositoryError;
use crate::worker::DefaultProfile;
use crate::worker::{DefaultOperator, TonkState};
#[cfg(target_arch = "wasm32")]
use tonk_schema::{MemberName, Membership};

// Spaces re-stamp member names on their own `main` content branch,
// which is the same name the profile's content branch carries.
// Only the wasm-gated rename handler re-stamps member names, so this and
// `restamp_member_name` exist only on the wasm target (the worker's real
// runtime). Gating them keeps the native `clippy -D warnings` build clean.
#[cfg(target_arch = "wasm32")]
const CONTENT_BRANCH: &str = "main";

/// The member's effective display name: stored override, else the
/// deterministic default derived from the profile DID.
///
/// The petname here is for places that must put SOME name on a person for
/// other people to read — a membership row, a roster entry projected into
/// a space. Anything describing this profile to its own UI wants
/// [`stored_display_name`] instead, so an unnamed profile reads as
/// unnamed rather than as a generated word.
pub(crate) async fn resolve_display_name(tonk: &TonkState) -> String {
    stored_display_name_from(&tonk.profile, &tonk.operator, &tonk.active_branch)
        .await
        .unwrap_or_else(|| petname(&tonk.profile.did()))
}

/// The stored name for an explicit profile, or `None` when none is set.
///
/// Read off `branch`, the one the profile is on: each account's branch
/// carries its own name, so a fixed branch would answer with another
/// account's.
pub(crate) async fn stored_display_name_from(
    profile: &DefaultProfile,
    operator: &DefaultOperator,
    branch: &str,
) -> Option<String> {
    let profile_entity = profile.did().this();

    let branch = match Repository::from(profile.did())
        .branch(branch)
        .open()
        .perform(operator)
        .await
    {
        Ok(branch) => branch,
        Err(e) => {
            log!("resolve_display_name: meta acquire failed: {e}");
            return None;
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
        .filter(|name| !name.trim().is_empty())
}

/// The routing keys of every real space the profile belongs to.
///
/// Reads the profile's replica index off the meta branch (the same query
/// `get_profile` runs) and projects only `tonk:repository` routing keys.
/// Profile and account system replicas carry no user-space roster.
/// A single unparseable subject is logged and dropped rather than failing
/// the whole list.
pub(crate) async fn real_space_keys(tonk: &TonkState) -> Vec<String> {
    real_space_keys_on(tonk, &tonk.active_branch).await
}

/// [`real_space_keys`] for the profile branch `branch`, which need not be
/// the active one.
pub(crate) async fn real_space_keys_on(tonk: &TonkState, branch: &str) -> Vec<String> {
    use dialog_varsig::Did;
    use tonk_schema::{Replica, domain::replica::Profile as ProfileEntity};

    let profile_did = tonk.profile.did();
    let profile_entity = profile_did.this();

    let session = match tonk
        .reactor
        .profile_repository()
        .branch(branch)
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
/// touching the branch. First reconciles a retired onboarding founder's
/// complete membership bundle when retained grants establish the association.
/// A linked profile's rename also clears the device-keyed row a pre-link join left behind (cardinality-one on a
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
    let repaired = super::rotation::reconcile_founder_membership(tonk, &repo_did, member)
        .await
        .map_err(|error| {
            RepositoryError::Internal(format!("reconcile founder in '{key}': {error}"))
        })?;
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
        return Ok(repaired);
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
