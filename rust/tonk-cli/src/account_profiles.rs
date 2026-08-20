//! Install-level routing for native account profiles.
//!
//! A profile owns one Dialog identity and one [`SpotStore`]. The install-level
//! registry contains only the selected default profile and directory bindings;
//! it never contains credentials or delegation bytes.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use dialog_effects::storage::Directory;
use dialog_operator::Profile;
use dialog_storage::provider::storage::{NativeSpace, Storage};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::spot::{self, Source, SpotStore};

/// Stable ID assigned to the grandfathered single-profile installation.
pub const LEGACY_PROFILE_ID: &str = "legacy";
const REGISTRY_VERSION: u8 = 1;
const REGISTRY_FILE: &str = "profiles.json";

/// Opaque install-local identifier for one native profile.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NativeProfileId(String);

impl NativeProfileId {
    /// Return the grandfathered profile ID.
    pub fn legacy() -> Self {
        Self(LEGACY_PROFILE_ID.to_owned())
    }

    /// Generate a cryptographically random native profile ID.
    pub fn generate() -> Self {
        Self::from_bytes(rand::random())
    }

    fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(format!("p-{}", hex::encode(bytes)))
    }

    /// Borrow the serialized identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn generated_hex(&self) -> Option<&str> {
        self.0.strip_prefix("p-").filter(|suffix| {
            suffix.len() == 32
                && suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
    }

    fn is_valid(&self) -> bool {
        self.0 == LEGACY_PROFILE_ID || self.generated_hex().is_some()
    }
}

/// Recover the deterministic Dialog profile name from a generated profile's
/// isolated state root. Legacy and arbitrary test stores return `None`.
pub(crate) fn generated_dialog_profile_name(store: &SpotStore) -> Option<String> {
    let profile_id = store.root().file_name()?.to_str()?;
    let suffix = profile_id.strip_prefix("p-")?;
    (suffix.len() == 32
        && suffix
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
    .then(|| format!("tonk-{suffix}"))
}

impl std::fmt::Display for NativeProfileId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Non-secret routing and deployment metadata for one native profile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeProfileRecord {
    /// Unique case-insensitive local label.
    pub label: String,
    /// Isolated Dialog key-profile name.
    pub dialog_profile_name: String,
    /// Immutable account-root index after the first successful ceremony.
    pub account_root: Option<String>,
    /// Origin that hosted the most recent successful ceremony.
    pub ceremony_origin: Option<String>,
    /// Provider-matched default content endpoint.
    pub default_access_remote: Option<String>,
    /// Provider-matched invitation-revocation relay.
    pub default_revocation_relay: Option<String>,
    /// Unknown forward-compatible profile fields.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Exact profile and space selected by a directory binding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundSpace {
    /// Owning native profile.
    pub profile: NativeProfileId,
    /// Profile-local space name.
    pub space: String,
}

/// Version-one install-level profile roster and directory bindings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeProfileRegistryV1 {
    /// Schema version, exactly one.
    pub version: u8,
    /// Default profile for account commands and explicit space names.
    pub selected: Option<NativeProfileId>,
    #[serde(default)]
    /// Profiles keyed by opaque install-local ID.
    pub profiles: BTreeMap<NativeProfileId, NativeProfileRecord>,
    #[serde(default)]
    /// Canonical directory paths mapped to exact profile-local spaces.
    pub bindings: BTreeMap<PathBuf, BoundSpace>,
    #[serde(flatten)]
    /// Unknown forward-compatible registry fields.
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for NativeProfileRegistryV1 {
    fn default() -> Self {
        Self {
            version: REGISTRY_VERSION,
            selected: None,
            profiles: BTreeMap::new(),
            bindings: BTreeMap::new(),
            extra: serde_json::Map::new(),
        }
    }
}

/// Fully resolved operational context for one native profile.
#[derive(Clone, Debug)]
pub struct NativeProfileContext {
    /// Install-local profile identifier.
    pub id: NativeProfileId,
    /// Persisted non-secret metadata.
    pub record: NativeProfileRecord,
    /// Profile-local space and account-state store.
    pub store: SpotStore,
}

/// Local provider session state shown by `tonk account list`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileSignIn {
    /// No active provider attachment.
    SignedOut,
    /// A browser ceremony can be resumed.
    Pending,
    /// A provider attachment is active.
    Active,
}

impl NativeProfileContext {
    /// Open or create this context's isolated Dialog key profile.
    pub async fn open_profile(&self) -> anyhow::Result<Profile> {
        Profile::open(self.record.dialog_profile_name.clone())
            .at(Directory::Profile)
            .perform(&Storage::<NativeSpace>::default())
            .await
            .with_context(|| {
                format!(
                    "failed to open Dialog profile '{}' for native profile {}",
                    self.record.dialog_profile_name, self.id
                )
            })
    }

    /// Load an existing Dialog profile without creating one.
    pub async fn load_profile(&self) -> anyhow::Result<Profile> {
        Profile::load(self.record.dialog_profile_name.clone())
            .at(Directory::Profile)
            .perform(&Storage::<NativeSpace>::default())
            .await
            .with_context(|| {
                format!(
                    "failed to load Dialog profile '{}' for native profile {}",
                    self.record.dialog_profile_name, self.id
                )
            })
    }

    /// Build site configuration pinned to this context.
    pub fn site_config(&self) -> crate::site::SiteConfig {
        crate::site::SiteConfig {
            profile_name: self.record.dialog_profile_name.clone(),
            profile_directory: Directory::Profile,
            require_account: std::env::var_os("TONK_UNSAFE_ALLOW_DEVICE_ROOT").is_none(),
            account_store: self.store.clone(),
        }
    }

    /// Inspect the local session sidecar without creating profile state.
    pub fn sign_in_state(&self) -> Result<ProfileSignIn, ProfileError> {
        crate::account_session::inspect_local(&self.store)
            .map(|phase| match phase {
                crate::account_session::LocalPhase::SignedOut => ProfileSignIn::SignedOut,
                crate::account_session::LocalPhase::Pending => ProfileSignIn::Pending,
                crate::account_session::LocalPhase::Active => ProfileSignIn::Active,
            })
            .map_err(|error| ProfileError::Io {
                path: self.store.account_dir(),
                detail: error.to_string(),
            })
    }
}

/// Space selection resolved to an immutable profile context.
#[derive(Clone, Debug)]
pub struct ResolvedSpace {
    /// Owning native profile.
    pub profile: NativeProfileContext,
    /// Profile-local registered name.
    pub name: String,
    /// Canonical site directory.
    pub site: PathBuf,
    /// Precedence source that selected this space.
    pub source: Source,
}

/// Install profile registry and routing failures.
#[derive(Debug, Error)]
pub enum ProfileError {
    /// Registry JSON or shape is invalid.
    #[error("corrupt native profile registry at {path}: {detail}")]
    Corrupt {
        /// Registry path.
        path: PathBuf,
        /// Parse or validation detail.
        detail: String,
    },
    /// Registry version is unsupported.
    #[error("unsupported version {version} in {path}")]
    UnsupportedVersion {
        /// Registry path.
        path: PathBuf,
        /// Unsupported serialized version.
        version: u64,
    },
    /// A selected or bound profile is absent from the roster.
    #[error("native profile registry at {path} references unknown profile '{profile}'")]
    UnknownProfile {
        /// Registry path.
        path: PathBuf,
        /// Referenced missing profile.
        profile: NativeProfileId,
    },
    /// A binding points at a missing profile-local space.
    #[error(
        "unknown space '{space}' in profile '{profile}'; native profile registry at {path} has a binding at {directory}; clear it with `tonk space unbind {directory}`"
    )]
    DanglingBinding {
        /// Registry path.
        path: PathBuf,
        /// Canonical bound directory.
        directory: PathBuf,
        /// Owning profile from the binding.
        profile: NativeProfileId,
        /// Missing profile-local name.
        space: String,
    },
    /// No default profile exists yet.
    #[error("no native account profile is selected; run `tonk account add`")]
    NoSelectedProfile,
    /// A label violates the documented slug contract.
    #[error("invalid account profile label '{0}': use [a-z0-9][a-z0-9-_]*")]
    InvalidLabel(String),
    /// A label collides case-insensitively.
    #[error("account profile label '{0}' is already in use")]
    DuplicateLabel(String),
    /// A serialized or generated profile ID is malformed.
    #[error("invalid native profile id '{0}'")]
    InvalidProfileId(String),
    /// Filesystem persistence failed.
    #[error("could not create or update {path}: {detail}")]
    Io {
        /// Affected path.
        path: PathBuf,
        /// Underlying error detail.
        detail: String,
    },
    /// Profile-local spot registry failure.
    #[error(transparent)]
    Spot(#[from] spot::SpotError),
}

/// Install-level profile registry rooted beside legacy `spots.json`.
#[derive(Clone, Debug)]
pub struct NativeProfileStore {
    install: SpotStore,
}

impl NativeProfileStore {
    /// Locate the native install root from the normal environment contract.
    pub fn open() -> Result<Self, ProfileError> {
        Ok(Self {
            install: SpotStore::open()?,
        })
    }

    /// Construct a store at an explicit install root.
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self {
            install: SpotStore::at(root),
        }
    }

    /// Borrow the install root.
    pub fn root(&self) -> &Path {
        self.install.root()
    }

    /// Return the install registry path.
    pub fn registry_path(&self) -> PathBuf {
        self.root().join(REGISTRY_FILE)
    }

    /// Load and validate the registry, bootstrapping legacy metadata only.
    pub fn load_or_bootstrap(&self) -> Result<NativeProfileRegistryV1, ProfileError> {
        let path = self.registry_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => self.parse_and_validate(&path, &text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => self.bootstrap_legacy(),
            Err(error) => Err(ProfileError::Io {
                path,
                detail: error.to_string(),
            }),
        }
    }

    fn parse_and_validate(
        &self,
        path: &Path,
        text: &str,
    ) -> Result<NativeProfileRegistryV1, ProfileError> {
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|error| ProfileError::Corrupt {
                path: path.to_path_buf(),
                detail: error.to_string(),
            })?;
        let version = value
            .get("version")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| ProfileError::Corrupt {
                path: path.to_path_buf(),
                detail: "missing integer field 'version'".to_owned(),
            })?;
        if version != u64::from(REGISTRY_VERSION) {
            return Err(ProfileError::UnsupportedVersion {
                path: path.to_path_buf(),
                version,
            });
        }
        let mut registry: NativeProfileRegistryV1 =
            serde_json::from_value(value).map_err(|error| ProfileError::Corrupt {
                path: path.to_path_buf(),
                detail: error.to_string(),
            })?;
        registry.bindings = registry
            .bindings
            .into_iter()
            .map(|(directory, binding)| (canonical(&directory), binding))
            .collect();
        self.validate(&registry)?;
        Ok(registry)
    }

    fn bootstrap_legacy(&self) -> Result<NativeProfileRegistryV1, ProfileError> {
        let has_legacy_state = self.install.registry_path().exists()
            || self.install.spots_root().exists()
            || self.install.account_dir().exists();
        if !has_legacy_state {
            return Ok(NativeProfileRegistryV1::default());
        }

        let spots = self.install.load()?;
        let legacy = NativeProfileId::legacy();
        let mut registry = NativeProfileRegistryV1 {
            selected: Some(legacy.clone()),
            ..NativeProfileRegistryV1::default()
        };
        registry.profiles.insert(
            legacy.clone(),
            NativeProfileRecord {
                label: "default".to_owned(),
                dialog_profile_name: crate::site::PROFILE_NAME.to_owned(),
                account_root: None,
                ceremony_origin: None,
                default_access_remote: None,
                default_revocation_relay: None,
                extra: serde_json::Map::new(),
            },
        );
        registry.bindings = spots
            .bindings
            .into_iter()
            .map(|(directory, space)| {
                (
                    canonical(&directory),
                    BoundSpace {
                        profile: legacy.clone(),
                        space,
                    },
                )
            })
            .collect();
        self.validate(&registry)?;
        self.save(&registry)?;
        Ok(registry)
    }

    fn validate(&self, registry: &NativeProfileRegistryV1) -> Result<(), ProfileError> {
        let path = self.registry_path();
        for id in registry.profiles.keys() {
            if !id.is_valid() {
                return Err(ProfileError::InvalidProfileId(id.to_string()));
            }
        }
        if let Some(selected) = &registry.selected
            && !registry.profiles.contains_key(selected)
        {
            return Err(ProfileError::UnknownProfile {
                path,
                profile: selected.clone(),
            });
        }
        let mut labels = BTreeSet::new();
        for record in registry.profiles.values() {
            validate_label(&record.label)?;
            if !labels.insert(record.label.to_ascii_lowercase()) {
                return Err(ProfileError::DuplicateLabel(record.label.clone()));
            }
        }
        for (directory, binding) in &registry.bindings {
            let Some(_) = registry.profiles.get(&binding.profile) else {
                return Err(ProfileError::UnknownProfile {
                    path: path.clone(),
                    profile: binding.profile.clone(),
                });
            };
            let context = self.context_from(registry, &binding.profile)?;
            if !context.store.load()?.spots.contains_key(&binding.space) {
                return Err(ProfileError::DanglingBinding {
                    path: path.clone(),
                    directory: directory.clone(),
                    profile: binding.profile.clone(),
                    space: binding.space.clone(),
                });
            }
        }
        Ok(())
    }

    /// Atomically persist a validated version-one registry.
    pub fn save(&self, registry: &NativeProfileRegistryV1) -> Result<(), ProfileError> {
        self.validate(registry)?;
        std::fs::create_dir_all(self.root()).map_err(|error| ProfileError::Io {
            path: self.root().to_path_buf(),
            detail: error.to_string(),
        })?;
        let path = self.registry_path();
        let tmp = self.root().join(format!("{REGISTRY_FILE}.tmp"));
        let bytes = serde_json::to_vec_pretty(registry).map_err(|error| ProfileError::Io {
            path: path.clone(),
            detail: error.to_string(),
        })?;
        std::fs::write(&tmp, bytes).map_err(|error| ProfileError::Io {
            path: tmp.clone(),
            detail: error.to_string(),
        })?;
        std::fs::rename(&tmp, &path).map_err(|error| ProfileError::Io {
            path,
            detail: error.to_string(),
        })
    }

    /// Resolve the currently selected profile without opening Dialog state.
    pub fn selected(&self) -> Result<Option<NativeProfileContext>, ProfileError> {
        let registry = self.load_or_bootstrap()?;
        registry
            .selected
            .as_ref()
            .map(|id| self.context_from(&registry, id))
            .transpose()
    }

    /// Return the selected context, creating the grandfathered local profile
    /// only when an explicit write needs a profile on an otherwise empty
    /// install. Read-only commands never call this boundary.
    pub fn ensure_selected_for_local_write(&self) -> Result<NativeProfileContext, ProfileError> {
        if let Some(selected) = self.selected()? {
            return Ok(selected);
        }
        let mut registry = self.load_or_bootstrap()?;
        let legacy = NativeProfileId::legacy();
        registry.selected = Some(legacy.clone());
        registry.profiles.insert(
            legacy.clone(),
            NativeProfileRecord {
                label: "default".to_owned(),
                dialog_profile_name: crate::site::PROFILE_NAME.to_owned(),
                account_root: None,
                ceremony_origin: None,
                default_access_remote: None,
                default_revocation_relay: None,
                extra: serde_json::Map::new(),
            },
        );
        self.save(&registry)?;
        self.context_from(&registry, &legacy)
    }

    /// Select a profile by case-insensitive label or exact ID.
    pub fn select(&self, id_or_label: &str) -> Result<NativeProfileContext, ProfileError> {
        let mut registry = self.load_or_bootstrap()?;
        let id = lookup_id(&registry, id_or_label).ok_or_else(|| ProfileError::UnknownProfile {
            path: self.registry_path(),
            profile: NativeProfileId(id_or_label.to_owned()),
        })?;
        registry.selected = Some(id.clone());
        self.save(&registry)?;
        self.context_from(&registry, &id)
    }

    /// Resolve an exact profile ID without selecting or opening it.
    pub fn context(&self, id: &NativeProfileId) -> Result<NativeProfileContext, ProfileError> {
        let registry = self.load_or_bootstrap()?;
        self.context_from(&registry, id)
    }

    /// Persist the immutable account-root index after a successful handoff.
    pub fn record_account_root(
        &self,
        id: &NativeProfileId,
        root: &str,
        ceremony_origin: Option<&str>,
    ) -> Result<NativeProfileContext, ProfileError> {
        let mut registry = self.load_or_bootstrap()?;
        let record = registry
            .profiles
            .get_mut(id)
            .ok_or_else(|| ProfileError::UnknownProfile {
                path: self.registry_path(),
                profile: id.clone(),
            })?;
        if let Some(existing) = record.account_root.as_deref()
            && existing != root
        {
            return Err(ProfileError::Io {
                path: self.registry_path(),
                detail: format!(
                    "this profile belongs to {existing}; run `tonk account add` to use another account"
                ),
            });
        }
        record.account_root = Some(root.to_owned());
        if let Some(origin) = ceremony_origin {
            record.ceremony_origin = Some(origin.to_owned());
        }
        self.save(&registry)?;
        self.context_from(&registry, id)
    }

    /// Atomically persist provider-matched content endpoints for one profile.
    pub fn record_deployment_defaults(
        &self,
        id: &NativeProfileId,
        defaults: &crate::deployment::DeploymentDefaults,
    ) -> Result<NativeProfileContext, ProfileError> {
        let mut registry = self.load_or_bootstrap()?;
        let record = registry
            .profiles
            .get_mut(id)
            .ok_or_else(|| ProfileError::UnknownProfile {
                path: self.registry_path(),
                profile: id.clone(),
            })?;
        record.ceremony_origin = Some(defaults.ceremony_origin.to_string());
        record.default_access_remote = Some(defaults.access_remote.to_string());
        record.default_revocation_relay = Some(defaults.revocation_relay.to_string());
        self.save(&registry)?;
        self.context_from(&registry, id)
    }

    fn context_from(
        &self,
        registry: &NativeProfileRegistryV1,
        id: &NativeProfileId,
    ) -> Result<NativeProfileContext, ProfileError> {
        let record =
            registry
                .profiles
                .get(id)
                .cloned()
                .ok_or_else(|| ProfileError::UnknownProfile {
                    path: self.registry_path(),
                    profile: id.clone(),
                })?;
        let store = if id.as_str() == LEGACY_PROFILE_ID {
            self.install.clone()
        } else {
            SpotStore::at(self.root().join("profiles").join(id.as_str()))
        };
        Ok(NativeProfileContext {
            id: id.clone(),
            record,
            store,
        })
    }

    /// Create a fresh unrooted profile with a random ID.
    pub fn create_pending(
        &self,
        label: Option<&str>,
    ) -> Result<NativeProfileContext, ProfileError> {
        self.create_pending_with_bytes(label, rand::random())
    }

    /// Resume the selected unrooted profile when it matches the requested
    /// label, otherwise create a fresh provisional profile.
    pub fn create_or_resume_pending(
        &self,
        label: Option<&str>,
    ) -> Result<NativeProfileContext, ProfileError> {
        if let Some(selected) = self.selected()?
            && selected.record.account_root.is_none()
            && label.is_none_or(|label| selected.record.label.eq_ignore_ascii_case(label))
        {
            return Ok(selected);
        }
        self.create_pending(label)
    }

    #[doc(hidden)]
    pub fn create_pending_with_bytes(
        &self,
        label: Option<&str>,
        bytes: [u8; 16],
    ) -> Result<NativeProfileContext, ProfileError> {
        let mut registry = self.load_or_bootstrap()?;
        let id = NativeProfileId::from_bytes(bytes);
        if registry.profiles.contains_key(&id) {
            return Err(ProfileError::InvalidProfileId(format!(
                "generated profile id collision: {id}"
            )));
        }
        let label = match label {
            Some(label) => {
                validate_label(label)?;
                label.to_owned()
            }
            None => next_default_label(&registry),
        };
        if registry
            .profiles
            .values()
            .any(|record| record.label.eq_ignore_ascii_case(&label))
        {
            return Err(ProfileError::DuplicateLabel(label));
        }
        let suffix = id
            .generated_hex()
            .expect("generated native profile id has a hex suffix");
        registry.profiles.insert(
            id.clone(),
            NativeProfileRecord {
                label,
                dialog_profile_name: format!("tonk-{suffix}"),
                account_root: None,
                ceremony_origin: None,
                default_access_remote: None,
                default_revocation_relay: None,
                extra: serde_json::Map::new(),
            },
        );
        if registry.selected.is_none() {
            registry.selected = Some(id.clone());
        }
        self.save(&registry)?;
        self.context_from(&registry, &id)
    }

    /// Resolve a space using flag, environment, then nearest-binding precedence.
    pub fn resolve(
        &self,
        flag: Option<&str>,
        env: Option<&str>,
        cwd: Option<&Path>,
    ) -> Result<ResolvedSpace, ProfileError> {
        let registry = self.load_or_bootstrap()?;
        if flag.is_some() || env.is_some() {
            let id = registry
                .selected
                .as_ref()
                .ok_or(ProfileError::NoSelectedProfile)?;
            let profile = self.context_from(&registry, id)?;
            let resolved = profile.store.resolve(flag, env, None)?;
            return Ok(ResolvedSpace {
                profile,
                name: resolved.name,
                site: resolved.site,
                source: resolved.source,
            });
        }
        if let Some((directory, binding)) = cwd.and_then(|cwd| directory_binding(&registry, cwd)) {
            let profile = self.context_from(&registry, &binding.profile)?;
            let local = profile.store.load()?;
            let entry =
                local
                    .spots
                    .get(&binding.space)
                    .ok_or_else(|| ProfileError::DanglingBinding {
                        path: self.registry_path(),
                        directory: directory.clone(),
                        profile: binding.profile.clone(),
                        space: binding.space.clone(),
                    })?;
            return Ok(ResolvedSpace {
                profile,
                name: binding.space,
                site: entry.site.clone(),
                source: Source::Directory(directory),
            });
        }
        let id = match registry.selected.as_ref() {
            Some(id) => id,
            None if registry.profiles.is_empty() => {
                return Err(ProfileError::Spot(spot::SpotError::NothingRegistered));
            }
            None => return Err(ProfileError::NoSelectedProfile),
        };
        let profile = self.context_from(&registry, id)?;
        let resolved = profile.store.resolve(None, None, None)?;
        Ok(ResolvedSpace {
            profile,
            name: resolved.name,
            site: resolved.site,
            source: resolved.source,
        })
    }

    /// Bind a canonical directory to an existing profile-local space.
    pub fn bind(
        &self,
        profile: &NativeProfileId,
        space: &str,
        directory: &Path,
    ) -> Result<Option<BoundSpace>, ProfileError> {
        let mut registry = self.load_or_bootstrap()?;
        let context = self.context_from(&registry, profile)?;
        let spaces = context.store.load()?;
        if !spaces.spots.contains_key(space) {
            return Err(spot::SpotError::Unknown {
                name: space.to_owned(),
                available: spaces.spots.keys().cloned().collect(),
                binding: None,
            }
            .into());
        }
        let previous = registry.bindings.insert(
            canonical(directory),
            BoundSpace {
                profile: profile.clone(),
                space: space.to_owned(),
            },
        );
        self.save(&registry)?;
        Ok(previous)
    }

    /// Remove the exact nearest binding for a directory.
    pub fn unbind(&self, directory: &Path) -> Result<BoundSpace, ProfileError> {
        let mut registry = self.load_or_bootstrap()?;
        let directory = canonical(directory);
        let binding = registry.bindings.remove(&directory).ok_or_else(|| {
            let ancestor = directory_binding(&registry, &directory)
                .map(|(path, binding)| (path, binding.space));
            ProfileError::Spot(spot::SpotError::NotBound {
                directory: directory.clone(),
                ancestor,
            })
        })?;
        self.save(&registry)?;
        Ok(binding)
    }

    /// Remove every install binding that names one exact profile-local space.
    pub fn remove_space_bindings(
        &self,
        profile: &NativeProfileId,
        space: &str,
    ) -> Result<Vec<PathBuf>, ProfileError> {
        let mut registry = self.load_or_bootstrap()?;
        let removed: Vec<PathBuf> = registry
            .bindings
            .iter()
            .filter(|(_, binding)| &binding.profile == profile && binding.space == space)
            .map(|(directory, _)| directory.clone())
            .collect();
        for directory in &removed {
            registry.bindings.remove(directory);
        }
        self.save(&registry)?;
        Ok(removed)
    }

    /// Remove one profile-local space and then prune only bindings that name
    /// that exact `(profile, space)` pair.
    pub fn remove_space(
        &self,
        profile: &NativeProfileId,
        space: &str,
        data: spot::Data,
        subject: Option<&str>,
    ) -> Result<spot::RemoveOutcome, ProfileError> {
        let mut registry = self.load_or_bootstrap()?;
        let context = self.context_from(&registry, profile)?;
        let mut outcome = match subject {
            Some(subject) => spot::remove_with_subject(&context.store, space, data, subject)?,
            None => spot::remove(&context.store, space, data)?,
        };
        let removed: Vec<PathBuf> = registry
            .bindings
            .iter()
            .filter(|(_, binding)| &binding.profile == profile && binding.space == space)
            .map(|(directory, _)| directory.clone())
            .collect();
        for directory in &removed {
            registry.bindings.remove(directory);
        }
        self.save(&registry)?;
        outcome.unbound = removed;
        Ok(outcome)
    }
}

fn lookup_id(registry: &NativeProfileRegistryV1, id_or_label: &str) -> Option<NativeProfileId> {
    let exact = NativeProfileId(id_or_label.to_owned());
    if registry.profiles.contains_key(&exact) {
        return Some(exact);
    }
    registry
        .profiles
        .iter()
        .find(|(_, record)| record.label.eq_ignore_ascii_case(id_or_label))
        .map(|(id, _)| id.clone())
}

fn validate_label(label: &str) -> Result<(), ProfileError> {
    let mut chars = label.chars();
    let head = chars
        .next()
        .is_some_and(|character| character.is_ascii_lowercase() || character.is_ascii_digit());
    let tail = chars.all(|character| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || character == '-'
            || character == '_'
    });
    if head && tail {
        Ok(())
    } else {
        Err(ProfileError::InvalidLabel(label.to_owned()))
    }
}

fn next_default_label(registry: &NativeProfileRegistryV1) -> String {
    let labels: BTreeSet<String> = registry
        .profiles
        .values()
        .map(|record| record.label.to_ascii_lowercase())
        .collect();
    if !labels.contains("account") {
        return "account".to_owned();
    }
    (2..)
        .map(|suffix| format!("account-{suffix}"))
        .find(|candidate| !labels.contains(candidate))
        .expect("the account label sequence is unbounded")
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn directory_binding(
    registry: &NativeProfileRegistryV1,
    cwd: &Path,
) -> Option<(PathBuf, BoundSpace)> {
    let cwd = canonical(cwd);
    cwd.ancestors().find_map(|directory| {
        registry
            .bindings
            .get_key_value(directory)
            .map(|(path, binding)| (path.clone(), binding.clone()))
    })
}
