//! `carry list` -- enumerate every space this profile knows about.
//!
//! Reads `MetaStore::list_replicas_for_profile` against the profile's
//! own meta branch (the one `Site::open_profile_meta` writes to) and
//! prints one line per replica: `<name>\t<subject DID>`. The
//! profile's self-replica is filtered out — it isn't a space in the
//! "joined or created" sense, just the anchor for the index itself.
//!
//! No `.carry/` is required to run this command. The profile-meta
//! lives on the profile's own storage (dialog's data directory), not
//! in any specific `.carry/` directory, so `carry list` works from
//! anywhere.
//!
//! Output is sorted by name (the order `list_replicas_for_profile`
//! already returns) so successive runs are stable.

use crate::identity_cmd::{self, ProfileLocation};
use anyhow::{Context, Result};
use dialog_repository::Repository;
use tonk_schema::{MetaStore, Replica, prelude::DidExt as _};

/// Name of the meta branch on the profile-as-repository. Mirrors
/// the constant in `site.rs`.
const META_BRANCH: &str = "meta";

/// Query every replica recorded against the active profile,
/// excluding the profile's own self-replica (which serves as the
/// index anchor, not a real space).
///
/// Split out from [`execute`] so integration tests can exercise the
/// query path without parsing stdout.
pub async fn list(profile_location: Option<ProfileLocation>) -> Result<Vec<Replica>> {
    let id = identity_cmd::ensure_identity(profile_location, None).await?;
    let profile_did = id.profile.did();

    let profile_repository = Repository::from(&id.profile);
    let profile_meta = profile_repository
        .branch(META_BRANCH)
        .open()
        .perform(&id.operator)
        .await
        .context("Failed to open profile meta branch")?;

    let store = MetaStore::new(&profile_meta, &id.operator);
    let replicas = store
        .list_replicas_for_profile(&profile_did)
        .await
        .context("Failed to query replicas on profile meta")?;

    let profile_entity = profile_did.this();
    Ok(replicas
        .into_iter()
        .filter(|r| r.subject.0 != profile_entity)
        .collect())
}

/// Execute `carry list`.
///
/// `profile_location` follows the rest of carry: `None` resolves
/// to the platform data directory; tests pass an explicit override.
pub async fn execute(profile_location: Option<ProfileLocation>) -> Result<()> {
    let spaces = list(profile_location).await?;

    if spaces.is_empty() {
        eprintln!(
            "No spaces yet. `carry init` to create one or `carry join <invite-url>` to join one."
        );
        return Ok(());
    }

    for replica in &spaces {
        println!("{}\t{}", replica.name.0, replica.subject.0);
    }
    Ok(())
}
