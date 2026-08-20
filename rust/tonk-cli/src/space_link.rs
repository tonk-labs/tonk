//! Linking one local-only space into the signed-in account.
//!
//! This is the only ownership transition tonk performs: a space that belongs
//! to no account becomes a space the signed-in account owns. It never runs in
//! reverse and never moves a space between accounts — a synced space stays
//! with its owner so the shares already handed out keep working.
//!
//! Nothing here is destructive, so a failed attempt leaves a usable local
//! space and a retry converges: every step is either idempotent or guarded by
//! the state the previous run left behind. The registry is tagged last, so a
//! space counts as the account's only once its content, authority, and
//! directory record are all in place.

use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use dialog_capability::Subject;
use dialog_query::{Output as _, Query, Term};
use dialog_varsig::Did;
use tonk_schema::prelude::DidExt as _;
use tonk_schema::{Invitation, MemberRole, Membership};

use crate::inventory::SpaceRole;
use crate::remote::DEFAULT_REMOTE;
use crate::site::SiteConfig;
use crate::spot::SpotStore;

/// Successful link result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkOutcome {
    /// Repository subject, unchanged by the link. Absent when the space
    /// already belonged to this account and no site was opened to read it.
    pub subject: Option<String>,
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
pub async fn execute(store: &SpotStore, config: &SiteConfig, name: &str) -> Result<LinkOutcome> {
    let registry = store.load()?;
    let account = registry
        .account
        .clone()
        .context("no account is signed in; run `tonk account link` first")?;
    let entry = registry
        .spots
        .get(name)
        .with_context(|| format!("unknown space '{name}'"))?
        .clone();
    if let Some(owner) = entry.account.as_deref() {
        if owner == account.root {
            return Ok(LinkOutcome {
                subject: None,
                name: name.to_owned(),
                site: entry.site.clone(),
                account: account.root,
                already_linked: true,
            });
        }
        bail!(already_owned_message(name, owner));
    }
    if crate::account::sign_in_phase(store)? != crate::account::SignInPhase::Active {
        bail!("this account is signed out; run `tonk account login` first");
    }
    let account_root: Did = account
        .root
        .parse()
        .context("the signed-in account root is invalid")?;
    let access = account
        .access_remote
        .as_deref()
        .context("the account has no content endpoint; sign in again")?;
    let relay = account
        .revocation_relay
        .as_deref()
        .context("the account has no revocation relay; sign in again")?;

    let mut site_config = config.clone();
    site_config.require_account = false;
    let site = crate::site::TonkSite::open_with(&entry.site, site_config).await?;
    let subject = site.repository.did();
    preflight(&site, access).await?;

    // Authority first: the account root can only host what it can prove it
    // was given, and this is the one boundary allowed to mint that grant.
    let prefix = crate::site::adopt_account_root_prefix_for(
        &site.profile,
        site.operator.local(),
        &subject,
        &account_root,
    )
    .await?;
    crate::customer::provision_in(&site.profile, store, &subject, &prefix).await?;

    if crate::remote::upstream_remote(&site).await?.is_none() {
        crate::remote::add_with_revocation(
            &site,
            DEFAULT_REMOTE,
            access,
            Some(subject.clone()),
            Some(relay),
        )
        .await?;
        crate::remote::set_upstream(&site, DEFAULT_REMOTE).await?;
    }
    crate::site::record_founder_membership(&site).await?;
    crate::sync::push(&site).await?;

    let operator =
        crate::account_state::credential_operator_for_store(&site.profile, store).await?;
    if crate::account_state::open_account_branch_in(&site.profile, &operator, store)
        .await?
        .is_none()
    {
        bail!("the account repository is not ready to hold this space");
    }
    crate::account_state::retain_space_delegation_in(&site.profile, &operator, store, &prefix)
        .await?;
    crate::account_spots::record_site_pushed(name, &site, store).await?;

    if crate::inventory::role_for_site(&site).await? != SpaceRole::Owner {
        bail!("the space is not signed as owned by this device after linking");
    }
    store.set_space_account(name, Some(&account.root))?;
    Ok(LinkOutcome {
        subject: Some(subject.to_string()),
        name: name.to_owned(),
        site: site.root,
        account: account.root,
        already_linked: false,
    })
}

/// Refuse anything that is not genuinely local-only.
///
/// An upstream already pointing at this account's own content service is the
/// one exception: that is what a half-finished link leaves behind, and a
/// retry has to be able to get past it.
async fn preflight(site: &crate::site::TonkSite, access: &str) -> Result<()> {
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
    let meta = site
        .repository
        .branch(crate::remote::META_BRANCH)
        .open()
        .perform(&site.operator)
        .await
        .context("failed to inspect local-space metadata")?;
    let invitations: Vec<Invitation> = meta
        .query()
        .select(Query::<Invitation> {
            this: Term::var("this"),
            subject: Term::var("subject"),
            inviter: Term::var("inviter"),
            audience: Term::var("audience"),
        })
        .perform(&site.operator)
        .try_vec()
        .await?;
    if !invitations.is_empty() {
        bail!("a space with recorded shares cannot be linked to an account");
    }
    let memberships: Vec<Membership> = meta
        .query()
        .select(Query::<Membership> {
            this: Term::var("this"),
            subject: Term::var("subject"),
            member: Term::var("member"),
        })
        .perform(&site.operator)
        .try_vec()
        .await?;
    if memberships
        .iter()
        .any(|membership| membership.member.0 != site.profile.did().this())
    {
        bail!("a space with another durable member cannot be linked to an account");
    }
    let roles: Vec<MemberRole> = meta
        .query()
        .select(Query::<MemberRole> {
            this: Term::var("this"),
            role: Term::var("role"),
        })
        .perform(&site.operator)
        .try_vec()
        .await?;
    if memberships.iter().any(|membership| {
        let matching: Vec<_> = roles
            .iter()
            .filter(|role| role.this == membership.this)
            .collect();
        matching.len() != 1 || matching[0].role.0.to_string() != MemberRole::FOUNDER
    }) {
        bail!("only a genuinely local-only space can be linked to an account");
    }
    Ok(())
}
