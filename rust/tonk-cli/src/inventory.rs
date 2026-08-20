//! Stable CLI space inventory assembled from independent lifecycle facts.

use std::collections::BTreeMap;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use crate::account_profiles::NativeProfileContext;

/// Whether this profile has local repository state for the subject.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LocalPresence {
    /// No local registration or inspectable orphan.
    Absent,
    /// A profile-local registry entry exists.
    Registered,
    /// Local storage exists without a registry entry.
    Orphaned,
}

/// Canonical account membership state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AccountMembership {
    /// No canonical membership fact is present.
    Unassociated,
    /// Canonical membership exists without an archive marker.
    Active,
    /// A monotonic canonical archive marker exists.
    Archived,
}

/// Content-transport configuration state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransportState {
    /// No content remote is configured.
    LocalOnly,
    /// Provisioning has begun but is not confirmed.
    Provisioning,
    /// A content remote is configured.
    Configured,
    /// Transport inspection failed.
    Error,
}

/// Relationship between local and remote content trees.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SyncState {
    /// No fresh comparison is available.
    Unknown,
    /// Local and remote trees match.
    Current,
    /// Local history is ahead.
    Ahead,
    /// Remote history is ahead.
    Behind,
    /// Histories have diverged.
    Diverged,
}

/// Repository authority available to the selected profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AuthorityState {
    /// No usable authority is present.
    Absent,
    /// Authority is retained by the account/profile.
    Retained,
    /// Authority is known to be revoked.
    Revoked,
    /// Authority could not be determined offline.
    Unknown,
}

/// Device-local visibility independent from account membership.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LocalVisibility {
    /// Shown in ordinary local/account inventory.
    Visible,
    /// Explicitly hidden on this profile/device.
    HiddenOnThisDevice,
}

/// Stable version-one JSON inventory row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpaceInventoryRowV1 {
    /// Schema version, exactly one.
    pub version: u8,
    /// Native profile identifier.
    pub profile: String,
    /// Repository subject DID.
    pub subject: String,
    /// Preferred display name; local registration wins over remote metadata.
    pub name: Option<String>,
    /// Independent local presence axis.
    pub local_presence: LocalPresence,
    /// Independent canonical account membership axis.
    pub account_membership: AccountMembership,
    /// Independent content transport axis.
    pub transport: TransportState,
    /// Independent content revision relationship.
    pub sync: SyncState,
    /// Independent authority axis.
    pub authority: AuthorityState,
    /// Independent profile/device visibility axis.
    pub visibility: LocalVisibility,
    /// Exact tree accepted by the content remote, when confirmed.
    pub confirmed_revision: Option<String>,
    /// Whether an explicit ordinary pull can safely proceed.
    pub pullable: bool,
}

#[derive(Clone, Debug)]
struct InventoryInput {
    profile: String,
    subject: String,
    local_name: Option<String>,
    remote_name: Option<String>,
    local_presence: LocalPresence,
    account_membership: AccountMembership,
    transport: TransportState,
    sync: SyncState,
    authority: AuthorityState,
    suppressed: bool,
    confirmed_revision: Option<String>,
    ambiguous: bool,
}

impl InventoryInput {
    fn absent(profile: &NativeProfileContext, subject: String) -> Self {
        Self {
            profile: profile.id.to_string(),
            subject,
            local_name: None,
            remote_name: None,
            local_presence: LocalPresence::Absent,
            account_membership: AccountMembership::Unassociated,
            transport: TransportState::LocalOnly,
            sync: SyncState::Unknown,
            authority: AuthorityState::Unknown,
            suppressed: false,
            confirmed_revision: None,
            ambiguous: false,
        }
    }
}

/// Read-only inventory controls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InventoryOptions {
    /// Include suppressed, archived, ambiguous, and orphaned rows.
    pub all: bool,
    /// Fetch remote content heads without merging or writing.
    pub refresh: bool,
}

fn normalize(inputs: Vec<InventoryInput>, all: bool) -> Vec<SpaceInventoryRowV1> {
    let mut rows = inputs
        .into_iter()
        .map(|input| {
            let visibility = if input.suppressed {
                LocalVisibility::HiddenOnThisDevice
            } else {
                LocalVisibility::Visible
            };
            let pullable = input.local_presence == LocalPresence::Absent
                && input.account_membership == AccountMembership::Active
                && input.transport == TransportState::Configured
                && input.authority == AuthorityState::Retained
                && !input.ambiguous;
            SpaceInventoryRowV1 {
                version: 1,
                profile: input.profile,
                subject: input.subject,
                name: input.local_name.or(input.remote_name),
                local_presence: input.local_presence,
                account_membership: input.account_membership,
                transport: input.transport,
                sync: input.sync,
                authority: input.authority,
                visibility,
                confirmed_revision: input.confirmed_revision,
                pullable,
            }
        })
        .filter(|row| {
            all || row.visibility == LocalVisibility::Visible
                && (row.local_presence == LocalPresence::Registered
                    || row.account_membership == AccountMembership::Active)
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| left.subject.cmp(&right.subject));
    rows
}

/// Assemble one stable inventory for an exact native profile.
///
/// The default path opens only local profile/account state. `refresh` adds
/// provider rows and remote head observations but never pulls, pushes,
/// provisions, archives, or changes a binding.
pub async fn list(
    context: &NativeProfileContext,
    options: InventoryOptions,
) -> Result<Vec<SpaceInventoryRowV1>> {
    let registry = context.store.load()?;
    let profile = context.load_profile().await?;
    let mut inputs: BTreeMap<String, InventoryInput> = BTreeMap::new();

    for (name, entry) in &registry.spots {
        let site = crate::site::TonkSite::open_with(&entry.site, context.site_config())
            .await
            .with_context(|| format!("failed to inspect registered space '{name}'"))?;
        let subject = site.repository.did().to_string();
        let input = inputs
            .entry(subject.clone())
            .or_insert_with(|| InventoryInput::absent(context, subject.clone()));
        input.local_name = Some(name.clone());
        input.local_presence = LocalPresence::Registered;
        input.suppressed = registry.is_suppressed(&subject);
        input.transport = match crate::remote::upstream_remote(&site).await {
            Ok(Some(_)) => TransportState::Configured,
            Ok(None) => TransportState::LocalOnly,
            Err(_) => TransportState::Error,
        };
        if let Some(root) = context.record.account_root.as_deref() {
            input.authority = match root.parse().ok() {
                Some(root) if crate::site::account_root_prefix(&site, &root).await.is_ok() => {
                    AuthorityState::Retained
                }
                Some(_) => AuthorityState::Absent,
                None => AuthorityState::Unknown,
            };
        }
        if options.refresh && input.transport == TransportState::Configured {
            input.sync = match crate::sync::status_with_hash(&site).await {
                Ok(status) => match status.state {
                    tonk_schema::SyncState::Synced => SyncState::Current,
                    tonk_schema::SyncState::Ahead => SyncState::Ahead,
                    tonk_schema::SyncState::Behind => SyncState::Behind,
                    tonk_schema::SyncState::Diverged => SyncState::Diverged,
                    tonk_schema::SyncState::NoUpstream => SyncState::Unknown,
                },
                Err(_) => SyncState::Unknown,
            };
        }
    }

    if options.all {
        for path in context.store.orphaned_sites(&registry) {
            let Ok(site) = crate::site::TonkSite::open_with(&path, context.site_config()).await
            else {
                continue;
            };
            let subject = site.repository.did().to_string();
            let input = inputs
                .entry(subject.clone())
                .or_insert_with(|| InventoryInput::absent(context, subject.clone()));
            input.local_presence = LocalPresence::Orphaned;
            input.suppressed = registry.is_suppressed(&subject);
        }
    }

    if let Some(account_root) = context.record.account_root.as_deref() {
        let account_root = account_root
            .parse()
            .context("native profile account root is invalid")?;
        let operator = crate::account_state::operator_for_store(&profile, &context.store).await?;
        if let Some(branch) =
            crate::account_state::open_account_branch_in(&profile, &operator, &context.store)
                .await?
        {
            for record in tonk_schema::account::list_account_spaces(&branch, &operator).await? {
                if record.account != account_root {
                    continue;
                }
                let subject = record.subject.to_string();
                let input = inputs
                    .entry(subject.clone())
                    .or_insert_with(|| InventoryInput::absent(context, subject.clone()));
                input.account_membership = if record.archived {
                    AccountMembership::Archived
                } else {
                    AccountMembership::Active
                };
                if record.name.is_some() {
                    input.remote_name = record.name;
                }
                if record.remote_url.is_some() {
                    input.transport = TransportState::Configured;
                }
                if record.confirmed_revision.is_some() {
                    input.confirmed_revision = record.confirmed_revision;
                }
                let retained_by_account = branch
                    .delegations()
                    .prove(
                        account_root.clone(),
                        dialog_ucan::Scope {
                            subject: dialog_ucan_core::subject::Subject::Specific(
                                record.subject.clone(),
                            ),
                            command: dialog_ucan_core::command::Command::parse("/")
                                .expect("root command is valid"),
                            parameters: dialog_ucan::Parameters::default(),
                        },
                    )
                    .perform(&operator)
                    .await
                    .is_ok();
                let retained_by_profile = crate::site::inspect_account_root_prefix_for(
                    &profile,
                    &operator,
                    &record.subject,
                    &account_root,
                )
                .await
                .is_ok();
                input.authority = if retained_by_account || retained_by_profile {
                    AuthorityState::Retained
                } else if input.authority == AuthorityState::Retained {
                    // A refreshed provider artifact is not canonical
                    // membership, but its verified root-ending chain is
                    // cryptographic authority evidence in its own right.
                    AuthorityState::Retained
                } else {
                    AuthorityState::Absent
                };
                input.suppressed = registry.is_suppressed(&subject);
            }
            for input in inputs.values_mut() {
                if input.account_membership != AccountMembership::Active
                    || input.authority != AuthorityState::Unknown
                {
                    continue;
                }
                let Ok(subject) = input.subject.parse() else {
                    input.authority = AuthorityState::Absent;
                    continue;
                };
                input.authority = if crate::site::inspect_account_root_prefix_for(
                    &profile,
                    &operator,
                    &subject,
                    &account_root,
                )
                .await
                .is_ok()
                {
                    AuthorityState::Retained
                } else {
                    AuthorityState::Absent
                };
            }
        }
    }

    Ok(normalize(inputs.into_values().collect(), options.all))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_profiles::{NativeProfileId, NativeProfileRecord};
    use crate::site::{SiteConfig, TonkSite};
    use dialog_effects::storage::Directory;

    fn input(subject: &str) -> InventoryInput {
        InventoryInput {
            profile: "personal".into(),
            subject: subject.into(),
            local_name: None,
            remote_name: None,
            local_presence: LocalPresence::Absent,
            account_membership: AccountMembership::Unassociated,
            transport: TransportState::LocalOnly,
            sync: SyncState::Unknown,
            authority: AuthorityState::Unknown,
            suppressed: false,
            confirmed_revision: None,
            ambiguous: false,
        }
    }

    fn local_context(root: &std::path::Path) -> NativeProfileContext {
        let id = NativeProfileId::generate();
        NativeProfileContext {
            record: NativeProfileRecord {
                label: "inventory".to_string(),
                dialog_profile_name: format!("tonk-inventory-test-{}", id.as_str()),
                account_root: None,
                ceremony_origin: None,
                default_access_remote: None,
                default_revocation_relay: None,
                extra: serde_json::Map::new(),
            },
            store: crate::spot::SpotStore::at(root.join(id.as_str())),
            id,
        }
    }

    async fn configured_local_space(
        context: &NativeProfileContext,
        endpoint: &str,
    ) -> anyhow::Result<TonkSite> {
        let path = context.store.canonical_site("garden");
        let site = TonkSite::init_at_with(
            &path,
            SiteConfig {
                profile_name: context.record.dialog_profile_name.clone(),
                profile_directory: Directory::Profile,
                require_account: false,
                account_store: context.store.clone(),
            },
        )
        .await?;
        crate::remote::add(&site, crate::remote::DEFAULT_REMOTE, endpoint, None).await?;
        crate::remote::set_upstream(&site, crate::remote::DEFAULT_REMOTE).await?;
        crate::spot::register_existing_unbound(&context.store, "garden", &site.root)?;
        Ok(site)
    }

    #[test]
    fn it_normalizes_lifecycle_axes_without_collapsing_them() {
        let rows = normalize(
            vec![
                InventoryInput {
                    profile: "personal".to_string(),
                    subject: "did:key:zLocal".to_string(),
                    local_name: Some("garden".to_string()),
                    remote_name: Some("remote-garden".to_string()),
                    local_presence: LocalPresence::Registered,
                    account_membership: AccountMembership::Active,
                    transport: TransportState::Configured,
                    sync: SyncState::Current,
                    authority: AuthorityState::Retained,
                    suppressed: false,
                    confirmed_revision: Some("#tree".to_string()),
                    ambiguous: false,
                },
                InventoryInput {
                    profile: "personal".to_string(),
                    subject: "did:key:zRemote".to_string(),
                    local_name: None,
                    remote_name: Some("shared".to_string()),
                    local_presence: LocalPresence::Absent,
                    account_membership: AccountMembership::Active,
                    transport: TransportState::Configured,
                    sync: SyncState::Unknown,
                    authority: AuthorityState::Retained,
                    suppressed: true,
                    confirmed_revision: Some("#other".to_string()),
                    ambiguous: false,
                },
                InventoryInput {
                    profile: "personal".to_string(),
                    subject: "did:key:zArchived".to_string(),
                    local_name: Some("old".to_string()),
                    remote_name: None,
                    local_presence: LocalPresence::Registered,
                    account_membership: AccountMembership::Archived,
                    transport: TransportState::Configured,
                    sync: SyncState::Unknown,
                    authority: AuthorityState::Retained,
                    suppressed: false,
                    confirmed_revision: None,
                    ambiguous: false,
                },
            ],
            true,
        );

        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].name.as_deref(), Some("old"));
        assert!(!rows[0].pullable);
        assert_eq!(rows[1].name.as_deref(), Some("garden"));
        assert_eq!(rows[2].visibility, LocalVisibility::HiddenOnThisDevice);
        assert!(rows[2].pullable);
        let json = serde_json::to_value(&rows[2]).unwrap();
        assert_eq!(json["localPresence"], "absent");
        assert_eq!(json["accountMembership"], "active");
        assert_eq!(json["visibility"], "hiddenOnThisDevice");
        assert_eq!(json["confirmedRevision"], "#other");
    }

    #[test]
    fn it_covers_the_space_inventory_lifecycle_table() {
        let mut local_only = input("01-local-only");
        local_only.local_presence = LocalPresence::Registered;
        let mut local_current = input("02-local-current");
        local_current.local_presence = LocalPresence::Registered;
        local_current.account_membership = AccountMembership::Active;
        local_current.transport = TransportState::Configured;
        local_current.sync = SyncState::Current;
        local_current.authority = AuthorityState::Retained;
        let mut local_ahead = local_current.clone();
        local_ahead.subject = "03-local-ahead".into();
        local_ahead.sync = SyncState::Ahead;
        let mut pullable = input("04-account-only");
        pullable.account_membership = AccountMembership::Active;
        pullable.transport = TransportState::Configured;
        pullable.authority = AuthorityState::Retained;
        let mut ambiguous = pullable.clone();
        ambiguous.subject = "05-ambiguous".into();
        ambiguous.ambiguous = true;
        let mut suppressed = pullable.clone();
        suppressed.subject = "06-suppressed".into();
        suppressed.suppressed = true;
        let mut archived_local = local_current.clone();
        archived_local.subject = "07-archived-local".into();
        archived_local.account_membership = AccountMembership::Archived;
        let mut archived_absent = pullable.clone();
        archived_absent.subject = "08-archived-absent".into();
        archived_absent.account_membership = AccountMembership::Archived;
        let mut orphaned = input("09-orphaned");
        orphaned.local_presence = LocalPresence::Orphaned;
        let mut missing_authority = pullable.clone();
        missing_authority.subject = "10-missing-authority".into();
        missing_authority.authority = AuthorityState::Absent;
        let mut provider_legacy = pullable.clone();
        provider_legacy.subject = "11-provider-legacy".into();

        let rows = normalize(
            vec![
                local_only,
                local_current,
                local_ahead,
                pullable,
                ambiguous,
                suppressed,
                archived_local,
                archived_absent,
                orphaned,
                missing_authority,
                provider_legacy,
            ],
            true,
        );
        assert_eq!(rows.len(), 11);
        let pullability: Vec<_> = rows.iter().map(|row| row.pullable).collect();
        assert_eq!(
            pullability,
            vec![
                false, false, false, true, false, true, false, false, false, false, true
            ]
        );
        assert_eq!(rows[1].sync, SyncState::Current);
        assert_eq!(rows[2].sync, SyncState::Ahead);
        assert_eq!(rows[5].visibility, LocalVisibility::HiddenOnThisDevice);
        assert_eq!(rows[6].account_membership, AccountMembership::Archived);
        assert_eq!(rows[8].local_presence, LocalPresence::Orphaned);
    }

    #[dialog_common::test]
    async fn it_lists_offline_without_provider_or_remote_requests() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let context = local_context(root.path());
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let endpoint = format!("http://{}/ucan/", listener.local_addr()?);
        configured_local_space(&context, &endpoint).await?;

        let rows = list(&context, InventoryOptions::default()).await?;

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].local_presence, LocalPresence::Registered);
        assert_eq!(rows[0].transport, TransportState::Configured);
        assert_eq!(rows[0].sync, SyncState::Unknown);
        assert!(
            matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
            "offline inventory must not contact the configured content remote"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_refreshes_read_only() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let context = local_context(root.path());
        let site = configured_local_space(&context, "http://127.0.0.1:9/ucan/").await?;
        let registry_path = context.store.registry_path();
        let registry_before = std::fs::read(&registry_path)?;
        let revision_before = site.branch().await?.handle().revision();
        let remote_before =
            serde_json::to_value(crate::remote::find(&site, crate::remote::DEFAULT_REMOTE).await?)?;

        let rows = list(
            &context,
            InventoryOptions {
                all: true,
                refresh: true,
            },
        )
        .await?;

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].sync, SyncState::Unknown);
        assert_eq!(std::fs::read(&registry_path)?, registry_before);
        assert_eq!(site.branch().await?.handle().revision(), revision_before);
        assert_eq!(
            serde_json::to_value(crate::remote::find(&site, crate::remote::DEFAULT_REMOTE).await?,)?,
            remote_before,
            "refresh may inspect but must not rewrite remote configuration"
        );
        Ok(())
    }
}
