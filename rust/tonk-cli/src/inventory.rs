//! Aggregate, offline inventory of every locally mounted space replica.
//!
//! Ownership is read, never recorded: the founder row of the roster the space
//! itself carries on its content branch names the owner, so a member device
//! knows the owner of a space it merely joined, and nothing beside the space
//! can drift out of step with the chains the access service validates.

use std::collections::BTreeSet;
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use dialog_query::{Output as _, Query, Term};
use serde::Serialize;
use tonk_schema::prelude::DidExt as _;
use tonk_schema::{MemberName, MemberRole, Membership};
use crate::listing::Listing;
use crate::site::SiteConfig;
use crate::space::SpaceStore;

/// Shortest DID abbreviation a listing starts from. The first four
/// characters of a `did:key` identifier are the shared ed25519 multibase
/// prefix, so anything shorter renders every row identically.
const ABBREVIATION: usize = 8;

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

impl SpaceRole {
    /// The name of this role, matching what `--json` serializes.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Owner => "owner",
            Self::Member => "member",
            Self::Unlisted => "unlisted",
            Self::Unknown => "unknown",
        }
    }

    /// How the role reads in the `ROLE` column.
    ///
    /// Only [`Self::Unlisted`] differs from its name: the listing already
    /// spells an absent `OWNER` as `-`, and "the roster has no row of ours"
    /// is the same absence in the neighbouring column. Everywhere else — a
    /// log line, an error, a future message — the role names itself.
    fn column(&self) -> &'static str {
        match self {
            Self::Unlisted => "-",
            other => other.as_str(),
        }
    }
}

impl std::fmt::Display for SpaceRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
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
    /// What was wrong with the rows that were readable but not sound —
    /// a row with contradictory role stamps, a role URI from no known
    /// vocabulary, more than one founder. Each row still appears in
    /// `members`; a roster is reported as far as it can be read.
    pub notes: Vec<String>,
}

impl Roster {
    /// Whether the space has been hosted at all.
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// The founder row, which names the owning account.
    ///
    /// A sound roster has at most one. When it has more the lowest DID
    /// wins — an arbitrary rule, but a fixed one, so the owner a listing
    /// prints and the owner a refusal names cannot disagree with each
    /// other or drift between runs on the query's ordering. The ambiguity
    /// itself is reported through [`Roster::notes`].
    pub fn founder(&self) -> Option<&RosterMember> {
        self.members
            .iter()
            .filter(|member| member.is_founder())
            .min_by(|left, right| left.did.cmp(&right.did))
    }

    /// The row `did` holds, if any.
    pub fn row_for(&self, did: &str) -> Option<&RosterMember> {
        self.members.iter().find(|member| member.did == did)
    }
}

/// Version-two local-replica row.
///
/// Version two is where ownership stopped being a registry tag: `account`
/// and `access` are gone, and `owner` / `ownerName` / `ownerIsYou` — read
/// from the space's own roster — take their place. A reader written against
/// version one cannot be fed this, so the number moves with the shape.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSpaceInventoryRowV2 {
    /// Schema version, exactly two.
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
    pub rows: Vec<LocalSpaceInventoryRowV2>,
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
    match name.map(sanitize).filter(|name| !name.is_empty()) {
        Some(name) => format!("{name} ({abbreviated})"),
        None => abbreviated,
    }
}

/// Make a display name safe to print on one line of a table.
///
/// A member's name is written by whoever holds that roster row, reaches this
/// device over sync, and is the only field in the listing this device does
/// not author. Control characters are dropped rather than escaped: a newline
/// would break the row apart and an ANSI escape would repaint the terminal,
/// and neither is anything a name needs.
fn sanitize(name: &str) -> String {
    name.chars()
        .filter(|character| !character.is_control())
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Render the listing exactly as `tonk space list` prints it.
///
/// Lives here rather than in the binary so the text a person actually reads
/// can be pinned by a test that builds real replicas.
pub fn render(rows: &[LocalSpaceInventoryRowV2]) -> String {
    let length = abbreviation_length(
        rows.iter()
            .map(|row| row.subject.as_str())
            .chain(rows.iter().filter_map(|row| row.owner.as_deref())),
    );
    let mut listing = Listing::new(
        &["NAME", "OWNER", "ROLE"],
        "no spaces registered; create one with `tonk space new <name>`",
    );
    for row in rows {
        listing.push([
            format!("{} ({})", row.name, abbreviate(&row.subject, length)),
            match &row.owner {
                None => "-".to_owned(),
                Some(owner) if row.owner_is_you => describe(owner, Some("you"), length),
                Some(owner) => describe(owner, row.owner_name.as_deref(), length),
            },
            row.role.column().to_owned(),
        ]);
    }
    listing.render()
}

/// Inspect the registry and every replica it names without remote I/O.
pub async fn list_local(store: &SpaceStore, config: &SiteConfig) -> Result<LocalSpaceInventory> {
    let registry = store.load()?;
    // Resolved from the first replica that opens and reused for the rest:
    // the account and the local root belong to the installation, not to any
    // one space, and a listing must not answer "who are we" per row.
    let mut identity = None;
    let mut report = LocalSpaceInventory::default();
    for (name, entry) in &registry.spaces {
        match inspect_replica(config, name, entry, &mut identity).await {
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
    entry: &crate::space::SpaceEntry,
    identity: &mut Option<crate::site::Identity>,
) -> Result<(LocalSpaceInventoryRowV2, Option<String>)> {
    let mut config = config.clone();
    config.require_account = false;
    let site = crate::site::TonkSite::open_with(&entry.site, config)
        .await
        .with_context(|| format!("could not open {}", entry.site.display()))?;
    // Every replica opens the same profile, so the first one to get this far
    // answers for all of them.
    let identity = match identity {
        Some(identity) => &*identity,
        slot => slot.insert(crate::site::Identity::of(&site).await?),
    };
    let subject = site.repository.did().to_string();
    let (roster, note) = match read_roster(&site).await {
        // A roster that read but did not add up still describes the space;
        // what was wrong with it travels as the row's diagnostic.
        Ok(roster) => {
            let note = (!roster.notes.is_empty()).then(|| roster.notes.join("; "));
            (Some(roster), note)
        }
        Err(error) => (None, Some(format!("{error:#}"))),
    };
    let founder = roster
        .as_ref()
        .and_then(Roster::founder)
        .map(|member| (member.did.clone(), member.name.clone()));
    let role = match &roster {
        None => SpaceRole::Unknown,
        Some(roster) => classify(roster, identity),
    };
    Ok((
        LocalSpaceInventoryRowV2 {
            version: 2,
            name: name.to_owned(),
            subject,
            // The account slot, not every identity this device holds: the
            // column answers "is this the account I am signed in as", which
            // is what flips when somebody switches accounts. Whether this
            // installation can act on the space is `role`'s question.
            owner_is_you: match (&founder, identity.account()) {
                (Some((owner, _)), Some(account)) => owner == account,
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

/// Which roster row this installation can claim, tried in
/// [`Identity`](crate::site::Identity)'s order of specificity.
///
/// The identity is passed rather than resolved here, so a listing answers
/// "who are we" once for the whole run instead of once per replica, and
/// cannot answer it two different ways within one report.
fn classify(roster: &Roster, identity: &crate::site::Identity) -> SpaceRole {
    if roster.is_empty() {
        return SpaceRole::Local;
    }
    let Some(row) = identity.dids().find_map(|did| roster.row_for(did)) else {
        return SpaceRole::Unlisted;
    };
    match row.role.as_deref() {
        Some(MemberRole::FOUNDER) => SpaceRole::Owner,
        Some(MemberRole::MEMBER) => SpaceRole::Member,
        _ => SpaceRole::Unknown,
    }
}

/// Classify one replica from the roster the space itself carries.
pub async fn role_for_site(site: &crate::site::TonkSite) -> Result<SpaceRole> {
    let identity = crate::site::Identity::of(site).await?;
    Ok(classify(&read_roster(site).await?, &identity))
}

/// Read the roster from the replica's content branch.
///
/// The content branch, not the meta branch: only upstreamed branches sync, so
/// a roster on `meta` would never converge across the devices and members it
/// exists to describe.
///
/// Fails only when the branch or a query does. An individual row that does
/// not add up degrades to a row with no role and a note in
/// [`Roster::notes`]: the roster is shared, written by every member, and one
/// member's bad row must not cost this device the owner of its own space.
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
    let mut roster = Roster {
        members: Vec::with_capacity(memberships.len()),
        notes: Vec::new(),
    };
    for membership in memberships {
        let did = membership.member.0.to_string();
        let stamped: Vec<String> = roles
            .iter()
            .filter(|role| role.this == membership.this)
            .map(|role| role.role.0.to_string())
            .collect();
        // A role this device cannot act on is the same as none: the row
        // still names a member, and `Unlisted` / `Unknown` is a truthful
        // answer where a guess would not be.
        let role = match stamped.split_first() {
            None => None,
            Some((role, rest)) if rest.iter().all(|other| other == role) => {
                if matches!(role.as_str(), MemberRole::FOUNDER | MemberRole::MEMBER) {
                    Some(role.clone())
                } else {
                    roster
                        .notes
                        .push(format!("{did} has unknown role '{role}'"));
                    None
                }
            }
            Some(_) => {
                roster
                    .notes
                    .push(format!("{did} has conflicting signed roles"));
                None
            }
        };
        roster.members.push(RosterMember {
            did,
            name: names
                .iter()
                .find(|name| name.this == membership.this)
                .map(|name| name.name.0.clone()),
            role,
        });
    }
    let founders = roster
        .members
        .iter()
        .filter(|member| member.is_founder())
        .count();
    if founders > 1 {
        roster.notes.push(format!(
            "the roster names {founders} founders; \
             the lowest DID is reported as the owner"
        ));
    }
    Ok(roster)
}
