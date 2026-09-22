//! Validation and typed routing for `tonk join` invitation links.
//!
//! Short links are resolved before their payload chooses a parser. The
//! resolved bearer remains in memory only and is handed to the selected
//! import path so shortcut resolution is never repeated.

use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};
use tonk_invite::connection::InvitationHint;
use crate::peer::NativePeer;

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
    /// Construct recovery state for a newly claimed replica.
    pub fn new(
        prepared: &PreparedOrdinary,
        directory: &std::path::Path,
        explicit_name: bool,
        has_remote: bool,
        phase: OrdinaryPhase,
    ) -> Result<Self> {
        Ok(Self {
            version: 1,
            invitation: prepared.invitation().this.to_string(),
            subject: prepared.invitation().subject.0.to_string(),
            directory: directory.canonicalize()?,
            explicit_name,
            advisory_name: prepared.advisory_name().map(str::to_owned),
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

    /// Advisory display name signed into the invitation at mint time.
    pub fn advisory_name(&self) -> Option<&str> {
        self.preflight.invite.space_name.as_deref()
    }

    /// Whether the validated invitation configures a sync remote.
    pub fn has_remote(&self) -> bool {
        self.preflight.invite.remote_url.is_some()
    }

    /// Consume this prepared value for import.
    pub fn into_preflight(self) -> crate::invite::InvitePreflight {
        self.preflight
    }
}

/// Reject a targeted ordinary invitation unless its exact recipient authority
/// is already available locally. This reads existing credentials and never
/// creates an onboarding account or switches accounts.
pub async fn ensure_ordinary_recipient(
    prepared: &PreparedOrdinary,
    config: &crate::site::SiteConfig,
) -> Result<()> {
    use dialog_storage::provider::storage::{NativeSpace, Storage};

    let Some(expected) = prepared.expected_root() else {
        return Ok(());
    };
    let storage = Storage::<NativeSpace>::default();
    let profile = dialog_peer::Peer::new()
        .storage(storage.clone())
        .load(dialog_effects::storage::Location::new(config.profile_directory.clone(), config.profile_name.clone()))
        .await
        .map_err(|_| targeted_recipient_error(expected))?;
    let operator = crate::account_state::store_operator_with_config(
        &profile,
        &config.account_store,
        &config.profile_name,
        config.profile_directory.clone(),
    )
    .await?;
    let local_root = crate::identity::local_root_with_operator(&profile, &operator)
        .await?
        .map(|root| root.root_did);
    let onboarding = crate::onboarding::did(&profile, &operator)
        .await?
        .map(|did| did.to_string());
    ensure!(
        local_root.as_deref() == Some(expected.as_str())
            || onboarding.as_deref() == Some(expected.as_str()),
        targeted_recipient_error(expected)
    );
    Ok(())
}

fn targeted_recipient_error(expected: &dialog_varsig::Did) -> anyhow::Error {
    anyhow::anyhow!(
        "invitation_recipient_mismatch: this invitation targets {expected}, which is not available on this device; request an invitation for an eligible local identity"
    )
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
    Ordinary(Box<PreparedOrdinary>),
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
        !agent_payload || agent_fragment,
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
        Ok(PreparedInvitation::Ordinary(Box::new(PreparedOrdinary {
            preflight,
        })))
    }
}
