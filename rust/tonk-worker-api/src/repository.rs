//! Repository-route wire DTOs.

use std::collections::HashMap;

use dialog_artifacts::Revision;
use dialog_varsig::Did;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Configuration for a single remote.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoteConfiguration {
    /// The remote's site address, carried verbatim as the JSON the
    /// worker serialized from its `SiteAddress` (an externally-tagged
    /// transport enum, e.g. `{"Ucan":{"endpoint":"…"}}`). The page
    /// only builds and forwards this, never inspects it, so it stays
    /// an opaque `Value` — no need to link the transport crates that
    /// define the real address enum, and no wire-shape to keep in sync.
    pub address: Value,
    /// Optional subject DID for the remote repository. Defaults to
    /// this repository's DID if omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<Did>,
}

impl RemoteConfiguration {
    /// Build a remote config for a UCAN-over-S3 access endpoint — the
    /// only transport the page configures. Produces the wire shape the
    /// worker's `SiteAddress` deserializes from
    /// (`{"Ucan":{"endpoint":"<url>"}}`).
    pub fn ucan(endpoint: impl Into<String>) -> Self {
        Self {
            address: json!({ "Ucan": { "endpoint": endpoint.into() } }),
            subject: None,
        }
    }

    /// Override the subject DID — by default the remote's subject
    /// is the same as the local repository's DID.
    pub fn subject(mut self, subject: Did) -> Self {
        self.subject = Some(subject);
        self
    }
}

/// Upstream wiring for a branch, pointing at a remote branch.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpstreamConfiguration {
    /// The remote's local name (e.g. `"origin"`).
    pub remote: String,
    /// The branch name on that remote.
    pub branch: String,
}

impl UpstreamConfiguration {
    /// Build an upstream config pointing at `{remote}/{branch}`.
    pub fn new(remote: impl Into<String>, branch: impl Into<String>) -> Self {
        Self {
            remote: remote.into(),
            branch: branch.into(),
        }
    }
}

/// Configuration / state for a single branch.
///
/// Same type is used for write (PUT body) and read (GET/PUT
/// response) — the server ignores `revision` on input and fills
/// it on output. Both fields serialize as `null` when absent so
/// the wire shape is consistent.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BranchConfiguration {
    /// Upstream wiring, or `null` if the branch has no upstream.
    #[serde(default)]
    pub upstream: Option<UpstreamConfiguration>,
    /// The branch's current revision, or `null` if it has no
    /// commits. Server-populated; ignored on incoming PUT bodies.
    #[serde(default)]
    pub revision: Option<Revision>,
}

impl BranchConfiguration {
    /// Attach an upstream pointing at `{remote}/{branch}`.
    pub fn upstream(mut self, remote: impl Into<String>, branch: impl Into<String>) -> Self {
        self.upstream = Some(UpstreamConfiguration::new(remote, branch));
        self
    }
}

/// Configuration for creating/updating a repository.
///
/// Serialized as the body of `PUT /api/repository/{repo}`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RepositoryConfiguration {
    /// Remotes to create, keyed by local name.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub remote: HashMap<String, RemoteConfiguration>,
    /// Branches to create, keyed by branch name.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub branch: HashMap<String, BranchConfiguration>,
}

impl RepositoryConfiguration {
    /// Add (or replace) a remote entry.
    pub fn remote(mut self, name: impl Into<String>, config: RemoteConfiguration) -> Self {
        self.remote.insert(name.into(), config);
        self
    }

    /// Add (or replace) a branch entry.
    pub fn branch(mut self, name: impl Into<String>, config: BranchConfiguration) -> Self {
        self.branch.insert(name.into(), config);
        self
    }
}

/// One member of a repository, assembled from the roster facts on
/// the meta branch. `did` is the member profile's did:key URI (the
/// meta entity, used directly as a `<tonk-sigil>` seed). `invited_by`
/// is the inviter's did:key, which the UI resolves to a name against
/// the member list; `None` for the founder and self-invites.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberInfo {
    /// The member profile's did:key URI.
    pub did: String,
    /// The member's published display name, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Whether this member is the active profile.
    pub is_self: bool,
    /// The inviter's did:key, when provenance was recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invited_by: Option<String>,
}

/// Read-side view of a repository.
///
/// Returned by `GET /api/repository/{repo}` and `PUT
/// /api/repository/{repo}` (on create). The shape mirrors the write
/// configuration but adds the observable fields — identifier DIDs
/// and per-branch revision state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RepositoryInfo {
    /// The repository's routing key (the DID suffix it's addressable
    /// at). The URL segment routes resolve through this; identity, not
    /// label.
    pub name: String,
    /// The user-typed display label, read from the repository's own
    /// `tonk/repository` name on its content branch (the cross-device
    /// source of truth). Distinct from `name`: two spaces may share a
    /// label, but each has a unique routing key.
    pub label: String,
    /// The repository's own DID.
    pub subject: Did,
    /// The operator's DID (ephemeral session key).
    pub operator: Did,
    /// The profile's DID (long-lived identity).
    pub profile: Did,
    /// Branches probed so far. Today only `main` is probed if it
    /// exists; other branches don't appear even if they're on disk.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub branch: HashMap<String, BranchConfiguration>,
    /// Remotes referenced by probed branches. Today only the
    /// remote that `main.upstream` points at is included.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub remote: HashMap<String, RemoteConfiguration>,
    /// The repository's members, read from the synced content branch.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<MemberInfo>,
}
