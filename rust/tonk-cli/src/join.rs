//! Validation and typed routing for `tonk join` invitation links.
//!
//! Short links are resolved before their payload chooses a parser. The
//! resolved bearer remains in memory only and is handed to the selected
//! import path so shortcut resolution is never repeated.

use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};
use tonk_invite::connection::InvitationHint;

/// Secret-free recovery journal for an ordinary invitation import.
pub const ORDINARY_STATE_FILE: &str = "ordinary-join.json";

/// Remaining remote work for an ordinary import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrdinaryPhase {
    /// The imported replica has not completed its required initial pull.
    PullPending,
    /// Membership/provenance is local but has not received a successful push.
    PublicationPending,
    /// No required remote work remains.
    Ready,
}

/// Public, non-authorizing state needed to resume an ordinary import.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct OrdinaryState {
    version: u8,
    /// Exact secret-free invitation entity.
    pub invitation: String,
    /// Repository subject expected at the retained replica.
    pub subject: String,
    /// Original directory to bind when completion succeeds.
    pub directory: std::path::PathBuf,
    /// Explicit aliases are never replaced with a repository display name.
    pub explicit_name: bool,
    /// Advisory mint-time name used only while synced content is unavailable.
    pub advisory_name: Option<String>,
    /// Whether this invitation configured a sync remote.
    pub has_remote: bool,
    /// Remaining required remote work.
    pub phase: OrdinaryPhase,
}

impl OrdinaryState {
    /// Construct recovery state for a previously claimed ordinary replica.
    ///
    /// New CLI imports never call this path. It exists only so an import
    /// journal written by an older CLI can still be resumed explicitly with
    /// `tonk --space NAME join`.
    pub fn legacy(
        invitation: &tonk_schema::Invitation,
        directory: &std::path::Path,
        explicit_name: bool,
        advisory_name: Option<&str>,
        has_remote: bool,
        phase: OrdinaryPhase,
    ) -> Result<Self> {
        Ok(Self {
            version: 1,
            invitation: invitation.this.to_string(),
            subject: invitation.subject.0.to_string(),
            directory: directory.canonicalize()?,
            explicit_name,
            advisory_name: advisory_name.map(str::to_owned),
            has_remote,
            phase,
        })
    }

    /// Read and validate ordinary recovery state.
    pub fn read(root: &std::path::Path) -> Result<Option<Self>> {
        let bytes = match std::fs::read(root.join(ORDINARY_STATE_FILE)) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let state: Self =
            serde_json::from_slice(&bytes).context("ordinary join state is malformed")?;
        let _: dialog_artifacts::Entity = state
            .invitation
            .parse()
            .context("ordinary join identity is malformed")?;
        let _: dialog_varsig::Did = state
            .subject
            .parse()
            .context("ordinary join subject is malformed")?;
        ensure!(
            state.version == 1 && state.directory.is_absolute(),
            "unsupported ordinary join state"
        );
        Ok(Some(state))
    }

    /// Atomically save recovery progress without bearer material.
    pub fn save(&self, root: &std::path::Path) -> Result<()> {
        crate::connections::atomic_public(root, ORDINARY_STATE_FILE, &serde_json::to_vec(self)?)
    }
}

/// An agent invitation whose signed public route and grants have been checked.
pub struct PreparedAgent {
    url: String,
    hint: InvitationHint,
}

impl std::fmt::Debug for PreparedAgent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedAgent")
            .field("hint", &self.hint)
            .field("url", &"[REDACTED]")
            .finish()
    }
}

impl PreparedAgent {
    /// Resolved bearer URL for full validation against trusted discovery.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Cryptographically checked public route and invitation identity.
    pub fn hint(&self) -> &InvitationHint {
        &self.hint
    }
}

/// Resolve an invitation once, reject ambiguous carriers, and validate it with
/// the parser selected by its versioned payload marker. A successful result is
/// always isolated tool authority; ordinary person invitations are recognized
/// only far enough to return the dedicated wrong-kind error.
pub async fn prepare(value: &str) -> Result<PreparedAgent> {
    let resolved = crate::invite::resolve_url(value).await?;
    let parsed = url::Url::parse(&resolved).context("invalid invitation URL")?;
    let agent_fragment = parsed
        .fragment()
        .is_some_and(|fragment| fragment.starts_with("tonk-agent-"));
    let mut ordinary_payload = false;
    let mut agent_payload = false;
    for (key, _) in parsed.query_pairs() {
        ordinary_payload |= key == "access";
        agent_payload |= key == "agent";
    }
    ensure!(
        !(ordinary_payload && (agent_fragment || agent_payload)),
        "ambiguous_invitation: link mixes ordinary and agent payloads"
    );
    ensure!(
        !agent_payload || agent_fragment,
        "malformed_agent_invitation: agent grants require a versioned agent fragment"
    );

    if agent_fragment {
        let hint = crate::connections::inspect_link(&resolved).await?;
        Ok(PreparedAgent {
            url: resolved,
            hint,
        })
    } else {
        crate::invite::preflight_resolved(resolved).await?;
        anyhow::bail!(
            "This link invites a person to the space.\nTo connect the CLI, ask for a link from \"connect agent\" in Tonk."
        )
    }
}
