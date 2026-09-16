//! Linking one local-only space into the signed-in account.
//!
//! This is the only ownership transition tonk performs: a space that belongs
//! to no account becomes a space the signed-in account owns. It never runs in
//! reverse and never moves a space between accounts — a synced space stays
//! with its owner so the shares already handed out keep working.
//!
//! Nothing here is destructive, so a failed attempt leaves a usable local
//! space and a retry converges: every step is either idempotent or guarded by
//! the state the previous run left behind. Ownership is settled by the founder
//! row on the space's own content branch, confirmed against the retained
//! `subject → … → account root` chain, so an interrupted run is visible as
//! exactly what it is — a half-finished link, not a finished one.

use std::future::Future;
use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use dialog_capability::Subject;
use dialog_query::{Output as _, Query, Term};
use dialog_repository::{Branch, Upstream};
use dialog_varsig::Did;
use thiserror::Error;
use tonk_schema::Invitation;
use tonk_schema::prelude::DidExt as _;

use crate::inventory::{Roster, SpaceRole};
use crate::remote::DEFAULT_REMOTE;
use crate::site::SiteConfig;
use crate::space::SpaceStore;

/// Successful link result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkOutcome {
    /// Repository subject, unchanged by the link.
    pub subject: String,
    /// Registered space name, unchanged by the link.
    pub name: String,
    /// Local site directory, unchanged by the link.
    pub site: PathBuf,
    /// Account root the space now belongs to.
    pub account: String,
    /// Whether the space already belonged to this account.
    pub already_linked: bool,
}

/// Stable checkpoints in account publication of an already-local space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicationStage {
    /// Record the signed account as founder and provision its authority.
    Founder,
    /// Ensure the account's content service is registered under the selected remote.
    Remote,
    /// Point the content and metadata branches at the selected remote.
    Upstream,
    /// Publish content and metadata to the configured service.
    Push,
    /// Retain custody/authority and publish the account directory entry.
    AccountDirectory,
}

impl PublicationStage {
    /// Stable stage identifier used in recovery output and fault tests.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Founder => "founder",
            Self::Remote => "remote",
            Self::Upstream => "upstream",
            Self::Push => "push",
            Self::AccountDirectory => "accountDirectory",
        }
    }
}

impl std::fmt::Display for PublicationStage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A locally registered space whose account publication stopped at one stage.
#[derive(Debug, Error)]
#[error("stage '{stage}' failed: {source}")]
pub struct PublicationError {
    /// Exact idempotent publication stage that did not settle successfully.
    pub stage: PublicationStage,
    /// Underlying failure at that stage.
    #[source]
    pub source: anyhow::Error,
}

/// Explain why an account-owned space cannot be linked somewhere else.
pub fn already_owned_message(name: &str, owner: &str) -> String {
    format!(
        "\"{name}\" already belongs to an account, so it stays there.\n\n\
         Once a space is synced with an account, it stays owned by that account.\n\
         This keeps existing shares working.\n\n\
         Share it instead:\n  tonk invite\n\n\
         owner account: {owner}"
    )
}

/// Link one genuinely local-only space into the signed-in account.
pub async fn execute(store: &SpaceStore, config: &SiteConfig, name: &str) -> Result<LinkOutcome> {
    let prepared = prepare(store, config, name).await?;
    publish(store, name, prepared)
        .await
        .map_err(anyhow::Error::new)
}

/// Finish publication for a freshly created, already registered local space.
///
/// Once `space::create` returns, the local site, DID, registry entry, and
/// directory binding are durable. Every later failure is therefore returned
/// as a typed partial outcome and is safe to continue with [`execute`].
pub async fn publish_created(
    store: &SpaceStore,
    config: &SiteConfig,
    name: &str,
) -> std::result::Result<LinkOutcome, PublicationError> {
    let prepared = prepare(store, config, name)
        .await
        .map_err(|source| PublicationError {
            stage: PublicationStage::Founder,
            source,
        })?;
    publish(store, name, prepared).await
}

struct PreparedPublication {
    site: crate::site::TonkSite,
    subject: Did,
    account_root: Did,
    account: String,
    access: String,
    remote: String,
    already_linked: bool,
}

async fn prepare(
    store: &SpaceStore,
    config: &SiteConfig,
    name: &str,
) -> Result<PreparedPublication> {
    let registry = store.load()?;
    let account = registry
        .account
        .clone()
        .context("no account is signed in; run `tonk account login` first")?;
    let entry = registry
        .spaces
        .get(name)
        .with_context(|| format!("unknown space '{name}'"))?
        .clone();
    let account_root: Did = account
        .root
        .parse()
        .context("the signed-in account root is invalid")?;

    let mut site_config = config.clone();
    site_config.require_account = false;
    let site = crate::site::TonkSite::open_with(&entry.site, site_config).await?;
    let subject = site.repository.did();

    // Ownership is the space's own answer, not the registry's, and it is
    // settled before anything about this account's hosting is consulted: who
    // a space belongs to does not depend on where we would have put it. A
    // founder row for somebody else is final; a founder row for us is only
    // finished once the chain behind it is here too, so an interrupted run
    // resumes instead of reporting a link that never completed.
    let roster = crate::inventory::read_roster(&site).await?;
    let mut already_linked = false;
    if let Some(founder) = roster.founder() {
        if founder.did != account.root {
            bail!(already_owned_message(name, &founder.did));
        }
        if holds_account_chain(&site, &subject, &account_root).await {
            already_linked = true;
        }
    }

    match crate::account::status_in(&site.profile, store).await? {
        crate::account::AccountStatus::Registered { root_did, .. } if root_did == account.root => {}
        _ => bail!("this account is signed out; run `tonk account login` first"),
    }

    let access = account
        .access_remote
        .clone()
        .context("the account has no content endpoint; sign in again")?;
    let remote = preflight(&site, &roster, &access, already_linked)
        .await?
        .unwrap_or_else(|| DEFAULT_REMOTE.to_owned());

    Ok(PreparedPublication {
        site,
        subject,
        account_root,
        account: account.root,
        access,
        remote,
        already_linked,
    })
}

async fn publish(
    store: &SpaceStore,
    name: &str,
    prepared: PreparedPublication,
) -> std::result::Result<LinkOutcome, PublicationError> {
    let PreparedPublication {
        site,
        subject,
        account_root,
        account,
        access,
        remote,
        already_linked,
    } = prepared;

    // Authority first: the account root can only host what it can prove it
    // was given, and this is the one boundary allowed to mint that grant.
    let prefix = publication_stage(PublicationStage::Founder, async {
        let prefix = crate::site::account_root_prefix(&site, &account_root).await?;
        crate::customer::provision_in(&site.profile, store, &subject, &prefix).await?;
        crate::site::record_founder_membership(&site).await?;
        Ok(prefix)
    })
    .await?;

    publication_stage(
        PublicationStage::Remote,
        ensure_remote(&site, &remote, &access, &subject),
    )
    .await?;
    publication_stage(PublicationStage::Upstream, ensure_upstream(&site, &remote)).await?;
    publication_stage(PublicationStage::Push, async {
        crate::sync::push(&site).await?;
        Ok(())
    })
    .await?;

    publication_stage(PublicationStage::AccountDirectory, async {
        let operator =
            crate::account_state::credential_operator_for_store(&site.profile, store).await?;
        let Some(account_branch) =
            crate::account_state::open_account_branch_in(&site.profile, &operator, store).await?
        else {
            bail!("the account repository is not ready to hold this space");
        };
        crate::account_state::retain_space_delegation_in(&site.profile, &operator, store, &prefix)
            .await?;
        // The seed rides the same boundary: a space the account hosts is a
        // space the account can re-derive. Sealing needs only the published
        // public key; an account that predates it links anyway — custody
        // catches up at the next `tonk account login`.
        if !crate::custody::has_custody(&account_branch, &subject, &operator).await? {
            match crate::custody::account_recipient(&account_branch, &account_root, &operator)
                .await?
            {
                Some(recipient) => {
                    if let Some(seed) = crate::custody::site_seed(&site).await? {
                        crate::custody::custody_space_seed(
                            &account_branch,
                            &subject,
                            &recipient,
                            &seed,
                            &operator,
                        )
                        .await?;
                    }
                }
                None => eprintln!(
                    "warning: the account has not published its encryption key; \
                     the space seed stays uncustodied until it does"
                ),
            }
        }
        crate::account_spaces::record_site_pushed(name, &site, store).await?;

        if crate::inventory::role_for_site(&site).await? != SpaceRole::Owner {
            bail!("the space is not signed as owned by this device after linking");
        }
        Ok(())
    })
    .await?;

    Ok(LinkOutcome {
        subject: subject.to_string(),
        name: name.to_owned(),
        site: site.root,
        account,
        already_linked,
    })
}

async fn ensure_remote(
    site: &crate::site::TonkSite,
    name: &str,
    access: &str,
    subject: &Did,
) -> Result<()> {
    crate::remote::ensure(site, name, access, subject.clone()).await?;
    Ok(())
}

async fn ensure_upstream(site: &crate::site::TonkSite, expected_remote: &str) -> Result<()> {
    match configured_upstream(site).await? {
        Some(Upstream::Remote { remote, branch, .. })
            if remote == expected_remote && branch == crate::site::BRANCH_NAME => {}
        Some(Upstream::Remote { remote, branch, .. }) => bail!(
            "the space already tracks '{remote}/{branch}'; refusing to replace it with \
             '{expected_remote}/{main}'",
            main = crate::site::BRANCH_NAME,
        ),
        Some(Upstream::Local { branch, .. }) => bail!(
            "the space already tracks local branch '{branch}'; refusing to replace it with \
             '{expected_remote}/{main}'",
            main = crate::site::BRANCH_NAME,
        ),
        None => {}
    }
    // Re-run even when main already points at origin: an interrupted prior
    // attempt may still need to wire metadata or assert its tracking record.
    crate::remote::set_upstream(site, expected_remote).await?;
    Ok(())
}

async fn configured_upstream(site: &crate::site::TonkSite) -> Result<Option<Upstream>> {
    let session = site
        .branch()
        .await
        .context("failed to inspect the space's upstream")?;
    Ok(session.handle().upstream())
}

async fn publication_stage<T>(
    stage: PublicationStage,
    future: impl Future<Output = Result<T>>,
) -> std::result::Result<T, PublicationError> {
    let outcome = future
        .await
        .map_err(|source| PublicationError { stage, source })?;
    inject_failure_after(stage)?;
    Ok(outcome)
}

#[cfg(feature = "integration-tests")]
fn inject_failure_after(stage: PublicationStage) -> std::result::Result<(), PublicationError> {
    const ENV: &str = "TONK_TEST_SPACE_NEW_FAIL_STAGE";
    if std::env::var(ENV).ok().as_deref() == Some(stage.as_str()) {
        return Err(PublicationError {
            stage,
            source: anyhow::anyhow!(
                "injected failure after the stage completed; its outcome must be treated as unknown"
            ),
        });
    }
    Ok(())
}

#[cfg(not(feature = "integration-tests"))]
fn inject_failure_after(_stage: PublicationStage) -> std::result::Result<(), PublicationError> {
    Ok(())
}

/// Whether the retained `subject → … → account root` chain is on this device.
///
/// The chain is what the access service validates and the roster is its
/// legible, synced mirror, so a founder row with no chain behind it is an
/// unfinished link rather than an ownership claim. This reads only what the
/// profile already holds — it never mints, so it cannot answer yes by
/// establishing the ownership it was asked to confirm.
async fn holds_account_chain(
    site: &crate::site::TonkSite,
    subject: &Did,
    account_root: &Did,
) -> bool {
    crate::site::load_account_root_prefix_for(
        &site.profile,
        site.operator.local(),
        subject,
        account_root,
    )
    .await
    .is_ok()
}

/// Refuse anything that is not genuinely local-only.
///
/// An upstream already pointing at this account's own content service is the
/// one exception: that is what a half-finished link leaves behind, and a
/// retry has to be able to get past it.
async fn preflight(
    site: &crate::site::TonkSite,
    roster: &Roster,
    access: &str,
    already_linked: bool,
) -> Result<Option<String>> {
    let existing_remote = match configured_upstream(site).await? {
        Some(Upstream::Remote { remote, branch, .. }) if branch == crate::site::BRANCH_NAME => {
            let endpoint = crate::remote::find(site, &remote)
                .await?
                .map(|record| record.endpoint);
            if endpoint.as_deref() != Some(access) {
                bail!(
                    "only a local-only space with no content upstream, or an interrupted \
                     link to this account's content endpoint, can be linked to an account"
                );
            }
            Some(remote)
        }
        Some(_) => bail!(
            "only a local-only space with no content upstream, or an interrupted link to \
             this account's content endpoint, can be linked to an account"
        ),
        None => None,
    };
    // Once founder ownership and its retained account chain agree, this is no
    // longer an ownership transition. Shares and members created afterwards
    // are expected; a retry only needs to finish the idempotent hosting and
    // account-directory steps below. Keep the upstream check above so a retry
    // never silently republishes a space through a different service.
    if already_linked {
        return Ok(existing_remote);
    }
    let profile_proof = site
        .profile
        .access()
        .prove(Subject::from(site.repository.did().clone()))
        .perform(site.operator.local())
        .await;
    if let Err(error) = profile_proof
        && site.repository.credential().signer().is_none()
    {
        return Err(error).context("this device cannot prove authority over this space");
    }
    if has_invitations(site).await? {
        bail!("a space with recorded shares cannot be linked to an account");
    }
    // Every identity this installation could have written a row under — the
    // account, the local root, the profile — counts as us; anything else is
    // a member this link would silently carry into the account.
    let ours = crate::site::Identity::of(site).await?;
    if roster
        .members
        .iter()
        .any(|member| !ours.dids().any(|did| did == member.did))
    {
        bail!("a space with another durable member cannot be linked to an account");
    }
    Ok(existing_remote)
}

/// Whether the space records any share it has already handed out.
///
/// Reads the content branch, where the worker writes invitations, and the
/// meta branch, where CLI releases through this one wrote them.
async fn has_invitations(site: &crate::site::TonkSite) -> Result<bool> {
    let content = site
        .branch()
        .await
        .context("failed to inspect the space's shares")?;
    if !invitations_on(site, content.handle()).await?.is_empty() {
        return Ok(true);
    }
    let meta = site
        .repository
        .branch(crate::remote::META_BRANCH)
        .open()
        .perform(&site.operator)
        .await
        .context("failed to inspect local-space metadata")?;
    Ok(!invitations_on(site, &meta).await?.is_empty())
}

async fn invitations_on(site: &crate::site::TonkSite, branch: &Branch) -> Result<Vec<Invitation>> {
    Ok(branch
        .query()
        .select(Query::<Invitation> {
            this: Term::var("this"),
            subject: Term::from(site.repository.did().this()),
            inviter: Term::var("inviter"),
            audience: Term::var("audience"),
        })
        .perform(&site.operator)
        .try_vec()
        .await?)
}
