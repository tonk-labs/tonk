//! The `branch@repo` location grammar shared by the `with` and
//! `allow` routing attributes.
//!
//! A **location** names the context a subtree operates with:
//!
//! - `main@did:key:zAlice` — the `main` branch of Alice's repository.
//! - `did:key:zAlice` — a bare repo means its default branch.
//! - `main@profile:tonk` — a `profile:<name>` repo token names the
//!   profile-as-repository endpoint (`/api/profile/branch/…`), not a
//!   named repository. The name (`tonk`, the profile the worker
//!   opens) is carried for forward compatibility; today the profile
//!   endpoint is singular. (Future: address the profile by its
//!   `did:key` like any repository, retiring the prefix.)
//!
//! An **allow list** names the set of locations a site permits its
//! descendants to reach, as space-separated tokens mirroring the
//! iframe `sandbox` attribute:
//!
//! - `*` — reach anything (the privileged case).
//! - explicit locations (`main@did:key:zBob …`). The sealed case is
//!   the embedder repeating the site's own `with` — the same binding
//!   wired into both attributes, no `self` sentinel to resolve.
//!
//! Both parse at connect time; a malformed attribute is a visible
//! mount error, never a silent deny at query time.

use std::fmt;
use std::str::FromStr;

/// The branch a bare-repo location resolves to, matching the URL
/// builder's default (`url.rs`).
const DEFAULT_BRANCH: &str = "main";

/// The repo-token prefix naming the profile-as-repository endpoint.
const PROFILE_PREFIX: &str = "profile:";

/// A repository reference: a named repository (a `did:key` or plain
/// name) or the profile endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Repo {
    /// The profile-as-repository endpoint (`/api/profile/branch/…`),
    /// with the profile's name (`profile:<name>`).
    Profile(String),
    /// A named repository (`/api/repository/{name}/branch/…`).
    Named(String),
}

/// A parsed `branch@repo` location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    /// Which repository.
    pub repo: Repo,
    /// Which branch; `None` means the repository's default branch.
    pub branch: Option<String>,
}

impl Location {
    /// The repository name for the URL builder — `None` in profile
    /// mode (the profile endpoint has no repo segment).
    pub fn space(&self) -> Option<&str> {
        match &self.repo {
            Repo::Profile(_) => None,
            Repo::Named(name) => Some(name),
        }
    }

    /// The branch name, if explicitly given.
    pub fn branch(&self) -> Option<&str> {
        self.branch.as_deref()
    }

    /// Whether this location targets the profile endpoint.
    pub fn profile(&self) -> bool {
        matches!(self.repo, Repo::Profile(_))
    }

    /// The branch this location resolves to, with the default
    /// filled in — the normalization equality-of-reach uses.
    pub fn effective_branch(&self) -> &str {
        self.branch.as_deref().unwrap_or(DEFAULT_BRANCH)
    }

    /// Whether `self` and `other` name the same reach: same repo,
    /// same effective branch (`did:key:zA` == `main@did:key:zA`).
    pub fn same_reach(&self, other: &Location) -> bool {
        self.repo == other.repo && self.effective_branch() == other.effective_branch()
    }
}

impl FromStr for Location {
    type Err = ParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let token = input.trim();
        if token.is_empty() {
            return Err(ParseError::EmptyLocation);
        }
        let (branch, repo) = match token.split_once('@') {
            Some((branch, repo)) => {
                if branch.is_empty() {
                    return Err(ParseError::EmptyBranch(token.to_owned()));
                }
                if repo.is_empty() {
                    return Err(ParseError::EmptyRepo(token.to_owned()));
                }
                (Some(branch.to_owned()), repo)
            }
            None => (None, token),
        };
        let repo = if let Some(name) = repo.strip_prefix(PROFILE_PREFIX) {
            if name.is_empty() {
                return Err(ParseError::ProfileNeedsName(token.to_owned()));
            }
            Repo::Profile(name.to_owned())
        } else if repo == "profile" {
            // A bare `profile` is almost certainly a mistyped profile
            // token, not a repository named "profile" — reject it
            // rather than routing to `/api/repository/profile`.
            return Err(ParseError::ProfileNeedsName(token.to_owned()));
        } else {
            Repo::Named(repo.to_owned())
        };
        Ok(Location { repo, branch })
    }
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(branch) = &self.branch {
            write!(f, "{branch}@")?;
        }
        match &self.repo {
            Repo::Profile(name) => write!(f, "{PROFILE_PREFIX}{name}"),
            Repo::Named(name) => write!(f, "{name}"),
        }
    }
}

/// A parsed `allow` list: the set of locations a site permits its
/// descendants to reach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Allow {
    /// `*` — reach anything.
    Any,
    /// An explicit set of locations.
    Only(Vec<Location>),
}

impl Allow {
    /// Whether a descendant's `requested` location is permitted.
    pub fn permits(&self, requested: &Location) -> bool {
        match self {
            Allow::Any => true,
            Allow::Only(locations) => locations
                .iter()
                .any(|location| location.same_reach(requested)),
        }
    }

    /// An allow list of exactly one location — the sealed shape a
    /// pinned portal grants (reach exactly its own `with`).
    pub fn only(location: Location) -> Self {
        Allow::Only(vec![location])
    }

    /// Permit nothing: every forwarded route is denied. The default
    /// for a portal with no pinned context of its own.
    pub fn none() -> Self {
        Allow::Only(Vec::new())
    }
}

impl FromStr for Allow {
    type Err = ParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let mut locations = Vec::new();
        for token in input.split_ascii_whitespace() {
            match token {
                // `*` subsumes every other token.
                "*" => return Ok(Allow::Any),
                _ => locations.push(token.parse()?),
            }
        }
        if locations.is_empty() {
            return Err(ParseError::EmptyAllow);
        }
        Ok(Allow::Only(locations))
    }
}

/// A malformed `with` / `allow` attribute value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// An empty (or all-whitespace) location token.
    EmptyLocation,
    /// A `@repo` token with nothing before the `@`.
    EmptyBranch(String),
    /// A `branch@` token with nothing after the `@`.
    EmptyRepo(String),
    /// A profile token without a name (`profile` / `main@profile:`).
    ProfileNeedsName(String),
    /// An empty `allow` attribute — a site must declare its reach.
    EmptyAllow,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::EmptyLocation => write!(f, "empty location, expected branch@repo"),
            ParseError::EmptyBranch(token) => {
                write!(f, "location {token:?} has an empty branch before '@'")
            }
            ParseError::EmptyRepo(token) => {
                write!(f, "location {token:?} has an empty repo after '@'")
            }
            ParseError::ProfileNeedsName(token) => {
                write!(
                    f,
                    "location {token:?} needs a profile name, e.g. profile:tonk"
                )
            }
            ParseError::EmptyAllow => {
                write!(f, "empty allow, expected '*' or branch@repo tokens")
            }
        }
    }
}

impl std::error::Error for ParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn it_parses_a_branch_at_repo_location() {
        let location: Location = "main@did:key:zAlice".parse().unwrap();
        assert_eq!(location.repo, Repo::Named("did:key:zAlice".into()));
        assert_eq!(location.branch(), Some("main"));
        assert_eq!(location.space(), Some("did:key:zAlice"));
        assert!(!location.profile());
    }

    #[dialog_common::test]
    fn it_parses_a_bare_repo_as_its_default_branch() {
        let location: Location = "did:key:zAlice".parse().unwrap();
        assert_eq!(location.branch(), None);
        assert_eq!(location.effective_branch(), "main");
    }

    #[dialog_common::test]
    fn it_parses_a_named_profile_token() {
        let location: Location = "main@profile:tonk".parse().unwrap();
        assert_eq!(location.repo, Repo::Profile("tonk".into()));
        assert_eq!(location.branch(), Some("main"));
        assert_eq!(location.space(), None);
        assert!(location.profile());
    }

    #[dialog_common::test]
    fn it_rejects_a_profile_token_without_a_name() {
        assert_eq!(
            "main@profile".parse::<Location>(),
            Err(ParseError::ProfileNeedsName("main@profile".into()))
        );
        assert_eq!(
            "profile".parse::<Location>(),
            Err(ParseError::ProfileNeedsName("profile".into()))
        );
        assert_eq!(
            "main@profile:".parse::<Location>(),
            Err(ParseError::ProfileNeedsName("main@profile:".into()))
        );
    }

    #[dialog_common::test]
    fn it_rejects_malformed_locations() {
        assert_eq!("".parse::<Location>(), Err(ParseError::EmptyLocation));
        assert_eq!("  ".parse::<Location>(), Err(ParseError::EmptyLocation));
        assert_eq!(
            "@did:key:zAlice".parse::<Location>(),
            Err(ParseError::EmptyBranch("@did:key:zAlice".into()))
        );
        assert_eq!(
            "main@".parse::<Location>(),
            Err(ParseError::EmptyRepo("main@".into()))
        );
    }

    #[dialog_common::test]
    fn it_round_trips_locations_through_display() {
        for token in ["main@did:key:zAlice", "did:key:zAlice", "main@profile:tonk"] {
            let location: Location = token.parse().unwrap();
            assert_eq!(location.to_string(), token);
        }
    }

    #[dialog_common::test]
    fn it_treats_a_bare_repo_and_its_main_branch_as_the_same_reach() {
        let bare: Location = "did:key:zAlice".parse().unwrap();
        let main: Location = "main@did:key:zAlice".parse().unwrap();
        let other: Location = "draft@did:key:zAlice".parse().unwrap();
        assert!(bare.same_reach(&main));
        assert!(!bare.same_reach(&other));
    }

    #[dialog_common::test]
    fn it_parses_star_as_allow_any() {
        assert_eq!("*".parse::<Allow>(), Ok(Allow::Any));
        // `*` subsumes any other token it appears with.
        assert_eq!("main@did:key:zA *".parse::<Allow>(), Ok(Allow::Any));
    }

    #[dialog_common::test]
    fn it_permits_only_listed_locations() {
        let allow: Allow = "main@did:key:zAlice main@did:key:zBob".parse().unwrap();
        let listed: Location = "did:key:zBob".parse().unwrap();
        let unlisted: Location = "did:key:zEve".parse().unwrap();
        let off_branch: Location = "draft@did:key:zAlice".parse().unwrap();
        assert!(allow.permits(&listed));
        assert!(!allow.permits(&unlisted));
        assert!(!allow.permits(&off_branch));
    }

    #[dialog_common::test]
    fn it_rejects_an_empty_allow() {
        assert_eq!("".parse::<Allow>(), Err(ParseError::EmptyAllow));
        assert_eq!("   ".parse::<Allow>(), Err(ParseError::EmptyAllow));
    }

    #[dialog_common::test]
    fn it_permits_anything_under_allow_any() {
        let allow = Allow::Any;
        let anywhere: Location = "wild@did:key:zEve".parse().unwrap();
        assert!(allow.permits(&anywhere));
    }

    #[dialog_common::test]
    fn it_permits_nothing_under_allow_none() {
        let allow = Allow::none();
        let anywhere: Location = "did:key:zEve".parse().unwrap();
        assert!(!allow.permits(&anywhere));
    }
}
