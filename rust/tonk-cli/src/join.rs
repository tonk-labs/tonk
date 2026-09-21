//! Validation and typed routing for `tonk join` invitation links.
//!
//! Short links are resolved before their payload chooses a parser. The
//! resolved bearer remains in memory only and is handed to the selected
//! import path so shortcut resolution is never repeated.

use anyhow::{Context as _, Result, ensure};
use tonk_invite::connection::InvitationHint;

/// An ordinary sharing invitation validated without changing local state.
pub struct PreparedOrdinary {
    preflight: crate::invite::InvitePreflight,
}

impl std::fmt::Debug for PreparedOrdinary {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedOrdinary")
            .field("invitation", &self.preflight.invitation)
            .field("expected_root", &self.preflight.expected_root)
            .field("url", &"[REDACTED]")
            .finish()
    }
}

impl PreparedOrdinary {
    /// Resolved bearer URL for the existing ordinary claim adapter.
    pub fn url(&self) -> &str {
        &self.preflight.url
    }

    /// Stable public identity of the exact invitation.
    pub fn invitation(&self) -> &tonk_schema::Invitation {
        &self.preflight.invitation
    }

    /// Required local root for a targeted invitation.
    pub fn expected_root(&self) -> Option<&dialog_varsig::Did> {
        self.preflight.expected_root.as_ref()
    }

    /// Consume this prepared value for import.
    pub fn into_preflight(self) -> crate::invite::InvitePreflight {
        self.preflight
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

/// A resolved invitation dispatched to exactly one validated parser.
#[derive(Debug)]
pub enum PreparedInvitation {
    /// Ordinary invitation authority claimed to an eligible local identity.
    Ordinary(PreparedOrdinary),
    /// Isolated agent credentials retained under their own recipient key.
    Agent(PreparedAgent),
}

/// Resolve an invitation once, reject ambiguous carriers, and validate it with
/// the parser selected by its versioned payload marker.
pub async fn prepare(value: &str) -> Result<PreparedInvitation> {
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
        !(agent_payload && !agent_fragment),
        "malformed_agent_invitation: agent grants require a versioned agent fragment"
    );

    if agent_fragment {
        let hint = crate::connections::inspect_link(&resolved).await?;
        Ok(PreparedInvitation::Agent(PreparedAgent {
            url: resolved,
            hint,
        }))
    } else {
        let preflight = crate::invite::preflight_resolved(resolved).await?;
        Ok(PreparedInvitation::Ordinary(PreparedOrdinary { preflight }))
    }
}
