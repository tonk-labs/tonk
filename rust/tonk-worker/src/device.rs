//! Which profile this device signs as.
//!
//! A browser profile owns one device signer and its local workspace.
//! Signing out only disconnects account services; it deliberately leaves
//! that profile, signer, historical account root, and every local space in
//! place. Choosing another account changes the active profile instead of
//! rebinding the existing profile to a different root.
//!
//! The active profile's name is recorded against a fixed registry
//! profile rather than inside the profile it names: a pointer stored in
//! the thing it points at could not be read before opening it.

use dialog_capability::{Subject, did};
use dialog_effects::storage::{self as storage_fx, Directory, Location, LocationExt};
use dialog_query::{Output as _, Query, Term};
use dialog_repository::{Branch, Repository};
use dialog_storage::provider::storage::Storage;
use dialog_varsig::{Did, Principal as _};
use tonk_common::log;
use tonk_schema::DeviceProfile;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tonk_schema::prelude::DidExt as _;

use crate::TonkWorkerError;
use crate::worker::{DefaultOperator, DefaultProfile, DefaultSpace};

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

/// How many times recording the active profile is tried again after its
/// publish lost to another commit on the registry's branch.
const SAVE_RETRY_LIMIT: usize = 4;

/// Whether a save failed because the head moved between its read and its
/// publish. Matched on the rendered error, as the reactor does for its
/// own commits: the `VersionMismatch` leaf renders its text through the
/// chain.
fn is_head_moved(error: &impl std::fmt::Display) -> bool {
    error.to_string().contains("Version mismatch")
}

/// Branch of the registry profile's repository holding the roster of every
/// profile this browser knows. It stores only the stable profile DID and
/// storage handle; the switcher reads mutable labels and account attachment
/// state from the profile that owns them.
///
/// Never upstreamed, so it stays on this device: the registry profile's
/// `main` is an account branch that syncs, and the roster is not the
/// account's business.
const ROSTER_BRANCH: &str = "roster";

/// One profile this browser knows about, as the switcher renders it.
///
/// The registry supplies a deterministic fallback name. The profile router
/// replaces it with the live display name and attachment facts before serving
/// the switcher response.
#[derive(Clone, Debug, PartialEq, Eq)]
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
    /// Display name at last refresh. `None` when nothing names this
    /// profile yet — a derived stand-in here would be indistinguishable
    /// from a name the person chose.
    pub display_name: Option<String>,
}

/// Every site secret the worker kept in a profile's space for `branch`
/// before site secrets were sealed to its peer's vault. Secrets kept once
/// per profile rather than per branch go with the main branch.
fn legacy_sites(branch: &str) -> Vec<String> {
    let mut sites: Vec<String> = [
        crate::router::identity::LOCAL_ROOT_SITE,
        tonk_account::ACCOUNT_PROVIDER_CREDENTIAL_SITE,
        tonk_account::CUSTOMER_CREDENTIAL_SITE,
        tonk_account::TRUSTED_BASE_CREDENTIAL_SITE,
        crate::onboarding::ONBOARDING_ENVELOPE_SITE,
        crate::onboarding::ONBOARDING_KEK_SITE,
    ]
    .into_iter()
    .map(|site| crate::credential::branch_site(site, branch))
    .collect();
    if branch == crate::router::repository::PROFILE_BRANCH {
        sites.extend(
            [
                ACTIVE_PROFILE_SITE,
                tonk_account::PENDING_WORK_CREDENTIAL_SITE,
            ]
            .into_iter()
            .map(String::from),
        );
    }
    sites
}

/// Where the space `name` is stored: the directory every profile's
/// spaces resolve against.
pub(crate) fn space_location(name: &str) -> Location {
    Location::new(Directory::Current, name)
}

/// Open the profile `name` in `directory`, and the storage it is mounted
/// in: a profile that does not exist yet is created.
#[cfg(any(
    test,
    all(feature = "helpers", target_arch = "wasm32", target_os = "unknown")
))]
pub(crate) async fn open_profile_at(
    name: &str,
    directory: Directory,
) -> Result<(Storage<DefaultSpace>, DefaultProfile), TonkWorkerError> {
    let registry = Registry {
        profile: name.to_owned(),
        directory,
    };
    let storage = registry.storage().await?;
    let profile = registry.open_profile(&storage, name).await?;
    Ok((storage, profile))
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

    /// The storage every profile in this registry's directory is
    /// mounted in, owned by the system tonk runs as there.
    pub(crate) async fn storage(&self) -> Result<Storage<DefaultSpace>, TonkWorkerError> {
        let (_, system) = tonk_account::peer::open_system::<DefaultSpace>(self.directory.clone())
            .await
            .map_err(|error| {
                TonkWorkerError::Internal(format!("failed to open the system key: {error}"))
            })?;
        Ok(Storage::<DefaultSpace>::default().owned_by(system.did()))
    }

    /// Open the profile `name` in this registry's directory over
    /// `storage`: load it, or, with `create`, make one when none is there.
    async fn open(
        &self,
        storage: &Storage<DefaultSpace>,
        name: &str,
        create: bool,
    ) -> Result<DefaultProfile, dialog_peer::PeerError> {
        let (credentials, system) =
            tonk_account::peer::open_system::<DefaultSpace>(self.directory.clone()).await?;
        // Space names resolve against the directory the worker has always
        // kept its spaces in.
        let location = Location::new(self.directory.clone(), name);
        let profile = tonk_account::peer::open_peer(
            location.clone(),
            space_location("").directory,
            storage.clone(),
            &credentials,
            &system,
            create,
        )
        .await?;
        tonk_account::peer::migrate_site_secrets(
            &profile,
            &location,
            &legacy_sites(crate::router::repository::PROFILE_BRANCH),
        )
        .await
        .map_err(|error| dialog_peer::PeerError::State(error.to_string()))?;
        Ok(profile)
    }

    /// Open the profile `name` as it acts on its account branch `branch`:
    /// the same key, its records kept in that branch, so the account the
    /// branch belongs to is the one it acts for.
    pub(crate) async fn open_on(
        &self,
        storage: &Storage<DefaultSpace>,
        name: &str,
        branch: &str,
    ) -> Result<DefaultProfile, TonkWorkerError> {
        let (credentials, system) =
            tonk_account::peer::open_system::<DefaultSpace>(self.directory.clone())
                .await
                .map_err(|error| {
                    TonkWorkerError::Internal(format!("failed to open the system key: {error}"))
                })?;
        tonk_account::peer::open_peer_on(
            Location::new(self.directory.clone(), name),
            space_location("").directory,
            storage.clone(),
            &credentials,
            &system,
            false,
            branch,
        )
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!(
                "failed to open the profile on the branch {branch}: {error}"
            ))
        })
    }

    /// Move the site secrets the profile `name` kept in its space for the
    /// branch `branch`, before site secrets were sealed to its peer's
    /// vault, into the peer's.
    pub(crate) async fn migrate_branch_secrets(
        &self,
        profile: &DefaultProfile,
        name: &str,
        branch: &str,
    ) -> Result<(), TonkWorkerError> {
        tonk_account::peer::migrate_site_secrets(
            profile,
            &Location::new(self.directory.clone(), name),
            &legacy_sites(branch),
        )
        .await
        .map(|_| ())
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to move the site secrets: {error}"))
        })
    }

    async fn open_self(
        &self,
        storage: &Storage<DefaultSpace>,
    ) -> Result<DefaultProfile, TonkWorkerError> {
        // PROBE (temporary): surface the raw storage::Load error that
        // opening the profile swallows before falling back to `Create`.
        let probe = Subject::from(did!("local:storage"))
            .attenuate(storage_fx::Storage)
            .attenuate(Location::new(self.directory.clone(), &self.profile))
            .load()
            .perform(storage)
            .await
            .err()
            .map(|error| error.to_string());
        if let Some(error) = &probe {
            log!("registry load probe failed: {error}");
        }
        self.open(storage, &self.profile, true)
            .await
            .map_err(|error| {
                TonkWorkerError::Internal(format!(
                    "failed to open the registry profile: {error}; load probe: {probe:?}"
                ))
            })
    }

    /// The recorded active profile name, or `None` when none was ever
    /// written.
    async fn read(&self, registry: &DefaultProfile) -> Result<Option<String>, TonkWorkerError> {
        let bytes = match registry
            .secrets()
            .site(ACTIVE_PROFILE_SITE)
            .load::<Vec<u8>>()
            .perform(registry)
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
    ) -> Result<(String, DefaultProfile), TonkWorkerError> {
        let registry = self.open_self(storage).await?;

        let name = match self.read(&registry).await {
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
    /// Opening is open-or-create, so callers activating a
    /// user-supplied name must validate it against the roster first —
    /// an unvalidated name would silently mint a garbage key.
    pub(crate) async fn open_profile(
        &self,
        storage: &Storage<DefaultSpace>,
        name: &str,
    ) -> Result<DefaultProfile, TonkWorkerError> {
        self.open(storage, name, true).await.map_err(|error| {
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
        // The registry profile is the one a device signs as until it
        // rotates, so the worker commits to the same branch this save
        // publishes on. A commit landing between the save's read of the
        // head and its publish fails it with a version mismatch; opening
        // the registry again reads the head that won.
        let mut attempt = 0;
        loop {
            let registry = self.open_self(storage).await?;
            match registry
                .secrets()
                .site(ACTIVE_PROFILE_SITE)
                .save(name.as_bytes().to_vec())
                .perform(&registry)
                .await
            {
                Ok(()) => return Ok(()),
                Err(error) if is_head_moved(&error) && attempt < SAVE_RETRY_LIMIT => {
                    attempt += 1;
                    log!("active profile save raced (attempt {attempt}); retrying");
                }
                Err(error) => {
                    return Err(TonkWorkerError::Internal(format!(
                        "failed to record the active profile: {error}"
                    )));
                }
            }
        }
    }

    /// The roster branch, opened fresh: it has no upstream and no
    /// subscribers, so a handle per operation is the simplest thing that
    /// cannot go stale.
    ///
    /// The registry profile is opened through `storage` so its space is
    /// loaded in the pool the operator routes through; the branch itself
    /// is read and written through `operator`, whichever profile it was
    /// derived from. Local reads and commits are not authorized against
    /// the branch's subject, and the roster belongs to the device, not to
    /// whichever profile happens to be active.
    async fn roster_branch(
        &self,
        storage: &Storage<DefaultSpace>,
        operator: &DefaultOperator,
    ) -> Result<Branch, TonkWorkerError> {
        let registry = self.open_self(storage).await?;
        Repository::from(registry.did())
            .branch(ROSTER_BRANCH)
            .open()
            .perform(operator)
            .await
            .map_err(|error| {
                TonkWorkerError::Internal(format!("failed to open the profile roster: {error}"))
            })
    }

    /// The stored roster, ordered by storage name; empty when no entry was
    /// ever written.
    ///
    /// Carries only what this device knows: the profile and the handle to
    /// open it with. A row's label, address and link state live on that
    /// profile's own account branch, and the caller fills them in for the
    /// profiles it opens.
    pub(crate) async fn read_roster(
        &self,
        storage: &Storage<DefaultSpace>,
        operator: &DefaultOperator,
    ) -> Result<Vec<RosterEntry>, TonkWorkerError> {
        let branch = self.roster_branch(storage, operator).await?;
        let profiles: Vec<DeviceProfile> = branch
            .query()
            .select(Query::<DeviceProfile> {
                this: Term::var("this"),
                name: Term::var("name"),
            })
            .perform(operator)
            .try_vec()
            .await
            .map_err(|error| {
                TonkWorkerError::Internal(format!("failed to read the profile roster: {error:?}"))
            })?;

        let mut roster: Vec<RosterEntry> = profiles
            .into_iter()
            .map(|profile| {
                // An unreadable profile is simply unnamed here; an
                // ordinary switcher read replaces this with the name
                // stored in the profile's own repository.
                let display_name = None;
                RosterEntry {
                    profile_name: profile.name.0,
                    root_did: None,
                    provider: None,
                    email: None,
                    display_name,
                }
            })
            .collect();
        roster.sort_by(|a, b| a.profile_name.cmp(&b.profile_name));
        Ok(roster)
    }

    /// Record that `profile` can be opened on this device under
    /// `storage_name`.
    ///
    /// Keyed on the profile's own DID, so re-recording it under a different
    /// handle updates the entry in place rather than leaving a second one
    /// behind. Nothing else is written: a row's label, address and link
    /// state belong to that profile's account branch.
    pub(crate) async fn upsert_roster(
        &self,
        storage: &Storage<DefaultSpace>,
        operator: &DefaultOperator,
        profile: &Did,
        storage_name: &str,
    ) -> Result<(), TonkWorkerError> {
        let branch = self.roster_branch(storage, operator).await?;
        branch
            .transaction()
            .assert(DeviceProfile::new(profile, storage_name))
            .commit()
            .publish()
            .perform(operator)
            .await
            .map(|_| ())
            .map_err(|error| {
                TonkWorkerError::Internal(format!("failed to save the profile roster: {error}"))
            })
    }

    /// Forget `profile`: drop its roster entry so the switcher stops
    /// listing it. Its storage is left alone.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    pub(crate) async fn remove_roster(
        &self,
        storage: &Storage<DefaultSpace>,
        operator: &DefaultOperator,
        profile: &Did,
    ) -> Result<(), TonkWorkerError> {
        let branch = self.roster_branch(storage, operator).await?;
        let entries: Vec<DeviceProfile> = branch
            .query()
            .select(Query::<DeviceProfile> {
                this: Term::from(profile.this()),
                name: Term::var("name"),
            })
            .perform(operator)
            .try_vec()
            .await
            .map_err(|error| {
                TonkWorkerError::Internal(format!("failed to read the profile roster: {error:?}"))
            })?;
        for entry in entries {
            branch
                .transaction()
                .retract(entry)
                .commit()
                .publish()
                .perform(operator)
                .await
                .map_err(|error| {
                    TonkWorkerError::Internal(format!("failed to forget the profile: {error}"))
                })?;
        }
        Ok(())
    }

    /// Generate a fresh profile without changing the active pointer.
    pub(crate) async fn create_profile(
        &self,
        storage: &Storage<DefaultSpace>,
    ) -> Result<(String, DefaultProfile), TonkWorkerError> {
        let suffix: [u8; 8] = rand::random();
        let name = format!("{}-{}", self.profile, hex::encode(suffix));

        // `create`, not `open`: a name collision must surface rather
        // than quietly hand back an existing key, since the whole point
        // is to leave the old one behind.
        if self.open(storage, &name, false).await.is_ok() {
            return Err(TonkWorkerError::Internal(format!(
                "failed to create profile '{name}': it already exists"
            )));
        }
        let profile = self.open(storage, &name, true).await.map_err(|error| {
            TonkWorkerError::Internal(format!("failed to create profile '{name}': {error}"))
        })?;

        Ok((name, profile))
    }
}

/// Open the profile this device currently signs as, with the name it was
/// opened under.
///
/// Falls back to [`REGISTRY_PROFILE`] when no promotion has happened, and
/// also when the pointer is unreadable — a device that cannot read its
/// pointer is better off signing as the profile it started with than
/// refusing to boot. A promoted device in that state re-links rather than
/// losing anything, because the pointer's only job is naming a key.
pub async fn open_active(
    storage: &Storage<DefaultSpace>,
) -> Result<(String, DefaultProfile), TonkWorkerError> {
    Registry::device().open_active(storage).await
}

/// Generate a fresh profile without changing which profile this device signs
/// as. The profile lifecycle module promotes it only after it boots fully.
///
/// The key left behind is not deleted — it still holds whatever local
/// spaces it opened. It is simply no longer the active browser profile.
pub async fn create_profile(
    storage: &Storage<DefaultSpace>,
) -> Result<(String, DefaultProfile), TonkWorkerError> {
    Registry::device().create_profile(storage).await
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
    ///
    /// Natively each registry also gets a directory of its own: the key
    /// of the system tonk runs as is kept per directory, and tests running
    /// in parallel processes that create it at once would each keep a
    /// different one.
    fn scratch() -> Registry {
        let name = format!("device-test-{}", hex::encode(rand::random::<[u8; 8]>()));
        #[cfg(not(target_arch = "wasm32"))]
        let directory = Directory::At(
            std::env::temp_dir()
                .join(&name)
                .to_string_lossy()
                .into_owned(),
        );
        #[cfg(target_arch = "wasm32")]
        let directory = Directory::Temp;
        Registry {
            profile: name,
            directory,
        }
    }

    #[dialog_common::test]
    async fn it_signs_as_the_initial_profile_before_any_rotation() {
        let registry = scratch();
        let storage = registry.storage().await.unwrap();

        let (name, _profile) = registry.open_active(&storage).await.unwrap();

        assert_eq!(name, registry.profile);
    }

    #[dialog_common::test]
    async fn it_creates_a_profile_without_repointing_the_device() {
        let registry = scratch();
        let storage = registry.storage().await.unwrap();
        let (_, before) = registry.open_active(&storage).await.unwrap();

        let (name, after) = registry.create_profile(&storage).await.unwrap();

        assert_ne!(name, registry.profile);
        assert_ne!(
            before.did(),
            after.did(),
            "a created profile must have its own signer"
        );
        let (active_name, active) = registry.open_active(&storage).await.unwrap();
        assert_eq!(active_name, registry.profile);
        assert_eq!(active.did(), before.did());
    }

    #[dialog_common::test]
    async fn it_reopens_a_promoted_profile_on_the_next_boot() {
        let registry = scratch();
        let storage = registry.storage().await.unwrap();
        let (created_name, created) = registry.create_profile(&storage).await.unwrap();
        registry.set_active(&storage, &created_name).await.unwrap();

        // A fresh pool stands in for a worker restart: nothing carries
        // over but what was persisted.
        let rebooted = registry.storage().await.unwrap();
        let (name, profile) = registry.open_active(&rebooted).await.unwrap();

        assert_eq!(name, created_name);
        assert_eq!(
            profile.did(),
            created.did(),
            "the pointer has to survive a restart, or a rotated device reverts \
             to the key it revoked"
        );
    }

    /// The row `read_roster` yields for a profile stored under `name`:
    /// the handle alone. The durable roster carries no display name —
    /// identity lives on the profile's own account branch, and a derived
    /// stand-in here would read as a name the person chose.
    fn entry(_profile: &Did, name: &str) -> RosterEntry {
        RosterEntry {
            profile_name: name.to_string(),
            root_did: None,
            provider: None,
            email: None,
            display_name: None,
        }
    }

    /// A DID to key an entry on, distinct per seed.
    async fn profile_did(seed: u8) -> Did {
        dialog_credentials::Ed25519Signer::import(&[seed; 32])
            .await
            .unwrap()
            .did()
    }

    /// An operator to read and write the roster through: the registry's
    /// own, as a device that never rotated would use.
    async fn operator(registry: &Registry, storage: &Storage<DefaultSpace>) -> DefaultOperator {
        let profile = registry.open_self(storage).await.unwrap();
        crate::session::open(&profile).await.unwrap().operator
    }

    #[dialog_common::test]
    async fn it_reads_an_empty_roster_before_any_entry_is_written() {
        let registry = scratch();
        let storage = registry.storage().await.unwrap();
        let operator = operator(&registry, &storage).await;

        assert_eq!(
            registry.read_roster(&storage, &operator).await.unwrap(),
            Vec::new()
        );
    }

    #[dialog_common::test]
    async fn it_lists_every_profile_it_recorded() {
        let registry = scratch();
        let storage = registry.storage().await.unwrap();
        let operator = operator(&registry, &storage).await;

        registry
            .upsert_roster(&storage, &operator, &profile_did(1).await, "one")
            .await
            .unwrap();
        registry
            .upsert_roster(&storage, &operator, &profile_did(2).await, "two")
            .await
            .unwrap();

        assert_eq!(
            registry.read_roster(&storage, &operator).await.unwrap(),
            vec![
                entry(&profile_did(1).await, "one"),
                entry(&profile_did(2).await, "two")
            ],
            "ordered by storage name"
        );
    }

    /// The entity is the profile, so recording the same profile under a
    /// new handle moves the entry rather than adding one.
    #[dialog_common::test]
    async fn it_keeps_one_entry_per_profile_when_the_handle_changes() {
        let registry = scratch();
        let storage = registry.storage().await.unwrap();
        let operator = operator(&registry, &storage).await;
        let profile = profile_did(1).await;

        registry
            .upsert_roster(&storage, &operator, &profile, "one")
            .await
            .unwrap();
        registry
            .upsert_roster(&storage, &operator, &profile, "renamed")
            .await
            .unwrap();

        assert_eq!(
            registry.read_roster(&storage, &operator).await.unwrap(),
            vec![entry(&profile, "renamed")],
            "one profile is one entry, whatever it is stored under"
        );
    }

    /// Two profiles sharing a storage name would be a bug elsewhere, but
    /// the roster keys on the profile, so they stay two rows.
    #[dialog_common::test]
    async fn it_keeps_distinct_profiles_apart() {
        let registry = scratch();
        let storage = registry.storage().await.unwrap();
        let operator = operator(&registry, &storage).await;

        registry
            .upsert_roster(&storage, &operator, &profile_did(1).await, "one")
            .await
            .unwrap();
        registry
            .upsert_roster(&storage, &operator, &profile_did(2).await, "one")
            .await
            .unwrap();

        assert_eq!(
            registry
                .read_roster(&storage, &operator)
                .await
                .unwrap()
                .len(),
            2,
            "two profiles are two entries"
        );
    }

    #[dialog_common::test]
    async fn it_serves_the_roster_to_another_profiles_operator() {
        let registry = scratch();
        let storage = registry.storage().await.unwrap();
        let operator = operator(&registry, &storage).await;
        registry
            .upsert_roster(&storage, &operator, &profile_did(1).await, "one")
            .await
            .unwrap();

        // A rotated device reads the roster through the profile it now
        // signs as, not the registry's key.
        let (_, created) = registry.create_profile(&storage).await.unwrap();
        let other = crate::session::open(&created).await.unwrap().operator;
        assert_eq!(
            registry.read_roster(&storage, &other).await.unwrap(),
            vec![entry(&profile_did(1).await, "one")]
        );
    }

    #[dialog_common::test]
    async fn it_promotes_profiles_in_order() {
        let registry = scratch();
        let storage = registry.storage().await.unwrap();

        let (first_name, first) = registry.create_profile(&storage).await.unwrap();
        registry.set_active(&storage, &first_name).await.unwrap();
        let (second_name, second) = registry.create_profile(&storage).await.unwrap();
        registry.set_active(&storage, &second_name).await.unwrap();
        let (_, active) = registry.open_active(&storage).await.unwrap();

        assert_ne!(first.did(), second.did());
        assert_eq!(
            active.did(),
            second.did(),
            "the pointer must name the newest key, not the first rotation"
        );
    }

    /// Recording the active profile survives the registry's branch
    /// moving under it.
    ///
    /// The registry profile is also the profile a device signs as until
    /// it rotates, so its branch takes the worker's own commits: account
    /// catch-up runs detached and writes there while a profile switch
    /// records the pointer. The save reads the head, then publishes
    /// against it, and a commit landing in between fails the publish.
    /// That failed the whole switch, after the profile had already left
    /// its account.
    #[dialog_common::test]
    async fn it_records_the_active_profile_while_the_registry_branch_moves() {
        let registry = scratch();
        let storage = registry.storage().await.unwrap();
        registry.open_self(&storage).await.unwrap();

        let names = ["one", "two", "three"];
        let recorded =
            futures_util::future::join_all(names.map(|name| registry.set_active(&storage, name)))
                .await;

        for (name, outcome) in names.iter().zip(recorded) {
            outcome.unwrap_or_else(|error| panic!("recording '{name}' failed: {error}"));
        }
        let profile = registry.open_self(&storage).await.unwrap();
        let active = registry.read(&profile).await.unwrap();
        assert!(
            active
                .as_deref()
                .is_some_and(|active| names.contains(&active)),
            "the pointer names one of the recorded profiles, got {active:?}"
        );
    }
}
