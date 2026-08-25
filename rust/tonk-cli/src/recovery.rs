//! Whether a space's data survives being deleted.
//!
//! `tonk space rm` destroys a site directory, and how bad that is
//! depends entirely on where else the facts exist. A space the account
//! directory lists can be pulled down again; a space that only ever
//! pushed to a remote can be recovered from that remote if the
//! operator still holds authority over it; a space with no upstream at
//! all exists nowhere else, and deleting it is final.
//!
//! The registry cannot answer this — [`crate::space::SpaceEntry`] is a
//! path and nothing more — so the answer is read out of the site
//! itself: the `main` branch's upstream, plus the account branch's
//! local copy of the directory. Both are local reads. Deliberately no
//! network round trip: a confirmation prompt that hangs on an
//! unreachable remote is a worse failure than one that reports what
//! this device knows.

use std::path::Path;

use crate::site::{SiteConfig, TonkSite};

/// Where a space's data exists besides this directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recovery {
    /// Listed in the linked account's directory, and pullable again
    /// by subject.
    Account {
        /// Repository subject DID, the argument `tonk account spaces
        /// pull` takes.
        subject: String,
    },
    /// Pushed to a remote, but not listed in the account directory.
    /// Recoverable only by someone who can still reach that remote
    /// with authority over the repository — an invite, or another
    /// device that holds the delegation.
    Remote {
        /// Local name of the tracked remote.
        name: String,
        /// Its endpoint URL.
        endpoint: String,
    },
    /// No upstream: this directory is the only copy.
    LocalOnly,
    /// The site would not open, so nothing can be claimed either
    /// way. Reported rather than assumed, because guessing
    /// "recoverable" invites data loss and guessing "local-only"
    /// cries wolf.
    Unknown {
        /// Why the inspection failed, for the operator to judge.
        detail: String,
    },
}

impl Recovery {
    /// True only when this device can point at a copy it knows how
    /// to fetch back.
    pub fn is_recoverable(&self) -> bool {
        matches!(self, Recovery::Account { .. })
    }

    /// What deleting `name` costs, phrased for a confirmation
    /// prompt.
    pub fn consequence(&self, name: &str) -> String {
        match self {
            Recovery::Account { subject } => format!(
                "'{name}' is listed in your account directory ({subject}).\n\
                 You can pull it down again after deleting it."
            ),
            Recovery::Remote {
                name: remote,
                endpoint,
            } => format!(
                "'{name}' pushes to '{remote}' ({endpoint}) but is not listed in\n\
                 your account directory. Deleting it here keeps whatever the remote\n\
                 already holds, but this device loses its access to it."
            ),
            Recovery::LocalOnly => format!(
                "'{name}' is local-only: it has no upstream and no account listing,\n\
                 so this is the only copy and deleting it cannot be undone."
            ),
            Recovery::Unknown { detail } => format!(
                "'{name}' could not be inspected, so whether it exists anywhere else\n\
                 is unknown ({detail}). Treat this as unrecoverable."
            ),
        }
    }

    /// How to get the data back, when this device knows a way.
    pub fn restore_hint(&self) -> Option<String> {
        match self {
            Recovery::Account { subject } => Some(format!(
                "restore it later with `tonk account spaces pull {subject}`"
            )),
            Recovery::Remote { .. } | Recovery::LocalOnly | Recovery::Unknown { .. } => None,
        }
    }
}

/// Inspect the site at `path` and report where else its data lives.
///
/// Never fails: every error becomes [`Recovery::Unknown`], because
/// the caller's next move is to show a human a warning either way,
/// and a probe that can abort the command it is trying to make safe
/// has the priorities backwards.
pub async fn inspect(path: &Path, config: SiteConfig) -> Recovery {
    let site = match TonkSite::open_with(path, config).await {
        Ok(site) => site,
        Err(error) => {
            return Recovery::Unknown {
                detail: format!("{error}"),
            };
        }
    };

    let upstream = match crate::remote::upstream_remote(&site).await {
        Ok(Some(upstream)) => upstream,
        // No upstream is a definite answer, not a failed one: a
        // branch that tracks nothing has pushed nowhere.
        Ok(None) => return Recovery::LocalOnly,
        Err(error) => {
            return Recovery::Unknown {
                detail: format!("{error}"),
            };
        }
    };

    // A directory mount record is only ever written for a site that
    // has an upstream, so this is checked second and only here.
    match crate::account_spaces::directory_lists(&site).await {
        Ok(true) => {
            return Recovery::Account {
                subject: site.repository.did().to_string(),
            };
        }
        Ok(false) => {}
        Err(error) => {
            return Recovery::Unknown {
                detail: format!("{error:#}"),
            };
        }
    }

    let endpoint = match crate::remote::find(&site, &upstream).await {
        Ok(Some(record)) => record.endpoint,
        // The upstream names a remote the meta branch no longer
        // describes. The push target is real enough to mention;
        // only its address is missing.
        Ok(None) => "address unknown".to_owned(),
        Err(error) => {
            return Recovery::Unknown {
                detail: format!("{error}"),
            };
        }
    };
    Recovery::Remote {
        name: upstream,
        endpoint,
    }
}
