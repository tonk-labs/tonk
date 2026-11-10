//! What an earlier release kept in the app's origin, read for the kernel
//! build to bring over.
//!
//! That release ran one worker on the app's origin: a registry profile,
//! `tonk`, the roster of every profile this browser made, and each profile's
//! spaces, all in that origin's storage. The kernel build keeps a profile and
//! each space on an origin of its own, which cannot read the app's. So the
//! app's page reads them here and hands each space over to be made again as
//! a copy.
//!
//! Only a space kept on this device alone is read: one whose `main` has
//! content and no upstream. A space that syncs comes back with the account
//! it syncs for, when the person signs in. Nothing here deletes what it read.

use std::collections::HashSet;

use dialog_capability::Subject;
use dialog_common::Blake3Hash;
use dialog_effects::storage::{Directory, Location};
use dialog_peer::Peer;
use dialog_query::{Output as _, Query, Term};
use dialog_reactor::ProfileUpgrade;
use dialog_repository::{
    ACCESS_BRANCH, HeldCaches, Repository, RepositoryExt as _, Upstream, codec, spaces,
};
use dialog_storage::provider::storage::Storage;
use dialog_ucan_core::time::Timestamp;
use dialog_varsig::{Did, Principal as _};
use tonk_account::peer::{open_peer_holding, open_system, upstream};
use tonk_schema::prelude::DidExt as _;
use tonk_schema::{DeviceProfile, RepositoryName};

/// The storage the earlier release kept everything in.
#[cfg(target_arch = "wasm32")]
type Kept = dialog_storage::provider::storage::WebSpace;
#[cfg(not(target_arch = "wasm32"))]
type Kept = dialog_storage::provider::storage::NativeSpace;

/// The profile the earlier release's registry is, and the one a browser
/// signed as until it first switched.
pub const REGISTRY_PROFILE: &str = "tonk";

/// The registry's branch listing every profile this browser made.
const ROSTER_BRANCH: &str = "roster";

/// The branch a space keeps its content on.
const CONTENT_BRANCH: &str = "main";

/// How long the session reading the spaces holds its grant.
const SESSION_SECONDS: u64 = 15 * 60;

/// A space kept on this device alone, as a copy is made from: the CARv1 of
/// what its `main` reaches, its revision as JSON, and its name.
#[derive(Debug)]
pub struct LocalSpace {
    pub subject: Did,
    pub name: String,
    pub content: Vec<u8>,
    pub revision: Vec<u8>,
}

/// Why what the earlier release kept could not be read.
#[derive(Debug, thiserror::Error)]
pub enum MigrateError {
    #[error("the earlier release's storage did not open: {0}")]
    Open(String),
    #[error("a profile's spaces could not be listed: {0}")]
    List(String),
    #[error("the space {0} could not be read: {1}")]
    Read(Did, String),
}

/// Every space kept on this device alone by the profiles the earlier release
/// made in `directory`, whose spaces it kept in `base`, as the registry
/// profile `registry` and its roster list them.
pub async fn local_spaces(
    directory: Directory,
    base: Directory,
    registry: &str,
) -> Result<Vec<LocalSpace>, MigrateError> {
    let opened = |error: &dyn std::fmt::Display| MigrateError::Open(error.to_string());
    let (credentials, system) = open_system::<Kept>(directory.clone())
        .await
        .map_err(|error| opened(&error))?;
    let storage = Storage::<Kept>::new().owned_by(system.did());
    let caches = HeldCaches::new();
    let open = |name: String| {
        open_peer_holding(
            Location::new(directory.clone(), name),
            base.clone(),
            storage.clone(),
            &credentials,
            &system,
            false,
            ACCESS_BRANCH,
            &caches,
            &ProfileUpgrade,
        )
    };

    let named = registry;
    let registry = open(named.to_owned())
        .await
        .map_err(|error| opened(&error))?;
    let reading = session(&registry).await?;
    let mut profiles = vec![named.to_owned()];
    for name in roster(&registry, &reading).await? {
        if !profiles.contains(&name) {
            profiles.push(name);
        }
    }

    let mut seen = HashSet::new();
    let mut found = Vec::new();
    for name in profiles {
        let profile = if name == named {
            registry.clone()
        } else {
            match open(name.clone()).await {
                Ok(profile) => profile,
                // A roster entry whose profile is gone has nothing to bring.
                Err(_) => continue,
            }
        };
        let reading = session(&profile).await?;
        let kept = spaces::kept_by(profile.state(), &profile.did(), &profile)
            .await
            .map_err(|error| MigrateError::List(error.to_string()))?;
        for (subject, _) in kept {
            if subject == profile.did() || !seen.insert(subject.clone()) {
                continue;
            }
            if let Some(space) = local_space(&profile, &reading, &subject).await? {
                found.push(space);
            }
        }
    }
    Ok(found)
}

/// A session of `profile` that may read what it keeps.
async fn session(profile: &Peer<Kept>) -> Result<Peer<Kept, dialog_peer::Session>, MigrateError> {
    let expiration = Timestamp::try_from((Timestamp::now().to_unix() + SESSION_SECONDS) as i128)
        .map_err(|error| MigrateError::Open(error.to_string()))?;
    profile
        .session(b"tonk-migrate")
        .space(Repository::from(profile.did()).branch(ACCESS_BRANCH))
        .grant(profile.access().claim(Subject::any()).expires(expiration))
        .await
        .map_err(|error| MigrateError::Open(error.to_string()))
}

/// The storage names of the profiles the registry's roster lists.
async fn roster(
    registry: &Peer<Kept>,
    operator: &Peer<Kept, dialog_peer::Session>,
) -> Result<Vec<String>, MigrateError> {
    let failed = |error: &dyn std::fmt::Display| MigrateError::List(error.to_string());
    let branch = Repository::from(registry.did())
        .branch(ROSTER_BRANCH)
        .open()
        .perform(operator)
        .await
        .map_err(|error| failed(&error))?;
    // A roster an earlier layout wrote reads only once moved to this one.
    dialog_reactor::migrate(&branch, operator)
        .await
        .map_err(|error| failed(&error))?;
    let entries: Vec<DeviceProfile> = branch
        .query()
        .select(Query::<DeviceProfile> {
            this: Term::var("this"),
            name: Term::var("name"),
        })
        .perform(operator)
        .try_vec()
        .await
        .map_err(|error| failed(&error))?;
    Ok(entries.into_iter().map(|entry| entry.name.0).collect())
}

/// `subject` as a copy is made from, if it is kept on this device alone and
/// its `main` has content.
async fn local_space(
    profile: &Peer<Kept>,
    operator: &Peer<Kept, dialog_peer::Session>,
    subject: &Did,
) -> Result<Option<LocalSpace>, MigrateError> {
    let failed =
        |error: &dyn std::fmt::Display| MigrateError::Read(subject.clone(), error.to_string());
    let repository = profile
        .space(subject.as_str())
        .load()
        .perform(operator)
        .await
        .map_err(|error| failed(&error))?;
    let branch = repository
        .branch(CONTENT_BRANCH)
        .open()
        .perform(operator)
        .await
        .map_err(|error| failed(&error))?;
    let Some(revision) = branch.revision() else {
        return Ok(None);
    };
    if matches!(upstream(&branch), Some(Upstream::Remote { .. })) {
        return Ok(None);
    }
    let name = branch
        .query()
        .select(Query::<RepositoryName> {
            this: Term::from(subject.this()),
            name: Term::var("name"),
        })
        .perform(operator)
        .try_vec()
        .await
        .map_err(|error| failed(&error))?
        .into_iter()
        .next()
        .map(|row| row.name.0)
        .unwrap_or_else(|| "Untitled".to_owned());
    let root = Blake3Hash::from(*revision.tree.hash());
    let content = codec::encode(
        repository
            .snapshot(revision.clone())
            .export()
            .perform(operator),
        vec![root],
    )
    .await
    .map_err(|error| failed(&error))?;
    let revision = serde_json::to_vec(&revision).map_err(|error| failed(&error))?;
    Ok(Some(LocalSpace {
        subject: subject.clone(),
        name,
        content,
        revision,
    }))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
