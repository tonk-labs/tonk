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

use dialog_effects::credential::CredentialError;
use dialog_effects::storage::Directory;
use dialog_operator::Profile;
use dialog_storage::provider::storage::Storage;
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

/// Where the pointer lives: a profile name and the directory it is
/// opened in. The worker uses [`Registry::device`]; tests point at a
/// scratch directory under their own name so they neither collide with
/// each other nor touch the real profile store.
struct Registry {
    profile: String,
    directory: Directory,
}

impl Registry {
    /// The one this device actually uses.
    fn device() -> Self {
        Self {
            profile: REGISTRY_PROFILE.to_string(),
            directory: Directory::Profile,
        }
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
            Err(CredentialError::NotFound(_)) => return Ok(None),
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
    async fn open_active(
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

        let profile = Profile::open(&name)
            .at(self.directory.clone())
            .perform(storage)
            .await
            .map_err(|error| {
                TonkWorkerError::Internal(format!("failed to open profile '{name}': {error}"))
            })?;
        Ok((name, profile))
    }

    /// Generate a fresh profile and make it the active one.
    async fn rotate(
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
