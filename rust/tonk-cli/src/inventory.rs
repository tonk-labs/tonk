//! Aggregate, offline inventory of every locally mounted space replica.

use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use dialog_query::{Output as _, Query, Term};
use serde::Serialize;
use tonk_schema::prelude::DidExt as _;
use tonk_schema::{MemberRole, Membership};

use crate::site::SiteConfig;
use crate::spot::SpotStore;

/// One replica's relationship to the account that owns it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SpaceRole {
    /// Account-independent local-only replica.
    Local,
    /// Replica whose account founded the space.
    Owner,
    /// Replica whose account joined through a share.
    Member,
    /// Account-owned replica whose signed membership could not be read.
    Unknown,
}

impl std::fmt::Display for SpaceRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Local => "local",
            Self::Owner => "owner",
            Self::Member => "member",
            Self::Unknown => "unknown",
        })
    }
}

/// Version-one local-replica row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSpaceInventoryRowV1 {
    /// Schema version, exactly one.
    pub version: u8,
    /// Registered space name.
    pub name: String,
    /// Repository subject DID.
    pub subject: String,
    /// Root DID of the owning account, absent for a local-only space.
    pub account: Option<String>,
    /// Local, owner, or member relationship.
    pub role: SpaceRole,
    /// Local site directory.
    pub site: PathBuf,
    /// Whether the signed-in account may use this replica.
    pub access: bool,
    /// Always true in this local-replica inventory.
    pub local: bool,
}

/// Rows plus non-fatal diagnostics for unreadable or unclassifiable replicas.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LocalSpaceInventory {
    /// Every valid local replica.
    pub rows: Vec<LocalSpaceInventoryRowV1>,
    /// One space-qualified message per skipped replica.
    pub diagnostics: Vec<String>,
}

/// Render the listing exactly as `tonk space list` prints it.
///
/// Lives here rather than in the binary so the text a person actually reads
/// can be pinned by a test that builds real replicas.
pub fn render(rows: &[LocalSpaceInventoryRowV1]) -> String {
    if rows.is_empty() {
        return "(no spaces registered; create one with `tonk space new <name>`)".to_owned();
    }
    let mut out = String::from("NAME\tSUBJECT\tACCOUNT\tROLE\tACCESS");
    for row in rows {
        out.push_str(&format!(
            "\n{}\t{}\t{}\t{}\t{}",
            row.name,
            row.subject,
            row.account.as_deref().unwrap_or("-"),
            row.role,
            if row.access { "yes" } else { "no" },
        ));
    }
    if rows.iter().any(|row| !row.access) {
        out.push_str(
            "\n\nspaces marked no belong to another account; sign back into it, \
             or ask its owner for an invite",
        );
    }
    out
}

/// Inspect the registry and every replica it names without remote I/O.
pub async fn list_local(store: &SpotStore, config: &SiteConfig) -> Result<LocalSpaceInventory> {
    let registry = store.load()?;
    let active = registry
        .account
        .as_ref()
        .map(|account| account.root.clone());
    let mut report = LocalSpaceInventory::default();
    for (name, entry) in &registry.spots {
        match inspect_replica(config, name, entry).await {
            Ok((mut row, note)) => {
                row.access = match (&row.account, &active) {
                    (Some(owner), Some(active)) => owner == active,
                    _ => true,
                };
                report.rows.push(row);
                if let Some(note) = note {
                    report.diagnostics.push(format!("{name}: {note}"));
                }
            }
            Err(error) => report.diagnostics.push(format!("{name}: {error:#}")),
        }
    }
    report
        .rows
        .sort_by(|left, right| (&left.name, &left.subject).cmp(&(&right.name, &right.subject)));
    Ok(report)
}

/// Returns the row, plus a diagnostic when the replica is listed with an
/// unresolved role. A space whose role cannot be read still belongs on
/// screen: it is registered, it occupies its name, and hiding it would make
/// the listing disagree with every other command.
async fn inspect_replica(
    config: &SiteConfig,
    name: &str,
    entry: &crate::spot::SpotEntry,
) -> Result<(LocalSpaceInventoryRowV1, Option<String>)> {
    let mut config = config.clone();
    config.require_account = false;
    let site = crate::site::TonkSite::open_with(&entry.site, config)
        .await
        .with_context(|| format!("could not open {}", entry.site.display()))?;
    let subject = site.repository.did().to_string();
    // The registry says which account a replica belongs to; the repository's
    // own signed membership says whether that account founded it or joined
    // it. Neither is guessed from the other.
    let (role, note) = match entry.account {
        None => (SpaceRole::Local, None),
        Some(_) => match role_for_site(&site).await {
            Ok(role) => (role, None),
            Err(error) => (SpaceRole::Unknown, Some(format!("{error:#}"))),
        },
    };
    Ok((
        LocalSpaceInventoryRowV1 {
            version: 1,
            name: name.to_owned(),
            subject,
            account: entry.account.clone(),
            role,
            site: site.root,
            access: true,
            local: true,
        },
        note,
    ))
}

/// Classify one replica from the signed membership the repository itself
/// carries, rather than from anything the registry claims.
pub async fn role_for_site(site: &crate::site::TonkSite) -> Result<SpaceRole> {
    let meta = site
        .repository
        .branch(crate::remote::META_BRANCH)
        .open()
        .perform(&site.operator)
        .await
        .context("could not open membership facts")?;
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
    let membership = memberships
        .into_iter()
        .find(|membership| {
            membership.subject.0 == site.repository.did().this()
                && membership.member.0 == site.profile.did().this()
        })
        .context("this device has no signed membership in the space")?;
    let roles: Vec<MemberRole> = meta
        .query()
        .select(Query::<MemberRole> {
            this: Term::var("this"),
            role: Term::var("role"),
        })
        .perform(&site.operator)
        .try_vec()
        .await?;
    let mut matching = roles
        .into_iter()
        .filter(|role| role.this == membership.this)
        .map(|role| role.role.0.to_string());
    let role = matching.next().context("membership has no signed role")?;
    if matching.any(|candidate| candidate != role) {
        bail!("membership has conflicting signed roles");
    }
    match role.as_str() {
        MemberRole::FOUNDER => Ok(SpaceRole::Owner),
        MemberRole::MEMBER => Ok(SpaceRole::Member),
        _ => bail!("membership has unknown role '{role}'"),
    }
}
