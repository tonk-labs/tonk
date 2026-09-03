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
use dialog_operator::Profile;
use dialog_query::{Output as _, Query, Term};
use dialog_repository::{Branch, Repository};
use dialog_storage::provider::storage::Storage;
use dialog_varsig::Did;
use tonk_common::log;
use tonk_schema::DeviceProfile;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tonk_schema::prelude::DidExt as _;

use crate::TonkWorkerError;
use crate::worker::{DefaultOperator, DefaultSpace};

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

/// Branch of the registry profile's repository holding the roster of
/// every profile this browser knows, one [`RosterProfile`] entity per
/// storage name with its attachment and email as stamps.
///
/// A switcher menu has to describe profiles it has not opened, and
/// opening each one just to render a row would cost key-material load
/// and credential reads per profile per render. The roster is maintained
/// by the worker at the moments it already has the facts in hand (boot,
/// link, unlink, rename, switch). Facts rather than one serialized blob:
/// concurrent refreshes merge per entity instead of racing a
/// read-modify-write of the whole roster.
///
/// Never upstreamed, so it stays on this device: the registry profile's
/// `main` is an account branch that syncs, and the roster is not the
/// account's business.
const ROSTER_BRANCH: &str = "roster";

/// One profile this browser knows about, as the switcher renders it.
///
/// Inactive entries are as-of their profile's last activation; only the
/// active profile's entry is refreshed from live state. A display name
/// renamed on another device converges the next time that account's
/// profile is activated here.
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
    /// Display name at last refresh.
    pub display_name: String,
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
        // PROBE (temporary): surface the raw storage::Load error that
        // `Profile::open` swallows before falling back to `Create`.
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
        Profile::open(&self.profile)
            .at(self.directory.clone())
            .perform(storage)
            .await
            .map_err(|error| {
                TonkWorkerError::Internal(format!(
                    "failed to open the registry profile: {error}; load probe: {probe:?}"
                ))
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
        Repository::from(&registry)
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
                // The row is keyed on the profile's DID, and the default
                // display name is the deterministic petname of that DID —
                // so an inactive profile keeps its name in the switcher
                // without being opened. A user-chosen rename or the
                // account email still only shows while the profile is
                // active, when the live splice reads them from where
                // they live.
                let display_name = profile
                    .this()
                    .as_str()
                    .parse()
                    .map(|did| tonk_schema::petname(&did))
                    .unwrap_or_default();
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
) -> Result<(String, Profile), TonkWorkerError> {
    Registry::device().open_active(storage).await
}

/// Generate a fresh profile without changing which profile this device signs
/// as. The profile lifecycle module promotes it only after it boots fully.
///
/// The key left behind is not deleted — it still holds whatever local
/// spaces it opened. It is simply no longer the active browser profile.
pub async fn create_profile(
    storage: &Storage<DefaultSpace>,
) -> Result<(String, Profile), TonkWorkerError> {
    Registry::device().create_profile(storage).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_varsig::Principal as _;

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
    async fn it_creates_a_profile_without_repointing_the_device() {
        let registry = scratch();
        let storage = Storage::<DefaultSpace>::default();
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
        let storage = Storage::<DefaultSpace>::default();
        let (created_name, created) = registry.create_profile(&storage).await.unwrap();
        registry.set_active(&storage, &created_name).await.unwrap();

        // A fresh pool stands in for a worker restart: nothing carries
        // over but what was persisted.
        let rebooted = Storage::<DefaultSpace>::default();
        let (name, profile) = registry.open_active(&rebooted).await.unwrap();

        assert_eq!(name, created_name);
        assert_eq!(
            profile.did(),
            created.did(),
            "the pointer has to survive a restart, or a rotated device reverts \
             to the key it revoked"
        );
    }

    /// The row `read_roster` yields for `profile` stored under `name`:
    /// the handle, plus the petname the display name defaults to —
    /// identity beyond that lives on the profile's own account branch.
    fn entry(profile: &Did, name: &str) -> RosterEntry {
        RosterEntry {
            profile_name: name.to_string(),
            root_did: None,
            provider: None,
            email: None,
            display_name: tonk_schema::petname(profile),
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
        crate::session::open(&profile, storage)
            .await
            .unwrap()
            .operator
    }

    #[dialog_common::test]
    async fn it_reads_an_empty_roster_before_any_entry_is_written() {
        let registry = scratch();
        let storage = Storage::<DefaultSpace>::default();
        let operator = operator(&registry, &storage).await;

        assert_eq!(
            registry.read_roster(&storage, &operator).await.unwrap(),
            Vec::new()
        );
    }

    #[dialog_common::test]
    async fn it_lists_every_profile_it_recorded() {
        let registry = scratch();
        let storage = Storage::<DefaultSpace>::default();
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
        let storage = Storage::<DefaultSpace>::default();
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
        let storage = Storage::<DefaultSpace>::default();
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
        let storage = Storage::<DefaultSpace>::default();
        let operator = operator(&registry, &storage).await;
        registry
            .upsert_roster(&storage, &operator, &profile_did(1).await, "one")
            .await
            .unwrap();

        // A rotated device reads the roster through the profile it now
        // signs as, not the registry's key.
        let (_, created) = registry.create_profile(&storage).await.unwrap();
        let other = crate::session::open(&created, &storage)
            .await
            .unwrap()
            .operator;
        assert_eq!(
            registry.read_roster(&storage, &other).await.unwrap(),
            vec![entry(&profile_did(1).await, "one")]
        );
    }

    #[dialog_common::test]
    async fn it_promotes_profiles_in_order() {
        let registry = scratch();
        let storage = Storage::<DefaultSpace>::default();

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
}
