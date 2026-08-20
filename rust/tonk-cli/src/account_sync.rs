//! Bounded reconciliation for the spaces owned by one native profile.

use anyhow::{Context, Result};
use dialog_varsig::Did;
use serde::{Deserialize, Serialize};

use crate::account_profiles::{NativeProfileContext, NativeProfileId, ProfileSignIn};
use crate::remote::DEFAULT_REMOTE;
use crate::site::TonkSite;

const CONFIRMED_SITE_PREFIX: &str = "tonk-space-confirmed-v1/";

/// Durable-enrollment state for one local space.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnrollmentPhase {
    /// The profile has no deployment default or the space intentionally has no remote.
    LocalOnly,
    /// Default remote creation began but content durability is not confirmed.
    Provisioning,
    /// Local work exists but the owning profile cannot currently publish it.
    PendingPush,
    /// This exact local tree was accepted by the content remote.
    Connected {
        /// Last locally recorded confirmed tree.
        confirmed: dialog_repository::TreeReference,
    },
    /// A bounded reconciliation step failed.
    Error {
        /// Stable operation label.
        step: &'static str,
        /// Actionable underlying failure.
        detail: String,
    },
}

/// Reconciliation result for one profile-local registry entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReconcileRow {
    /// Profile-local space name.
    pub name: String,
    /// Repository subject, or the profile DID when the site could not open.
    pub subject: Did,
    /// Final enrollment phase for this pass.
    pub phase: EnrollmentPhase,
}

/// Deterministic reconciliation result for one native profile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReconcileReport {
    /// Exact profile that was scanned.
    pub profile: NativeProfileId,
    /// Rows sorted by profile-local space name.
    pub rows: Vec<ReconcileRow>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConfirmedRevisionV1 {
    version: u8,
    tree: String,
}

fn confirmed_site(subject: &Did, account_root: &Did) -> String {
    format!("{CONFIRMED_SITE_PREFIX}{subject}/{account_root}")
}

async fn current_tree(site: &TonkSite) -> Result<Option<dialog_repository::TreeReference>> {
    let branch = site.branch().await.context("failed to open local branch")?;
    Ok(branch.handle().revision().map(|revision| revision.tree))
}

async fn save_confirmed(
    site: &TonkSite,
    account_root: &Did,
    tree: &dialog_repository::TreeReference,
) -> Result<()> {
    let marker = ConfirmedRevisionV1 {
        version: 1,
        tree: tree.to_string(),
    };
    site.profile
        .credential()
        .site(confirmed_site(&site.repository.did(), account_root))
        .save(serde_json::to_vec(&marker)?)
        .perform(&site.operator)
        .await
        .context("failed to save confirmed content revision")
}

/// Record the site's current tree after a successful content push.
pub async fn record_current_revision_confirmed(
    site: &TonkSite,
    account_root: &Did,
) -> Result<bool> {
    let Some(tree) = current_tree(site).await? else {
        return Ok(false);
    };
    save_confirmed(site, account_root, &tree).await?;
    Ok(true)
}

/// Read the confirmed tree for one subject/root pair without contacting a
/// remote. Missing and obsolete markers return `None`.
pub async fn confirmed_revision(site: &TonkSite, account_root: &Did) -> Result<Option<String>> {
    let bytes = match site
        .profile
        .credential()
        .site(confirmed_site(&site.repository.did(), account_root))
        .load::<Vec<u8>>()
        .perform(&site.operator)
        .await
    {
        Ok(bytes) => bytes,
        Err(error) if crate::account_state::credential_is_missing(&error) => return Ok(None),
        Err(error) => return Err(error).context("failed to load confirmed content revision"),
    };
    let marker: ConfirmedRevisionV1 =
        serde_json::from_slice(&bytes).context("confirmed content revision is corrupt")?;
    if marker.version != 1 {
        return Ok(None);
    }
    Ok(Some(marker.tree))
}

/// True only when the local branch still equals its last successful push.
pub async fn current_revision_is_confirmed(site: &TonkSite, account_root: &Did) -> Result<bool> {
    let Some(current) = current_tree(site).await? else {
        return Ok(false);
    };
    Ok(confirmed_revision(site, account_root).await?.as_deref() == Some(&current.to_string()))
}

async fn reconcile_site(
    context: &NativeProfileContext,
    name: String,
    path: &std::path::Path,
) -> ReconcileRow {
    let fallback_subject = match context.open_profile().await {
        Ok(profile) => profile.did(),
        Err(_) => "did:key:z6MkhFDyBYNT1Y1jNj8RJKVc7CWurCVPmrnGEGmbYxvwHJkX"
            .parse()
            .expect("static fallback DID"),
    };
    let site = match TonkSite::open_with(path, context.site_config()).await {
        Ok(site) => site,
        Err(error) => {
            return ReconcileRow {
                name,
                subject: fallback_subject,
                phase: EnrollmentPhase::Error {
                    step: "open",
                    detail: format!("{error:#}"),
                },
            };
        }
    };
    let subject = site.repository.did();
    let Some(root_text) = context.record.account_root.as_deref() else {
        return ReconcileRow {
            name,
            subject,
            phase: EnrollmentPhase::LocalOnly,
        };
    };
    let account_root: Did = match root_text.parse() {
        Ok(root) => root,
        Err(error) => {
            return ReconcileRow {
                name,
                subject,
                phase: EnrollmentPhase::Error {
                    step: "profile",
                    detail: format!("invalid account root: {error}"),
                },
            };
        }
    };

    let upstream = match crate::remote::upstream_remote(&site).await {
        Ok(upstream) => upstream,
        Err(error) => return error_row(name, subject, "inspect", error),
    };
    if upstream.is_none() {
        let (Some(endpoint), Some(relay)) = (
            context.record.default_access_remote.as_deref(),
            context.record.default_revocation_relay.as_deref(),
        ) else {
            return ReconcileRow {
                name,
                subject,
                phase: EnrollmentPhase::LocalOnly,
            };
        };
        if let Err(error) = crate::remote::add_with_revocation(
            &site,
            DEFAULT_REMOTE,
            endpoint,
            Some(subject.clone()),
            Some(relay),
        )
        .await
        {
            return error_row(name, subject, "provision", error);
        }
        if let Err(error) = crate::remote::set_upstream(&site, DEFAULT_REMOTE).await {
            return error_row(name, subject, "provision", error);
        }
    }

    if !matches!(context.sign_in_state(), Ok(ProfileSignIn::Active)) {
        return ReconcileRow {
            name,
            subject,
            phase: EnrollmentPhase::PendingPush,
        };
    }

    let pushed = match crate::sync::push(&site).await {
        Ok(outcome) => Ok(outcome),
        Err(crate::sync::SyncError::NonFastForward) => match crate::sync::pull(&site).await {
            Ok(_) => crate::sync::push(&site).await,
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    };
    if let Err(error) = pushed {
        return error_row(name, subject, "push", error);
    }
    let tree = match current_tree(&site).await {
        Ok(Some(tree)) => tree,
        Ok(None) => {
            return ReconcileRow {
                name,
                subject,
                phase: EnrollmentPhase::Provisioning,
            };
        }
        Err(error) => return error_row(name, subject, "confirm", error),
    };
    if let Err(error) = save_confirmed(&site, &account_root, &tree).await {
        return error_row(name, subject, "confirm", error);
    }
    let prefix = match crate::site::load_account_root_prefix_for(
        &site.profile,
        site.operator.inner(),
        &subject,
        &account_root,
    )
    .await
    {
        Ok(prefix) => prefix,
        Err(error) => return error_row(name, subject, "retain", error),
    };
    let account_operator =
        match crate::account_state::operator_for_store(&site.profile, &context.store).await {
            Ok(operator) => operator,
            Err(error) => return error_row(name, subject, "account-open", error),
        };
    if let Err(error) = crate::account_state::retain_space_delegation_in(
        &site.profile,
        &account_operator,
        &context.store,
        &prefix,
    )
    .await
    {
        return error_row(name, subject, "retain", error);
    }
    let account_branch = match crate::account_state::open_account_branch_in(
        &site.profile,
        &account_operator,
        &context.store,
    )
    .await
    {
        Ok(Some(branch)) => branch,
        Ok(None) => {
            return error_row(
                name,
                subject,
                "account-push",
                "account repository is not hydrated",
            );
        }
        Err(error) => return error_row(name, subject, "account-push", error),
    };
    let upstream = match crate::remote::upstream_remote(&site).await {
        Ok(Some(upstream)) => upstream,
        Ok(None) => return error_row(name, subject, "membership", "space has no upstream"),
        Err(error) => return error_row(name, subject, "membership", error),
    };
    let remote = match crate::remote::find(&site, &upstream).await {
        Ok(Some(remote)) => remote,
        Ok(None) => return error_row(name, subject, "membership", "upstream is not registered"),
        Err(error) => return error_row(name, subject, "membership", error),
    };
    if let Err(error) = tonk_schema::account::record_active_account_space(
        &account_branch,
        tonk_schema::account::AccountSpaceInput {
            account: account_root,
            subject: subject.clone(),
            name: Some(name.clone()),
            remote_url: Some(remote.endpoint),
            revocation_url: remote.revocation_url,
            confirmed_revision: Some(tree.to_string()),
        },
        &account_operator,
    )
    .await
    {
        return error_row(name, subject, "membership", error);
    }
    if let Err(error) = account_branch.push().perform(&account_operator).await {
        return error_row(name, subject, "account-push", error);
    }
    if let Err(error) = crate::account_spots::record_site_in(&name, &site, &context.store).await {
        return error_row(name, subject, "project", error);
    }
    ReconcileRow {
        name,
        subject,
        phase: EnrollmentPhase::Connected { confirmed: tree },
    }
}

fn error_row(
    name: String,
    subject: Did,
    step: &'static str,
    error: impl std::fmt::Display,
) -> ReconcileRow {
    ReconcileRow {
        name,
        subject,
        phase: EnrollmentPhase::Error {
            step,
            detail: error.to_string(),
        },
    }
}

/// Reconcile only the spaces registered in `context`, continuing after each
/// per-space failure and returning rows in stable name order.
pub async fn reconcile_profile(context: &NativeProfileContext) -> ReconcileReport {
    let mut rows = Vec::new();
    match context.store.load() {
        Ok(registry) => {
            for (name, entry) in registry.spots {
                rows.push(reconcile_site(context, name, &entry.site).await);
            }
        }
        Err(error) => {
            let subject = match context.open_profile().await {
                Ok(profile) => profile.did(),
                Err(_) => "did:key:z6MkhFDyBYNT1Y1jNj8RJKVc7CWurCVPmrnGEGmbYxvwHJkX"
                    .parse()
                    .expect("static fallback DID"),
            };
            rows.push(error_row(
                "(registry)".to_owned(),
                subject,
                "registry",
                error,
            ));
        }
    }
    rows.sort_by(|left, right| left.name.cmp(&right.name));
    ReconcileReport {
        profile: context.id.clone(),
        rows,
    }
}

/// Render a stable one-line phase for CLI status output.
pub fn phase_label(phase: &EnrollmentPhase) -> String {
    match phase {
        EnrollmentPhase::LocalOnly => "local-only".to_owned(),
        EnrollmentPhase::Provisioning => "provisioning".to_owned(),
        EnrollmentPhase::PendingPush => "pending push".to_owned(),
        EnrollmentPhase::Connected { confirmed } => format!("connected {confirmed}"),
        EnrollmentPhase::Error { step, detail } => format!("error ({step}): {detail}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_profiles::{NativeProfileId, NativeProfileRecord};
    use crate::site::{SiteConfig, TonkSite};
    use dialog_effects::storage::Directory;

    fn local_context(root: &std::path::Path, label: &str) -> NativeProfileContext {
        let id = NativeProfileId::generate();
        NativeProfileContext {
            record: NativeProfileRecord {
                label: label.to_string(),
                dialog_profile_name: format!("tonk-account-sync-test-{}", id.as_str()),
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

    async fn add_local_space(context: &NativeProfileContext, name: &str) -> anyhow::Result<()> {
        let path = context.store.canonical_site(name);
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
        crate::spot::register_existing_unbound(&context.store, name, &site.root)?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_reconciles_only_the_activated_profile() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let active = local_context(root.path(), "active");
        let inactive = local_context(root.path(), "inactive");
        add_local_space(&active, "active-garden").await?;
        add_local_space(&inactive, "inactive-garden").await?;

        let report = reconcile_profile(&active).await;

        assert_eq!(report.profile, active.id);
        assert_eq!(report.rows.len(), 1);
        assert_eq!(report.rows[0].name, "active-garden");
        assert_eq!(report.rows[0].phase, EnrollmentPhase::LocalOnly);
        assert!(
            inactive.store.load()?.spots.contains_key("inactive-garden"),
            "reconciling the active context must not mutate another profile's registry"
        );
        Ok(())
    }
}
