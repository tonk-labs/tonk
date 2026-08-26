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

use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use dialog_capability::Subject;
use dialog_query::{Output as _, Query, Term};
use dialog_repository::Branch;
use dialog_ucan::UcanDelegation;
use dialog_varsig::Did;
use tonk_schema::Invitation;

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

    let site = crate::site::TonkSite::open_with(&entry.site, config.clone()).await?;
    let subject = site.repository.did();

    // Ownership is the space's own answer, not the registry's, and it is
    // settled before anything about this account's hosting is consulted: who
    // a space belongs to does not depend on where we would have put it. A
    // founder row for somebody else is final; a founder row for us is only
    // finished once the chain behind it is here too, so an interrupted run
    // resumes instead of reporting a link that never completed.
    let roster = crate::inventory::read_roster(&site).await?;
    if let Some(founder) = roster.founder() {
        if founder.did != account.root {
            bail!(already_owned_message(name, &founder.did));
        }
    }
    let connection = crate::account::optional_connection_in(&site.profile, store)
        .await?
        .context("this account is signed out; run `tonk account login` first")?;
    if connection.root_did != account_root {
        bail!(
            "the active account does not match the registry; run `tonk account logout` and sign in again"
        );
    }

    let access = account
        .access_remote
        .as_deref()
        .context("the account has no content endpoint; sign in again")?;
    let existing_cutover = crate::site::load_link_cutover(&site, &account_root).await?;
    let already_linked = roster
        .founder()
        .is_some_and(|founder| founder.did == account.root)
        && existing_cutover
            .as_ref()
            .is_some_and(|cutover| cutover.revocation_published)
        && site.repository.credential().signer().is_none();
    if !already_linked {
        preflight(&site, &roster, access).await?;
    }

    // Every signer-dependent artifact is durable before account or network
    // state changes. A retry after demotion loads this record.
    let (mut cutover, prefix) = crate::site::prepare_link_cutover(&site, &account_root).await?;
    site.profile
        .access()
        .save(UcanDelegation(prefix.clone()))
        .perform(site.operator.local())
        .await
        .context("failed to retain direct account authority locally")?;

    let operator =
        crate::account_state::credential_operator_for_store(&site.profile, store).await?;
    let Some(account_branch) =
        crate::account_state::open_account_branch_in(&site.profile, &operator, store).await?
    else {
        bail!("the account repository is not ready to hold this space");
    };
    crate::account_state::retain_space_delegation_in(&site.profile, &operator, store, &prefix)
        .await?;

    let recipient = crate::custody::account_recipient(&account_branch, &account_root, &operator)
        .await?
        .context(
            "the account has not published its encryption key; open /account in a signed-in browser, then retry",
        )?;
    if !crate::custody::has_custody(&account_branch, &subject, &recipient, &operator).await? {
        let seed = crate::custody::site_seed(&site)
            .await?
            .context("the space seed is not custodied and the local signer is unavailable")?;
        crate::custody::custody_space_seed(&account_branch, &subject, &recipient, &seed, &operator)
            .await?;
    }
    account_branch
        .push()
        .perform(&operator)
        .await
        .context("failed to push account authority and custody")?;

    crate::customer::provision_in(&site.profile, store, &subject, &prefix).await?;

    if crate::remote::upstream_remote(&site).await?.is_none() {
        crate::remote::add(&site, DEFAULT_REMOTE, access, Some(subject.clone())).await?;
        crate::remote::set_upstream(&site, DEFAULT_REMOTE).await?;
    }
    crate::site::record_founder_membership_for(&site, account_root.clone()).await?;
    crate::sync::push(&site).await?;

    if !cutover.revocation_published {
        let artifact =
            hex::decode(&cutover.revocation_hex).context("stored link revocation is not hex")?;
        crate::account::publish_revocation(
            &site.profile,
            &account_branch,
            &operator,
            store,
            &artifact,
        )
        .await
        .context("the old device grant was not revoked; retry `tonk space link`")?;
        crate::site::mark_link_revocation_published(&site, &mut cutover).await?;
    }

    crate::site::demote_repository_signer(&site)
        .await
        .context("the local signer was not demoted; retry `tonk space link`")?;
    let reopened = crate::site::TonkSite::open_with(&entry.site, config.clone()).await?;
    if reopened.repository.credential().signer().is_some() {
        bail!("the local space signer is still present; retry `tonk space link`");
    }

    // Directory publication is last: discovery implies the authority,
    // custody, remote, revocation, and signer cutover are complete.
    match crate::account_spaces::record_site_pushed(name, &site, store).await? {
        crate::account_spaces::RecordOutcome::Recorded
        | crate::account_spaces::RecordOutcome::Unchanged => {}
        outcome => bail!("account directory publication did not complete: {outcome:?}"),
    }

    if crate::inventory::role_for_site(&site).await? != SpaceRole::Owner {
        bail!("the space is not signed as owned by this device after linking");
    }
    Ok(LinkOutcome {
        subject: subject.to_string(),
        name: name.to_owned(),
        site: site.root,
        account: account.root,
        already_linked,
    })
}

/// Refuse anything that is not genuinely local-only.
///
/// An upstream already pointing at this account's own content service is the
/// one exception: that is what a half-finished link leaves behind, and a
/// retry has to be able to get past it.
async fn preflight(site: &crate::site::TonkSite, roster: &Roster, access: &str) -> Result<()> {
    if let Some(name) = crate::remote::upstream_remote(site).await? {
        let endpoint = crate::remote::find(site, &name)
            .await?
            .map(|record| record.endpoint);
        if endpoint.as_deref() != Some(access) {
            bail!("only a local-only space with no content upstream can be linked to an account");
        }
    }
    site.profile
        .access()
        .prove(Subject::from(site.repository.did().clone()))
        .perform(site.operator.local())
        .await
        .context("this device cannot prove authority over this space")?;
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
    Ok(())
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
            subject: Term::var("subject"),
            inviter: Term::var("inviter"),
            audience: Term::var("audience"),
        })
        .perform(&site.operator)
        .try_vec()
        .await?)
}
