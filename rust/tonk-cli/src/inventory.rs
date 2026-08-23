//! Aggregate, offline inventory of every locally mounted space replica.
//!
//! Ownership is read, never recorded: the founder row of the roster the space
//! itself carries on its content branch names the owner, so a member device
//! knows the owner of a space it merely joined, and nothing beside the space
//! can drift out of step with the chains the access service validates.

use std::collections::BTreeSet;
use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use dialog_query::{Output as _, Query, Term};
use serde::Serialize;
use tonk_schema::prelude::DidExt as _;
use tonk_schema::{MemberName, MemberRole, Membership};

use crate::site::SiteConfig;
use crate::spot::SpotStore;

/// Shortest DID abbreviation a listing starts from. The first four
/// characters of a `did:key` identifier are the shared ed25519 multibase
/// prefix, so anything shorter renders every row identically.
const ABBREVIATION: usize = 8;

/// The gutter between rendered columns.
const GUTTER: usize = 3;

/// One replica's relationship to the roster its space carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SpaceRole {
    /// The space carries no roster at all: nobody has hosted it yet.
    Local,
    /// The roster names this installation as the space's founder.
    Owner,
    /// The roster names this installation as a member.
    Member,
    /// The space carries a roster, and no row in it is ours.
    Unlisted,
    /// The replica or its roster could not be read.
    Unknown,
}

impl std::fmt::Display for SpaceRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Local => "local",
            Self::Owner => "owner",
            Self::Member => "member",
            Self::Unlisted => "-",
            Self::Unknown => "unknown",
        })
    }
}

/// One row of the roster a space carries on its content branch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RosterMember {
    /// The member's DID — an account root for a linked profile, a device
    /// profile otherwise.
    pub did: String,
    /// The member's self-chosen display name, when one was written.
    pub name: Option<String>,
    /// The role URI stamped on the membership, when one was written.
    pub role: Option<String>,
}

impl RosterMember {
    /// Whether this row carries the founder stamp.
    pub fn is_founder(&self) -> bool {
        self.role.as_deref() == Some(MemberRole::FOUNDER)
    }
}

/// The roster a space carries, in the order the query returned it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Roster {
    /// Every membership row scoped to this repository.
    pub members: Vec<RosterMember>,
}

impl Roster {
    /// Whether the space has been hosted at all.
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// The founder row, which names the owning account.
    pub fn founder(&self) -> Option<&RosterMember> {
        self.members.iter().find(|member| member.is_founder())
    }

    /// The row `did` holds, if any.
    pub fn row_for(&self, did: &str) -> Option<&RosterMember> {
        self.members.iter().find(|member| member.did == did)
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
    /// Account root of the space's founder, read from its roster. Absent
    /// until the space is linked or hosted.
    pub owner: Option<String>,
    /// The founder's display name, when the roster carries one.
    pub owner_name: Option<String>,
    /// Whether the founder is the account signed in here.
    pub owner_is_you: bool,
    /// This installation's own role in the roster.
    pub role: SpaceRole,
    /// Local site directory.
    pub site: PathBuf,
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

/// The method-specific identifier of a DID — everything after `did:<method>:`.
///
/// Anything that does not parse is returned whole rather than truncated at a
/// boundary that might not exist.
fn identifier(did: &str) -> &str {
    did.strip_prefix("did:")
        .and_then(|rest| rest.split_once(':'))
        .map_or(did, |(_, identifier)| identifier)
}

/// A true prefix of `did`'s identifier, `length` characters long.
pub fn abbreviate(did: &str, length: usize) -> String {
    identifier(did).chars().take(length).collect()
}

/// The shortest abbreviation that tells every DID in `dids` apart.
///
/// Git's short-hash discipline: eight characters unless this particular piece
/// of output contains an ambiguous prefix, and then as many as it takes. One
/// length is used throughout, so two abbreviations that render the same
/// always name the same identifier.
pub fn abbreviation_length<'a>(dids: impl IntoIterator<Item = &'a str>) -> usize {
    let distinct: BTreeSet<&str> = dids.into_iter().collect();
    let longest = distinct
        .iter()
        .map(|did| identifier(did).chars().count())
        .max()
        .unwrap_or(ABBREVIATION);
    let mut length = ABBREVIATION;
    while length < longest {
        let abbreviated: BTreeSet<String> =
            distinct.iter().map(|did| abbreviate(did, length)).collect();
        if abbreviated.len() == distinct.len() {
            break;
        }
        length += 1;
    }
    length
}

/// A human name paired with an abbreviation of its stable identifier — git's
/// `Name <email>` discipline. Names orient; only the identifier decides.
pub fn describe(did: &str, name: Option<&str>, length: usize) -> String {
    let abbreviated = abbreviate(did, length);
    match name {
        Some(name) => format!("{name} ({abbreviated})"),
        None => abbreviated,
    }
}

/// Render the listing exactly as `tonk space list` prints it.
///
/// Lives here rather than in the binary so the text a person actually reads
/// can be pinned by a test that builds real replicas.
pub fn render(rows: &[LocalSpaceInventoryRowV1]) -> String {
    if rows.is_empty() {
        return "(no spaces registered; create one with `tonk space new <name>`)".to_owned();
    }
    let length = abbreviation_length(
        rows.iter()
            .map(|row| row.subject.as_str())
            .chain(rows.iter().filter_map(|row| row.owner.as_deref())),
    );
    let cells: Vec<[String; 3]> = rows
        .iter()
        .map(|row| {
            [
                format!("{} ({})", row.name, abbreviate(&row.subject, length)),
                match &row.owner {
                    None => "-".to_owned(),
                    Some(owner) if row.owner_is_you => describe(owner, Some("you"), length),
                    Some(owner) => describe(owner, row.owner_name.as_deref(), length),
                },
                row.role.to_string(),
            ]
        })
        .collect();
    let headers = ["NAME", "OWNER", "ROLE"];
    let widths: Vec<usize> = (0..headers.len())
        .map(|column| {
            cells
                .iter()
                .map(|row| row[column].chars().count())
                .chain(std::iter::once(headers[column].len()))
                .max()
                .unwrap_or_default()
        })
        .collect();
    let mut out = String::new();
    for (index, header) in headers.iter().enumerate() {
        push_cell(&mut out, header, widths[index], index + 1 == headers.len());
    }
    for row in &cells {
        out.push('\n');
        for (index, cell) in row.iter().enumerate() {
            push_cell(&mut out, cell, widths[index], index + 1 == row.len());
        }
    }
    out
}

/// Append one cell, padded to `width` plus the gutter unless it ends the row.
fn push_cell(out: &mut String, cell: &str, width: usize, last: bool) {
    out.push_str(cell);
    if last {
        return;
    }
    let pad = width - cell.chars().count() + GUTTER;
    out.extend(std::iter::repeat_n(' ', pad));
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
        match inspect_replica(config, name, entry, active.as_deref()).await {
            Ok((row, note)) => {
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
/// unresolved role. A space whose roster cannot be read still belongs on
/// screen: it is registered, it occupies its name, and hiding it would make
/// the listing disagree with every other command.
async fn inspect_replica(
    config: &SiteConfig,
    name: &str,
    entry: &crate::spot::SpotEntry,
    active: Option<&str>,
) -> Result<(LocalSpaceInventoryRowV1, Option<String>)> {
    let mut config = config.clone();
    config.require_account = false;
    let site = crate::site::TonkSite::open_with(&entry.site, config)
        .await
        .with_context(|| format!("could not open {}", entry.site.display()))?;
    let subject = site.repository.did().to_string();
    let (roster, note) = match read_roster(&site).await {
        Ok(roster) => (Some(roster), None),
        Err(error) => (None, Some(format!("{error:#}"))),
    };
    let founder = roster
        .as_ref()
        .and_then(Roster::founder)
        .map(|member| (member.did.clone(), member.name.clone()));
    let role = match &roster {
        None => SpaceRole::Unknown,
        Some(roster) => classify(roster, &site)?,
    };
    Ok((
        LocalSpaceInventoryRowV1 {
            version: 1,
            name: name.to_owned(),
            subject,
            owner_is_you: match (&founder, active) {
                (Some((owner, _)), Some(active)) => owner == active,
                _ => false,
            },
            owner: founder.as_ref().map(|(did, _)| did.clone()),
            owner_name: founder.and_then(|(_, name)| name),
            role,
            site: site.root,
            local: true,
        },
        note,
    ))
}

/// Which roster row this installation can claim: the signed-in account root
/// first, the device profile second.
fn classify(roster: &Roster, site: &crate::site::TonkSite) -> Result<SpaceRole> {
    if roster.is_empty() {
        return Ok(SpaceRole::Local);
    }
    let account = crate::site::member_did(site)?.to_string();
    let profile = site.profile.did().to_string();
    let Some(row) = roster
        .row_for(&account)
        .or_else(|| roster.row_for(&profile))
    else {
        return Ok(SpaceRole::Unlisted);
    };
    Ok(match row.role.as_deref() {
        Some(MemberRole::FOUNDER) => SpaceRole::Owner,
        Some(MemberRole::MEMBER) => SpaceRole::Member,
        _ => SpaceRole::Unknown,
    })
}

/// Classify one replica from the roster the space itself carries.
pub async fn role_for_site(site: &crate::site::TonkSite) -> Result<SpaceRole> {
    classify(&read_roster(site).await?, site)
}

/// Read the roster from the replica's content branch.
///
/// The content branch, not the meta branch: only upstreamed branches sync, so
/// a roster on `meta` would never converge across the devices and members it
/// exists to describe.
pub async fn read_roster(site: &crate::site::TonkSite) -> Result<Roster> {
    let session = site
        .branch()
        .await
        .context("could not open the roster branch")?;
    let branch = session.handle();
    let memberships: Vec<Membership> = branch
        .query()
        .select(Query::<Membership> {
            this: Term::var("this"),
            subject: Term::var("subject"),
            member: Term::var("member"),
        })
        .perform(&site.operator)
        .try_vec()
        .await
        .context("could not read membership facts")?;
    let subject = site.repository.did().this();
    let memberships: Vec<Membership> = memberships
        .into_iter()
        .filter(|membership| membership.subject.0 == subject)
        .collect();
    if memberships.is_empty() {
        return Ok(Roster::default());
    }
    let roles: Vec<MemberRole> = branch
        .query()
        .select(Query::<MemberRole> {
            this: Term::var("this"),
            role: Term::var("role"),
        })
        .perform(&site.operator)
        .try_vec()
        .await
        .context("could not read membership roles")?;
    let names: Vec<MemberName> = branch
        .query()
        .select(Query::<MemberName> {
            this: Term::var("this"),
            name: Term::var("name"),
        })
        .perform(&site.operator)
        .try_vec()
        .await
        .context("could not read membership names")?;
    let mut members = Vec::with_capacity(memberships.len());
    for membership in memberships {
        let mut matching = roles
            .iter()
            .filter(|role| role.this == membership.this)
            .map(|role| role.role.0.to_string());
        let role = matching.next();
        if let Some(role) = &role
            && matching.any(|candidate| &candidate != role)
        {
            bail!("membership has conflicting signed roles");
        }
        members.push(RosterMember {
            did: membership.member.0.to_string(),
            name: names
                .iter()
                .find(|name| name.this == membership.this)
                .map(|name| name.name.0.clone()),
            role,
        });
    }
    Ok(Roster { members })
}
