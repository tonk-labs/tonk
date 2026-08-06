//! Which profile this device signs as.
//!
//! Revocation is permanent for a key. The access-service screen matches
//! a revoked device DID against every chain that DID issues a hop in,
//! unscoped by account, and there is no un-revoke anywhere. So signing
//! out — which revokes this device — has to leave a *different* key
//! behind, or this browser could never presign again, local spaces
//! included.
//!
//! Rotating is safe because nothing outside the device is keyed to the
//! device DID: a linked profile's roster entries name the account root,
//! and the spaces it holds are escrowed under that root, so a fresh
//! profile that re-links restores them. What rotation does cost is the
//! passkey prompt to link again, and anything never escrowed — a space
//! that was never sync-enabled has no backup to restore from.
//!
//! The active profile's name is recorded against a fixed registry
//! profile rather than inside the profile it names: a pointer stored in
//! the thing it points at could not be read before opening it.

use dialog_effects::storage::Directory;
use dialog_operator::Profile;
use dialog_storage::provider::storage::Storage;
use serde::{Deserialize, Serialize};
use tonk_common::log;

use crate::TonkWorkerError;
use crate::worker::DefaultSpace;

/// The profile that holds the pointer to the active one. Fixed, because
/// boot has to find it without being told where to look.
///
/// It is also the profile this device signs as until the first
/// rotation — there is no reason to burn a generation on a device that
/// has never signed out.
pub const REGISTRY_PROFILE: &str = "tonk";

/// Credential site on the registry profile holding the active profile's
/// name as UTF-8.
const ACTIVE_PROFILE_SITE: &str = "tonk-active-profile-v1";

/// Credential site on the registry profile holding the roster of every
/// profile this browser knows, as a JSON-serialized `Vec<RosterEntry>`.
///
/// A switcher menu has to describe profiles it has not opened, and
/// opening each one just to render a row would cost key-material load
/// and credential reads per profile per render. The roster is one
/// credential load, maintained by the worker at the moments it already
/// has the facts in hand (boot, link, unlink, rename, switch).
const PROFILE_ROSTER_SITE: &str = "tonk-profile-roster-v1";

/// One profile this browser knows about, as the switcher renders it.
///
/// Inactive entries are as-of their profile's last activation; only the
/// active profile's entry is refreshed from live state. A display name
/// renamed on another device converges the next time that account's
/// profile is activated here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RosterEntry {
    /// Storage name the profile opens under.
    pub profile_name: String,
    /// Account root the profile is attached to. `None` marks a local
    /// workspace — never signed in, or signed out.
    pub root_did: Option<String>,
    /// Attached provider base URL.
    pub provider: Option<String>,
    /// Account email, captured best-effort at link time. May lag.
    pub email: Option<String>,
    /// Display name at last refresh.
    pub display_name: Option<String>,
    /// When the profile was last activated, unix seconds.
    pub last_active_at: u64,
}

/// Where the pointer lives: a profile name and the directory it is
/// opened in. The worker uses [`Registry::device`]; tests point at a
/// scratch directory under their own name so they neither collide with
/// each other nor touch the real profile store.
///
/// Reads and writes go directly against storage — never through the
/// active profile — so they work regardless of which profile is
/// active. That is what lets an activation write the pointer for a
/// profile it has not swapped in yet.
#[derive(Clone)]
pub(crate) struct Registry {
    pub(crate) profile: String,
    pub(crate) directory: Directory,
}

impl Registry {
    /// The one this device actually uses.
    pub(crate) fn device() -> Self {
        Self {
            profile: REGISTRY_PROFILE.to_string(),
            directory: Directory::Profile,
        }
    }

    /// The name of the profile a device starts with — the registry's
    /// own. Valid as an activation target even when no roster entry
    /// names it.
    pub(crate) fn initial_profile(&self) -> &str {
        &self.profile
    }

    async fn open_self(&self, storage: &Storage<DefaultSpace>) -> Result<Profile, TonkWorkerError> {
        Profile::open(&self.profile)
            .at(self.directory.clone())
            .perform(storage)
            .await
            .map_err(|error| {
                TonkWorkerError::Internal(format!("failed to open the registry profile: {error}"))
            })
    }

    /// The recorded active profile name, or `None` when none was ever
    /// written.
    async fn read(
        &self,
        registry: &Profile,
        storage: &Storage<DefaultSpace>,
    ) -> Result<Option<String>, TonkWorkerError> {
        let bytes = match registry
            .credential()
            .site(ACTIVE_PROFILE_SITE)
            .load::<Vec<u8>>()
            .perform(storage)
            .await
        {
            Ok(bytes) => bytes,
            Err(error) if crate::credential::is_missing(&error) => return Ok(None),
            Err(error) => {
                return Err(TonkWorkerError::Internal(format!(
                    "failed to read the active profile pointer: {error}"
                )));
            }
        };

        if bytes.is_empty() {
            return Ok(None);
        }
        String::from_utf8(bytes).map(Some).map_err(|error| {
            TonkWorkerError::Internal(format!("active profile name is not utf-8: {error}"))
        })
    }

    /// Open the profile this device currently signs as.
    pub(crate) async fn open_active(
        &self,
        storage: &Storage<DefaultSpace>,
    ) -> Result<(String, Profile), TonkWorkerError> {
        let registry = self.open_self(storage).await?;

        let name = match self.read(&registry, storage).await {
            Ok(Some(name)) => name,
            Ok(None) => self.profile.clone(),
            Err(error) => {
                log!("active-profile pointer unreadable, signing as the initial profile: {error}");
                self.profile.clone()
            }
        };

        if name == self.profile {
            return Ok((name, registry));
        }

        let profile = self.open_profile(storage, &name).await?;
        Ok((name, profile))
    }

    /// Open a profile by name in the registry's directory.
    ///
    /// `Profile::open` is open-or-create, so callers activating a
    /// user-supplied name must validate it against the roster first —
    /// an unvalidated name would silently mint a garbage key.
    pub(crate) async fn open_profile(
        &self,
        storage: &Storage<DefaultSpace>,
        name: &str,
    ) -> Result<Profile, TonkWorkerError> {
        Profile::open(name)
            .at(self.directory.clone())
            .perform(storage)
            .await
            .map_err(|error| {
                TonkWorkerError::Internal(format!("failed to open profile '{name}': {error}"))
            })
    }

    /// Point the active-profile pointer at `name`.
    ///
    /// Callers repoint only after the target profile opened (and its
    /// state built) successfully, so a failed activation never strands
    /// the next boot on a profile that does not work.
    pub(crate) async fn set_active(
        &self,
        storage: &Storage<DefaultSpace>,
        name: &str,
    ) -> Result<(), TonkWorkerError> {
        let registry = self.open_self(storage).await?;
        registry
            .credential()
            .site(ACTIVE_PROFILE_SITE)
            .save(name.as_bytes().to_vec())
            .perform(storage)
            .await
            .map_err(|error| {
                TonkWorkerError::Internal(format!("failed to record the active profile: {error}"))
            })
    }

    /// The stored roster, or empty when none was ever written.
    pub(crate) async fn read_roster(
        &self,
        storage: &Storage<DefaultSpace>,
    ) -> Result<Vec<RosterEntry>, TonkWorkerError> {
        let registry = self.open_self(storage).await?;
        let bytes = match registry
            .credential()
            .site(PROFILE_ROSTER_SITE)
            .load::<Vec<u8>>()
            .perform(storage)
            .await
        {
            Ok(bytes) => bytes,
            Err(error) if crate::credential::is_missing(&error) => return Ok(Vec::new()),
            Err(error) => {
                return Err(TonkWorkerError::Internal(format!(
                    "failed to read the profile roster: {error}"
                )));
            }
        };
        if bytes.is_empty() {
            return Ok(Vec::new());
        }
        serde_json::from_slice(&bytes).map_err(|error| {
            TonkWorkerError::Internal(format!("stored profile roster is malformed: {error}"))
        })
    }

    /// Insert or replace the roster entry named by `entry.profile_name`.
    pub(crate) async fn upsert_roster(
        &self,
        storage: &Storage<DefaultSpace>,
        entry: RosterEntry,
    ) -> Result<(), TonkWorkerError> {
        let mut roster = self.read_roster(storage).await?;
        match roster
            .iter_mut()
            .find(|existing| existing.profile_name == entry.profile_name)
        {
            Some(existing) => *existing = entry,
            None => roster.push(entry),
        }
        self.write_roster(storage, &roster).await
    }

    async fn write_roster(
        &self,
        storage: &Storage<DefaultSpace>,
        roster: &[RosterEntry],
    ) -> Result<(), TonkWorkerError> {
        let bytes = serde_json::to_vec(roster).map_err(|error| {
            TonkWorkerError::Internal(format!("failed to serialize the profile roster: {error}"))
        })?;
        let registry = self.open_self(storage).await?;
        registry
            .credential()
            .site(PROFILE_ROSTER_SITE)
            .save(bytes)
            .perform(storage)
            .await
            .map_err(|error| {
                TonkWorkerError::Internal(format!("failed to save the profile roster: {error}"))
            })
    }

    /// Generate a fresh profile and make it the active one.
    pub(crate) async fn rotate(
        &self,
        storage: &Storage<DefaultSpace>,
    ) -> Result<(String, Profile), TonkWorkerError> {
        let suffix: [u8; 8] = rand::random();
        let name = format!("{}-{}", self.profile, hex::encode(suffix));

        // `create`, not `open`: a name collision must surface rather
        // than quietly hand back an existing key, since the whole point
        // is to leave the old one behind.
        let profile = Profile::create(&name)
            .at(self.directory.clone())
            .perform(storage)
            .await
            .map_err(|error| {
                TonkWorkerError::Internal(format!("failed to create profile '{name}': {error}"))
            })?;

        // Recorded only once the profile exists, so a failure between
        // the two leaves an orphaned profile rather than a pointer to
        // nothing.
        let registry = self.open_self(storage).await?;
        registry
            .credential()
            .site(ACTIVE_PROFILE_SITE)
            .save(name.clone().into_bytes())
            .perform(storage)
            .await
            .map_err(|error| {
                TonkWorkerError::Internal(format!("failed to record the active profile: {error}"))
            })?;

        Ok((name, profile))
    }
}

/// Open the profile this device currently signs as, with the name it was
/// opened under.
///
/// Falls back to [`REGISTRY_PROFILE`] when no rotation has happened, and
/// also when the pointer is unreadable — a device that cannot read its
/// pointer is better off signing as the profile it started with than
/// refusing to boot. A rotated device in that state re-links rather than
/// losing anything, because the pointer's only job is naming a key.
pub async fn open_active(
    storage: &Storage<DefaultSpace>,
) -> Result<(String, Profile), TonkWorkerError> {
    Registry::device().open_active(storage).await
}

/// Generate a fresh profile and make it the one this device signs as.
///
/// The key left behind is not deleted — it still holds whatever local
/// spaces it opened. It is simply no longer what this device presents,
/// which is the point when the old key has just been revoked.
pub async fn rotate(storage: &Storage<DefaultSpace>) -> Result<(String, Profile), TonkWorkerError> {
    Registry::device().rotate(storage).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test_configure!(run_in_service_worker);

    /// A registry under its own name in a scratch directory, so tests
    /// neither collide with each other nor touch the real profile store.
    ///
    /// Randomly named rather than sequentially: `Directory::Temp` is a
    /// stable path, so a counter is unique only *within* a run and the
    /// next run's differently-ordered tests would inherit the last
    /// run's pointers. Rotation is persistent, so that reads as "a
    /// profile that never rotated has rotated".
    fn scratch() -> Registry {
        Registry {
            profile: format!("device-test-{}", hex::encode(rand::random::<[u8; 8]>())),
            directory: Directory::Temp,
        }
    }

    #[dialog_common::test]
    async fn it_signs_as_the_initial_profile_before_any_rotation() {
        let registry = scratch();
        let storage = Storage::<DefaultSpace>::default();

        let (name, _profile) = registry.open_active(&storage).await.unwrap();

        assert_eq!(name, registry.profile);
    }

    #[dialog_common::test]
    async fn it_leaves_the_old_key_behind_when_it_rotates() {
        let registry = scratch();
        let storage = Storage::<DefaultSpace>::default();
        let (_, before) = registry.open_active(&storage).await.unwrap();

        let (name, after) = registry.rotate(&storage).await.unwrap();

        assert_ne!(name, registry.profile);
        assert_ne!(
            before.did(),
            after.did(),
            "a rotated device must not keep signing with the key it just revoked"
        );
    }

    #[dialog_common::test]
    async fn it_reopens_the_rotated_profile_on_the_next_boot() {
        let registry = scratch();
        let storage = Storage::<DefaultSpace>::default();
        let (rotated_name, rotated) = registry.rotate(&storage).await.unwrap();

        // A fresh pool stands in for a worker restart: nothing carries
        // over but what was persisted.
        let rebooted = Storage::<DefaultSpace>::default();
        let (name, profile) = registry.open_active(&rebooted).await.unwrap();

        assert_eq!(name, rotated_name);
        assert_eq!(
            profile.did(),
            rotated.did(),
            "the pointer has to survive a restart, or a rotated device reverts \
             to the key it revoked"
        );
    }

    fn entry(name: &str, display_name: &str) -> RosterEntry {
        RosterEntry {
            profile_name: name.to_string(),
            root_did: None,
            provider: None,
            email: None,
            display_name: Some(display_name.to_string()),
            last_active_at: 0,
        }
    }

    #[dialog_common::test]
    async fn it_reads_an_empty_roster_before_any_entry_is_written() {
        let registry = scratch();
        let storage = Storage::<DefaultSpace>::default();

        assert_eq!(registry.read_roster(&storage).await.unwrap(), Vec::new());
    }

    #[dialog_common::test]
    async fn it_upserts_a_roster_entry_by_profile_name() {
        let registry = scratch();
        let storage = Storage::<DefaultSpace>::default();

        registry
            .upsert_roster(&storage, entry("one", "first"))
            .await
            .unwrap();
        registry
            .upsert_roster(&storage, entry("two", "second"))
            .await
            .unwrap();
        registry
            .upsert_roster(&storage, entry("one", "renamed"))
            .await
            .unwrap();

        let roster = registry.read_roster(&storage).await.unwrap();
        assert_eq!(roster.len(), 2, "an upsert replaces, never duplicates");
        assert_eq!(
            roster[0].display_name.as_deref(),
            Some("renamed"),
            "the entry keeps its position and takes the new value"
        );
        assert_eq!(roster[1].profile_name, "two");
    }

    #[dialog_common::test]
    async fn it_rotates_again_from_an_already_rotated_profile() {
        let registry = scratch();
        let storage = Storage::<DefaultSpace>::default();

        let (_, first) = registry.rotate(&storage).await.unwrap();
        let (_, second) = registry.rotate(&storage).await.unwrap();
        let (_, active) = registry.open_active(&storage).await.unwrap();

        assert_ne!(first.did(), second.did());
        assert_eq!(
            active.did(),
            second.did(),
            "the pointer must name the newest key, not the first rotation"
        );
    }
}
