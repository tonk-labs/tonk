//! What an agent's space opens to the agent: each space it reaches and how,
//! as the agent space's kernel answers it and the hosted agent reads it.

use std::collections::BTreeMap;

use dialog_varsig::Did;
use serde::{Deserialize, Serialize};

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

/// Whether a grant whose leaf command is `command` covers some
/// `/use/{verb}/...` invocation.
pub fn reaches(command: &[String], verb: &str) -> bool {
    let wanted = ["use", verb];
    command
        .iter()
        .zip(wanted)
        .all(|(segment, wanted)| segment == wanted)
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
