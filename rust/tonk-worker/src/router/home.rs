//! Home repo as the profile's meta-index of registered repos.
//!
//! Tonk tracks which repos a profile has created or joined by
//! asserting a claim in the profile's `home` repo:
//!
//! ```text
//! the = "tonk/repo"
//! of  = <profile DID>
//! is  = <local repo name>
//! ```
//!
//! `GET /api/repositories` reads these claims; `PUT /api/repository/{name}`
//! and `POST /api/claim` assert them after a successful create.
//!
//! Home is itself a dialog repo — synced and versioned like any other —
//! which is why enumeration lives here rather than in `dialog-repository`:
//! a storage adapter is scoped to a single repo, and cross-repo listing
//! isn't something dialog-db is trying to solve.

use dialog_artifacts::{Attribute, Entity, Value};
use dialog_repository::RepositoryExt as _;
use futures_util::StreamExt;
use tonk_common::log;

use super::claim::RawClaim;
use crate::TonkWorkerError;
use crate::worker::TonkState;

/// Local name of the meta / home repo. TonkShell is responsible for
/// PUT-ing this on startup; all other create paths assume it exists.
pub const DEFAULT_REPO: &str = "home";

/// Branch in `home` that holds the registered-repo claims.
const DEFAULT_BRANCH: &str = "main";

/// Attribute asserted on the profile DID for each registered repo.
/// Namespace/name form is required by `Attribute`'s parser.
const REGISTERED_REPO_ATTR: &str = "tonk/repo";

/// Assert a claim in home registering `local_name` against the profile DID.
///
/// Called after any successful repo create (PUT) or claim (invite). Fails
/// loudly if home isn't openable — the invariant is that TonkShell has
/// already bootstrapped it.
pub(super) async fn register_repo(
    tonk: &TonkState,
    local_name: &str,
) -> Result<(), TonkWorkerError> {
    let home = tonk
        .profile
        .repository(DEFAULT_REPO)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!(
                "cannot register '{local_name}': home repo not opened: {e}"
            ))
        })?;

    let main = home
        .branch(DEFAULT_BRANCH)
        .open()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!(
                "cannot register '{local_name}': failed to open home/main: {e}"
            ))
        })?;

    let entity = profile_entity(tonk)?;
    let attribute: Attribute = REGISTERED_REPO_ATTR.parse().map_err(|e| {
        TonkWorkerError::Internal(format!("invalid registered-repo attribute: {e}"))
    })?;

    main.transaction()
        .assert(RawClaim {
            the: attribute,
            of: entity,
            is: Value::String(local_name.to_string()),
        })
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("failed to register '{local_name}': {e}"))
        })?;

    log!("Registered '{}' in home", local_name);
    Ok(())
}

/// Select the local names of every repo registered to this profile.
///
/// Returns the values of `(profile_did, tonk/repo, ?)` claims from
/// `home/main`. Empty if home exists but has no registrations yet.
pub(super) async fn list_registered(tonk: &TonkState) -> Result<Vec<String>, TonkWorkerError> {
    let home = tonk
        .profile
        .repository(DEFAULT_REPO)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("cannot list repositories: home not opened: {e}"))
        })?;

    let main = home
        .branch(DEFAULT_BRANCH)
        .open()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!(
                "cannot list repositories: failed to open home/main: {e}"
            ))
        })?;

    let entity = profile_entity(tonk)?;
    let attribute: Attribute = REGISTERED_REPO_ATTR.parse().map_err(|e| {
        TonkWorkerError::Internal(format!("invalid registered-repo attribute: {e}"))
    })?;

    let selector = dialog_artifacts::ArtifactSelector::new()
        .the(attribute)
        .of(entity);

    let stream = main
        .claims()
        .select(selector)
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("registered-repo query failed: {e}")))?;

    tokio::pin!(stream);

    let mut names = Vec::new();
    while let Some(result) = stream.next().await {
        match result {
            Ok(artifact) => {
                if let Value::String(name) = artifact.is {
                    names.push(name);
                }
            }
            Err(e) => log!("ignoring registered-repo artifact read error: {:?}", e),
        }
    }

    Ok(names)
}

fn profile_entity(tonk: &TonkState) -> Result<Entity, TonkWorkerError> {
    tonk.profile
        .did()
        .to_string()
        .parse::<Entity>()
        .map_err(|e| TonkWorkerError::Internal(format!("profile DID is not a valid entity: {e}")))
}
