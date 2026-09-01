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
use dialog_varsig::Did;
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
        .as_deref()
        .context("the account has no content endpoint; sign in again")?;
    preflight(&site, &roster, access, already_linked).await?;

    // Authority first: the account root can only host what it can prove it
    // was given, and this is the one boundary allowed to mint that grant.
    let prefix = crate::site::account_root_prefix(&site, &account_root).await?;
    crate::customer::provision_in(&site.profile, store, &subject, &prefix).await?;

    if crate::remote::upstream_remote(&site).await?.is_none() {
        crate::remote::add(&site, DEFAULT_REMOTE, access, Some(subject.clone())).await?;
        crate::remote::set_upstream(&site, DEFAULT_REMOTE).await?;
    }
    crate::site::record_founder_membership(&site).await?;
    crate::sync::push(&site).await?;

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
        match crate::custody::account_recipient(&account_branch, &account_root, &operator).await? {
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
    Ok(LinkOutcome {
        subject: subject.to_string(),
        name: name.to_owned(),
        site: site.root,
        account: account.root,
        already_linked,
    })
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
) -> Result<()> {
    if let Some(name) = crate::remote::upstream_remote(site).await? {
        let endpoint = crate::remote::find(site, &name)
            .await?
            .map(|record| record.endpoint);
        if endpoint.as_deref() != Some(access) {
            bail!("only a local-only space with no content upstream can be linked to an account");
        }
    }
    // Once founder ownership and its retained account chain agree, this is no
    // longer an ownership transition. Shares and members created afterwards
    // are expected; a retry only needs to finish the idempotent hosting and
    // account-directory steps below. Keep the upstream check above so a retry
    // never silently republishes a space through a different service.
    if already_linked {
        return Ok(());
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
            subject: Term::from(site.repository.did().this()),
            inviter: Term::var("inviter"),
            audience: Term::var("audience"),
        })
        .perform(&site.operator)
        .try_vec()
        .await?)
}
