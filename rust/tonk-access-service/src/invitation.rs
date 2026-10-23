//! Email invitation delivery: authenticated, durable, and bound to a confirmed account.

use crate::store::{
    Store,
    invitation::{InvitationStore, PendingInvitation},
};
use dialog_common::ConditionalSync;
use dialog_credentials::{DidKeyResolver, Ed25519Signer, Signer};
use dialog_ucan_core::promise::Promised;
use dialog_ucan_core::revocation::RevocationChecker;
use dialog_ucan_core::verification::{Environment, VerificationContext};
use dialog_ucan_core::{DelegationChain, InvocationBuilder, InvocationChain, time::Timestamp};
use dialog_varsig::{AnySignature, Did, Principal};
use ipld_core::ipld::Ipld;
use serde_json::{Value, json};
use tonk_account::customer::CustomerStatus;
use tonk_invite::{
    Invite, InviteAudience,
    delivery::{DELIVERY_ID, DeliveryEnvelope, valid_id},
};

/// Maximum pending lifetime; issued account authority follows the original grant.
pub const LIFETIME: u64 = 7 * 86400;

/// Fixed, non-secret errors safe to render to either participant.
#[derive(Debug, thiserror::Error)]
pub enum InvitationError {
    /// Malformed request.
    #[error("The invitation request is invalid.")]
    Invalid,
    /// Invalid/revoked account authority or mismatched recipient. The
    /// refusal names the failed check for the service log only.
    #[error("This account cannot use this invitation.")]
    Forbidden(Refusal),
    /// No live pending invitation remains.
    #[error("This invitation is missing, expired, or cancelled.")]
    Gone,
    /// A registration has not been confirmed.
    #[error("Confirm the invited email before joining this space.")]
    Unconfirmed,
    /// Immutable content differs, or claim/cancel already won.
    #[error("The invitation has already changed. Refresh before continuing.")]
    Conflict,
    /// Delivery is bounded and retryable only after its cooldown.
    #[error("Invitation delivery is limited. Wait before retrying the same invitation.")]
    Limited,
    /// Storage/transport did not acknowledge the operation.
    #[error("Invitation delivery could not be confirmed. Retry the same invitation.")]
    Unavailable,
}
impl InvitationError {
    /// HTTP status for the closed error vocabulary.
    pub fn status(&self) -> u16 {
        match self {
            Self::Invalid => 400,
            Self::Forbidden(_) => 403,
            Self::Gone => 410,
            Self::Unconfirmed | Self::Conflict => 409,
            Self::Limited => 429,
            Self::Unavailable => 503,
        }
    }
}

/// Which check refused an invitation request. Logged by the service so a
/// refusal can be diagnosed; never rendered to either participant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The caller's invocation failed verification: signature, time bounds,
    /// or a revoked proof.
    Authority,
    /// The invocation was addressed to another service.
    Audience,
    /// The invocation had no expiry or one outside the five-minute window.
    Expiry,
    /// The caller is not a customer of this service.
    UnknownCaller,
    /// The space has no subscription on this service.
    NotHosted,
    /// Someone other than the original sender acted on the invitation.
    NotSender,
    /// The signed grant names another space.
    Space,
    /// The signed grant is not delegated to this invitation's delivery key.
    DeliveryKey,
    /// The signed grant does not attribute the invitation to the caller.
    Inviter,
    /// The signed grant carries another invitation's delivery id.
    DeliveryId,
    /// The signed grant points its home address at another deployment.
    HomeAddress,
    /// The signed grant does not let the delivery key use the space: a
    /// revoked, expired, or insufficient proof.
    Grant,
    /// The invitation was already claimed by another account.
    OtherRecipient,
    /// The caller's confirmed email is not the invited address.
    EmailMismatch,
    /// Re-delegating the grant to the claimant failed.
    Claim,
}

impl Refusal {
    /// Stable snake_case name for logs.
    pub fn name(self) -> &'static str {
        match self {
            Self::Authority => "authority",
            Self::Audience => "audience",
            Self::Expiry => "expiry",
            Self::UnknownCaller => "unknown_caller",
            Self::NotHosted => "not_hosted",
            Self::NotSender => "not_sender",
            Self::Space => "space",
            Self::DeliveryKey => "delivery_key",
            Self::Inviter => "inviter",
            Self::DeliveryId => "delivery_id",
            Self::HomeAddress => "home_address",
            Self::Grant => "grant",
            Self::OtherRecipient => "other_recipient",
            Self::EmailMismatch => "email_mismatch",
            Self::Claim => "claim",
        }
    }
}

/// Transport must deduplicate retries with this invitation's stable idempotency key.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch="wasm32", async_trait::async_trait(?Send))]
pub trait InvitationEmailSender {
    /// Send a locator only, never the delivery key or an open invitation.
    async fn send_invitation(
        &self,
        email: &str,
        link: &str,
        id: &str,
    ) -> Result<(), InvitationError>;
}

/// Request-scoped service dependencies; no sender or recipient key is retained.
pub struct InvitationService<'a, S, E, R> {
    /// Authoritative control database, not the eventual-consistency lookup replica.
    pub store: &'a S,
    /// Idempotent email transport.
    pub email: &'a E,
    /// Configured service signer.
    pub service: &'a Ed25519Signer,
    /// Existing service secret used for domain-separated delivery-key derivation.
    pub service_seed: &'a str,
    /// This deployment's origin.
    pub origin: &'a str,
    /// Current wall-clock time in seconds.
    pub now: u64,
    /// Same revocation checker as storage admission.
    pub revocations: &'a R,
}

impl<S: Store + InvitationStore, E: InvitationEmailSender, R: RevocationChecker + ConditionalSync>
    InvitationService<'_, S, E, R>
{
    /// Verify a short-lived account invocation and execute one exact operation.
    pub async fn handle(&self, body: &[u8]) -> Result<Value, InvitationError> {
        use InvitationError::*;
        if body.len() > 128 * 1024 {
            return Err(Invalid);
        }
        let call = InvocationChain::<AnySignature>::try_from(body).map_err(|_| Invalid)?;
        self.verify(&call, Refusal::Authority).await?;
        if call.invocation.audience() != &self.service.did() {
            return Err(Forbidden(Refusal::Audience));
        }
        let expiry = call
            .invocation
            .expiration()
            .ok_or(Forbidden(Refusal::Expiry))?
            .to_unix();
        if expiry <= self.now || expiry > self.now + 300 {
            return Err(Forbidden(Refusal::Expiry));
        }
        let args = call.invocation.arguments();
        let text = |name: &str| match args.get(name) {
            Some(Promised::String(value)) => Ok(value.as_str()),
            _ => Err(Invalid),
        };
        let id = text("invitation")?;
        if !valid_id(id) {
            return Err(Invalid);
        }
        let caller = call.subject();
        let customer = self
            .store
            .customer(caller.as_str())
            .await
            .map_err(|_| Unavailable)?
            .ok_or(Forbidden(Refusal::UnknownCaller))?;
        if customer.status != CustomerStatus::Active {
            return Err(Unconfirmed);
        }
        let command: Vec<_> = call.command().0.iter().map(String::as_str).collect();
        if command == ["email-invitation", "prepare"] {
            if args.len() != 3 {
                return Err(Invalid);
            }
            let email = crate::email::normalize_email(text("email")?);
            let parts: Vec<_> = email.split('@').collect();
            if email.len() > 254
                || parts.len() != 2
                || parts.iter().any(|s| s.is_empty())
                || email.chars().any(char::is_whitespace)
                || email.contains(['\r', '\n'])
            {
                return Err(Invalid);
            }
            let space: Did = text("space")?.parse().map_err(|_| Invalid)?;
            let existing = self.store.invitation(id).await.map_err(|_| Unavailable)?;
            if let Some(row) = existing {
                if row.sender != caller.as_str()
                    || row.email != email
                    || row.space != space.as_str()
                {
                    return Err(Conflict);
                }
                self.live(&row)?;
                return Ok(
                    json!({"invitation":id,"recipientRoot":self.key(id).await?.did(),"expiresAt":row.expires_at,"original":row.original}),
                );
            }
            if self
                .store
                .customer_by_email(&email)
                .await
                .map_err(|_| Unavailable)?
                .is_some()
            {
                return Err(Conflict);
            }
            if !self
                .store
                .servability(space.as_str())
                .await
                .map_err(|_| Unavailable)?
                .consumer
            {
                return Err(Forbidden(Refusal::NotHosted));
            }
            let row = PendingInvitation {
                id: id.into(),
                sender: caller.to_string(),
                email,
                space: space.to_string(),
                created_at: self.now,
                expires_at: self.now + LIFETIME,
                original: None,
                recipient: None,
                completed: None,
                sent_at: None,
                cancelled: 0,
                attempts: 0,
                next_attempt_at: 0,
            };
            if !self
                .store
                .reserve_invitation(&row)
                .await
                .map_err(|_| Unavailable)?
            {
                return Err(Limited);
            }
            return Ok(
                json!({"invitation":id,"recipientRoot":self.key(id).await?.did(),"expiresAt":row.expires_at,"original":row.original}),
            );
        }
        let mut row = self
            .store
            .invitation(id)
            .await
            .map_err(|_| Unavailable)?
            .ok_or(Gone)?;
        self.live(&row)?;
        match command.as_slice() {
            ["email-invitation", "send"] => {
                if args.len() != 2 || row.sender != caller.as_str() {
                    return Err(Forbidden(Refusal::NotSender));
                }
                let original = text("original")?;
                let chain = decode(original)?;
                if chain.subject().map(Did::as_str) != Some(row.space.as_str()) {
                    return Err(Forbidden(Refusal::Space));
                }
                if chain.audience() != &self.key(id).await?.did() {
                    return Err(Forbidden(Refusal::DeliveryKey));
                }
                if tonk_invite::inviter(&chain).map_err(|_| Invalid)?.as_ref() != Some(caller) {
                    return Err(Forbidden(Refusal::Inviter));
                }
                let leaf = chain.proofs().last().ok_or(Invalid)?;
                if leaf.meta().get(DELIVERY_ID) != Some(&Ipld::String(id.into())) {
                    return Err(Forbidden(Refusal::DeliveryId));
                }
                let remote = tonk_invite::home_address(&chain)
                    .map_err(|_| Invalid)?
                    .ok_or(Invalid)?;
                if remote.origin().ascii_serialization() != self.origin {
                    return Err(Forbidden(Refusal::HomeAddress));
                }
                self.verify_grant(&chain, id).await?;
                if row.original.is_none() {
                    self.store
                        .publish_invitation(id, original, self.now)
                        .await
                        .map_err(|_| Unavailable)?;
                    row = self
                        .store
                        .invitation(id)
                        .await
                        .map_err(|_| Unavailable)?
                        .ok_or(Gone)?;
                }
                if row.original.as_deref() != Some(original) {
                    return Err(Conflict);
                }
                if row.sent_at.is_some() {
                    return Ok(json!({"state":"sent","invitation":id}));
                }
                if !self
                    .store
                    .lease_invitation(id, self.now)
                    .await
                    .map_err(|_| Unavailable)?
                {
                    return Err(Limited);
                }
                let link = format!("{}/email-invite#{}", self.origin, id);
                self.email.send_invitation(&row.email, &link, id).await?;
                self.store
                    .invitation_sent(id, self.now)
                    .await
                    .map_err(|_| Unavailable)?;
                Ok(json!({"state":"sent","invitation":id}))
            }
            ["email-invitation", "claim"] => {
                if args.len() != 1 {
                    return Err(Invalid);
                }
                if let Some(recipient) = &row.recipient {
                    if recipient != caller.as_str() {
                        return Err(Forbidden(Refusal::OtherRecipient));
                    }
                    return self.delivery(row, caller).await;
                }
                if customer.email != row.email {
                    return Err(Forbidden(Refusal::EmailMismatch));
                }
                let original = decode(row.original.as_deref().ok_or(Gone)?)?;
                self.verify_grant(&original, id).await?;
                let seed = self.seed(id)?;
                let invite = Invite::new(original, InviteAudience::Open { seed }, None)
                    .await
                    .map_err(|_| Invalid)?;
                let completed = invite
                    .claim(caller)
                    .await
                    .map_err(|_| Forbidden(Refusal::Claim))?
                    .chain;
                let completed =
                    bs58::encode(completed.to_bytes().map_err(|_| Unavailable)?).into_string();
                self.store
                    .complete_invitation(id, caller.as_str(), &completed, self.now)
                    .await
                    .map_err(|_| Unavailable)?;
                let row = self
                    .store
                    .invitation(id)
                    .await
                    .map_err(|_| Unavailable)?
                    .ok_or(Gone)?;
                if row.recipient.as_deref() != Some(caller.as_str()) {
                    return Err(Conflict);
                }
                self.delivery(row, caller).await
            }
            ["email-invitation", "cancel"] => {
                if args.len() != 1 || row.sender != caller.as_str() {
                    return Err(Forbidden(Refusal::NotSender));
                }
                if !self
                    .store
                    .cancel_invitation(id, caller.as_str())
                    .await
                    .map_err(|_| Unavailable)?
                {
                    return Err(Conflict);
                }
                Ok(json!({"state":"cancelled"}))
            }
            _ => Err(Invalid),
        }
    }

    async fn delivery(
        &self,
        row: PendingInvitation,
        recipient: &Did,
    ) -> Result<Value, InvitationError> {
        let envelope = DeliveryEnvelope {
            invitation: row.id,
            original: row.original.ok_or(InvitationError::Gone)?,
            completed: row.completed.ok_or(InvitationError::Gone)?,
        };
        envelope
            .into_invite(recipient, &envelope.invitation)
            .await
            .map_err(|_| InvitationError::Invalid)?;
        serde_json::to_value(envelope).map_err(|_| InvitationError::Unavailable)
    }

    fn live(&self, row: &PendingInvitation) -> Result<(), InvitationError> {
        if row.cancelled != 0 || row.expires_at <= self.now {
            Err(InvitationError::Gone)
        } else {
            Ok(())
        }
    }

    fn seed(&self, id: &str) -> Result<[u8; 32], InvitationError> {
        let mut material =
            hex::decode(self.service_seed.trim()).map_err(|_| InvitationError::Unavailable)?;
        if material.len() != 32 {
            return Err(InvitationError::Unavailable);
        }
        material.extend_from_slice(id.as_bytes());
        Ok(blake3::derive_key(
            "tonk.email-invitation.delivery-key.v1",
            &material,
        ))
    }

    async fn key(&self, id: &str) -> Result<Ed25519Signer, InvitationError> {
        Ed25519Signer::import(&self.seed(id)?)
            .await
            .map_err(|_| InvitationError::Unavailable)
    }

    async fn verify(
        &self,
        chain: &InvocationChain<AnySignature>,
        refusal: Refusal,
    ) -> Result<(), InvitationError> {
        chain
            .verify(&VerificationContext::new(&Environment::new(
                chain.proof_store(),
                DidKeyResolver,
                self.revocations,
            )))
            .await
            .map_err(|_| InvitationError::Forbidden(refusal))?;
        Ok(())
    }

    // Prove that the delivery key can exercise the original space grant. Checking
    // signatures alone would permit a chain with widened scopes or revoked proofs.
    async fn verify_grant(&self, chain: &DelegationChain, id: &str) -> Result<(), InvitationError> {
        let invocation = InvocationBuilder::new()
            .issuer(Signer::from(self.key(id).await?))
            .audience(&self.service.did())
            .subject(chain.subject().ok_or(InvitationError::Invalid)?)
            .command(vec!["use".into()])
            .proofs(chain.proof_cids().to_vec())
            .expiration(Timestamp::five_minutes_from_now())
            .try_build()
            .await
            .map_err(|_| InvitationError::Invalid)?;
        self.verify(
            &InvocationChain::new(invocation, chain.export().collect()),
            Refusal::Grant,
        )
        .await
    }
}

fn decode(value: &str) -> Result<DelegationChain, InvitationError> {
    let bytes = bs58::decode(value)
        .into_vec()
        .map_err(|_| InvitationError::Invalid)?;
    DelegationChain::try_from(bytes.as_slice()).map_err(|_| InvitationError::Invalid)
}

/// Dispatch hint only. The service verifies all bytes before performing anything.
pub fn is_invitation(body: &[u8]) -> bool {
    InvocationChain::<AnySignature>::try_from(body)
        .ok()
        .is_some_and(|chain| {
            chain.command().0.first().map(String::as_str) == Some("email-invitation")
        })
}

#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]
mod tests;
