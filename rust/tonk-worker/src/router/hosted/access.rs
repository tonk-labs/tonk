//! What an agent space reaches, read from the agent space itself.
//!
//! The approval page retains each grant (a chain `space -> ... -> agent
//! space`) in the agent space's `main` with
//! [`tonk_account::delegations::retain_space_delegation`], the same call
//! an account uses for `space -> account`, and records a directory entry
//! on the granted space's entity: its [`tonk_schema::SpaceName`] and its
//! [`tonk_schema::SpaceHomeAddress`]. There is no access-level fact:
//! access is what the retained chains prove.
//!
//! Two readers:
//!
//! - [`read_agent_access`] classifies the grants, for the agent space's
//!   own writer to answer the front with;
//! - [`adopt_agent_access`] points a writer peer's access branch at the
//!   agent space and pulls it, so the peer's operator finds `space -> ...
//!   -> agent space` the way a device finds what its account holds, and
//!   proves on through the `agent space -> session -> writer` chain it
//!   already holds.

use std::collections::BTreeMap;

use dialog_capability::access::TimeRange;
use dialog_query::{Output as _, Query, Term};
use dialog_remote_ucan::UcanAddress;
use dialog_repository::{Repository, SiteAddress};
use dialog_ucan::{Parameters, Scope};
use dialog_ucan_core::subject::Subject as UcanSubject;
use dialog_ucan_core::time::Timestamp;
use dialog_varsig::Did;
use serde::{Deserialize, Serialize};
use tonk_schema::prelude::DidExt as _;

use crate::TonkWorkerError;
use crate::worker::TonkState;

/// The branch of the agent space that holds its grants and directory.
const GRANTS_BRANCH: &str = "main";

/// What one space is open to through the agent space.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceAccess {
    /// Whether this is the agent space itself.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub agent: bool,
    /// Whether a grant in force reaches `/use/get/...`.
    #[serde(default)]
    pub read: bool,
    /// Whether a grant in force also reaches `/use/put/...`.
    #[serde(default)]
    pub write: bool,
    /// The earliest expiry among the grants in force, in Unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<u64>,
    /// Grants for this space that have expired or are not yet valid.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub lapsed: usize,
    /// The space's name in the agent space's directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Where the space syncs: the `home.address` a grant in force carries
    /// in its signed meta, as a space's grants carry it for every peer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// The CIDs of the delegations to the agent space that are in force,
    /// so a writer can tell a grant it has not pulled yet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grants: Vec<String>,
}

fn is_zero(count: &usize) -> bool {
    *count == 0
}

/// Every space the agent space reaches, keyed by its DID, the agent
/// space among them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentAccess {
    /// The spaces, by DID.
    pub spaces: BTreeMap<String, SpaceAccess>,
}

impl AgentAccess {
    /// What `space` is open to, if anything.
    pub fn get(&self, space: &Did) -> Option<&SpaceAccess> {
        self.spaces.get(&space.to_string())
    }
}

/// Classify the grants the agent space `agent` holds in its `main`, as
/// `tonk` (a peer with `agent` mounted) reads it now.
///
/// A grant is a delegation retained there whose audience is `agent` and
/// whose subject is another space; it counts when a chain from that
/// space through it verifies. A chain in force for `/use/get/...` makes
/// the space readable, one for `/use/put/...` writable too. A grant
/// outside its time bounds counts as lapsed, so a tool can name the space
/// whose access ran out. One whose chain does not verify counts as
/// nothing.
pub async fn read_agent_access(
    tonk: &TonkState,
    agent: &Did,
) -> Result<AgentAccess, TonkWorkerError> {
    let internal = |error: &dyn std::fmt::Display| TonkWorkerError::Internal(error.to_string());
    let session = tonk
        .reactor
        .repository(agent.as_ref())
        .branch(GRANTS_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| internal(&error))?;
    let branch = session.handle();
    let leaves = branch
        .delegations()
        .issued_to(agent.clone())
        .perform(&tonk.operator)
        .await
        .map_err(|error| internal(&error))?;
    let now = Timestamp::now().to_unix();
    let at_now = TimeRange {
        not_before: Some(now),
        expiration: Some(now + 1),
    };
    let mut spaces: BTreeMap<Did, SpaceAccess> = BTreeMap::new();
    for leaf in leaves {
        let chain = &leaf.0;
        let Some(subject) = chain.subject().cloned() else {
            // A subject-open delegation to the agent space names no space
            // to list.
            continue;
        };
        if &subject == agent {
            continue;
        }
        let Some(delegation) = chain.proofs().next() else {
            continue;
        };
        let command = delegation.command().clone();
        let in_window = chain.expiration().is_none_or(|at| at.to_unix() > now)
            && chain.not_before().is_none_or(|at| at.to_unix() <= now);
        let scope = Scope {
            subject: UcanSubject::Specific(subject.clone()),
            command: command.clone(),
            parameters: Parameters::default(),
        };
        let proof = branch
            .delegations()
            .prove(agent.clone(), scope)
            .during(at_now)
            .perform(&tonk.operator)
            .await;
        match proof {
            Ok(proof) if in_window => {
                let access = spaces.entry(subject).or_default();
                access.read |= reaches(&command.0, "get");
                access.write |= reaches(&command.0, "put");
                if let Some(at) = proof.duration.expiration {
                    access.expires = Some(access.expires.map_or(at, |held| held.min(at)));
                }
                access.grants.push(delegation.to_cid().to_string());
                // Signed by whoever granted the space, so the agent, which
                // can write its own space's directory, cannot redirect it.
                if access.address.is_none() {
                    access.address = tonk_invite::home_address(chain)
                        .ok()
                        .flatten()
                        .map(|address| address.to_string());
                }
            }
            _ if !in_window => spaces.entry(subject).or_default().lapsed += 1,
            // A chain that does not verify grants nothing.
            _ => {}
        }
    }
    let mut access = AgentAccess::default();
    for (space, mut entry) in spaces {
        // A space is writable only if it is readable too: a write pulls
        // first.
        entry.write &= entry.read;
        if !entry.read && entry.lapsed == 0 {
            continue;
        }
        entry.grants.sort_unstable();
        entry.grants.dedup();
        entry.name = directory_name(tonk, branch, &space).await;
        access.spaces.insert(space.to_string(), entry);
    }
    access.spaces.insert(
        agent.to_string(),
        SpaceAccess {
            agent: true,
            read: true,
            write: true,
            // The agent space names itself, as any space does; a directory
            // entry for it, should one exist, wins.
            name: match directory_name(tonk, branch, agent).await {
                Some(name) => Some(name),
                None => own_name(tonk, branch, agent).await,
            },
            ..SpaceAccess::default()
        },
    );
    Ok(access)
}

/// The name the agent space's directory gives `space`.
async fn directory_name(
    tonk: &TonkState,
    branch: &dialog_repository::Branch,
    space: &Did,
) -> Option<String> {
    let rows: Vec<tonk_schema::SpaceName> = branch
        .query()
        .select(Query::<tonk_schema::SpaceName> {
            this: Term::from(space.this()),
            name: Term::var("name"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .ok()?;
    rows.into_iter().next().map(|row| row.name.0)
}

/// The name `space` gives itself (`tonk/repository`).
async fn own_name(
    tonk: &TonkState,
    branch: &dialog_repository::Branch,
    space: &Did,
) -> Option<String> {
    let rows: Vec<tonk_schema::RepositoryName> = branch
        .query()
        .select(Query::<tonk_schema::RepositoryName> {
            this: Term::from(space.this()),
            name: Term::var("name"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .ok()?;
    rows.into_iter().next().map(|row| row.name.0)
}

/// Whether a grant whose leaf command is `command` covers some
/// `/use/{verb}/...` invocation.
pub fn reaches(command: &[String], verb: &str) -> bool {
    let wanted = ["use", verb];
    command
        .iter()
        .zip(wanted)
        .all(|(segment, wanted)| segment == wanted)
}

/// Point the access branch of `tonk`'s profile at the agent space
/// `agent`, served at `home`, and pull it.
///
/// [`tonk_account::delegations::adopt_account_upstream`] does the work,
/// as it does for a device and its account: the profile must already
/// hold `agent -> ... -> profile` (the writer chain), which authorizes
/// the pull. Called again, it pulls again.
pub async fn adopt_agent_access(
    tonk: &TonkState,
    agent: &Did,
    home: &url::Url,
) -> Result<(), TonkWorkerError> {
    let internal = |error: &dyn std::fmt::Display| TonkWorkerError::Internal(error.to_string());
    let address = SiteAddress::from(UcanAddress::new(home.as_str()));
    let remote = tonk_account::peer::connect(address, agent.clone(), &tonk.operator)
        .await
        .map_err(|error| internal(&error))?;
    let upstream = remote
        .branch(dialog_repository::ACCESS_BRANCH)
        .open()
        .perform(&tonk.operator)
        .await
        .map_err(|error| internal(&error))?;
    // Opened after connecting: connecting records the agent space's
    // service in the profile, which moves the branch's head, and a
    // handle opened before it would merge onto a stale head.
    let access = Repository::from(tonk.profile.did())
        .branch(tonk.active_branch.as_str())
        .open()
        .perform(&tonk.operator)
        .await
        .map_err(|error| internal(&error))?;
    tonk_account::delegations::adopt_account_upstream(&access, upstream, &tonk.operator)
        .await
        .map_err(|error| internal(&format!("pull the agent space's grants: {error}")))?;
    Ok(())
}

/// One granted space's directory entry in the agent space.
#[cfg(test)]
pub(crate) struct DirectoryEntry {
    pub space: Did,
    pub name: Option<String>,
    pub address: Option<String>,
}

/// What the approval does to the agent space `agent` as `tonk` holds it:
/// retain each of `chains` in its `main` with
/// [`tonk_account::delegations::retain_space_delegation`], as an account
/// retains `space -> account`, and record each entry of `directory` as
/// [`tonk_schema::SpaceName`] and [`tonk_schema::SpaceHomeAddress`] on
/// the space's entity. The caller pushes.
#[cfg(test)]
pub(crate) async fn retain_agent_grants(
    tonk: &TonkState,
    agent: &Did,
    chains: &[dialog_ucan_core::DelegationChain],
    directory: &[DirectoryEntry],
) -> Result<(), TonkWorkerError> {
    let internal = |error: &dyn std::fmt::Display| TonkWorkerError::Internal(error.to_string());
    let key = agent.to_string();
    let session = tonk
        .reactor
        .repository(&key)
        .branch(GRANTS_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| internal(&error))?;
    for chain in chains {
        tonk_account::delegations::retain_space_delegation(session.handle(), chain, &tonk.operator)
            .await
            .map_err(|error| internal(&error))?;
    }
    if directory.is_empty() {
        return Ok(());
    }
    let mut transaction = tonk
        .reactor
        .repository(&key)
        .branch(GRANTS_BRANCH)
        .transaction();
    for entry in directory {
        if let Some(name) = &entry.name {
            transaction = transaction.assert(tonk_schema::SpaceName::new(&entry.space, name));
        }
        if let Some(address) = &entry.address {
            transaction =
                transaction.assert(tonk_schema::SpaceHomeAddress::new(&entry.space, address));
        }
    }
    transaction
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(|error| internal(&error))?;
    Ok(())
}

/// `seconds` since the Unix epoch as an RFC 3339 UTC timestamp.
pub fn rfc3339(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    // Days to a civil date (Howard Hinnant's `civil_from_days`).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn it_formats_rfc3339() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_801_079_968), "2027-01-27T19:59:28Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
    }

    #[dialog_common::test]
    fn it_reads_verbs_off_a_command() {
        let command = |path: &str| -> Vec<String> {
            path.split('/')
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        };
        assert!(reaches(&command("/use/get/memory/cell"), "get"));
        assert!(!reaches(&command("/use/get/memory/cell"), "put"));
        assert!(
            reaches(&command("/"), "put"),
            "the root command reaches all"
        );
        assert!(!reaches(&command("/archive"), "get"));
    }
}
