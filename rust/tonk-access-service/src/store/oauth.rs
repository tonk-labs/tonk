//! OAuth authorizations, from `/authorize` to the single `/token` that
//! redeems them. See [`crate::oauth`].

use super::StoreError;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// An authorization the approval page has not finished yet.
///
/// Every secret is held as a hex SHA-256: `request` and `browser` are
/// hashes of what the browser holds, so the table alone cannot answer
/// for the page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingAuthorization {
    /// Hash of the handle the approval page carries.
    pub request: String,
    /// Hash of the cookie binding the request to the browser that
    /// started it.
    pub browser: String,
    /// The client, as `/authorize` named it.
    pub client_id: String,
    /// What the client calls itself, for the approval page.
    pub client_name: String,
    /// How the client is known: a metadata document or a dynamic
    /// registration.
    pub registration: String,
    /// The callback the code is delivered to, verbatim.
    pub redirect_uri: String,
    /// The client's `state`, returned with the code.
    pub state: String,
    /// The resource the token is for: this origin's `/mcp`.
    pub resource: String,
    /// The PKCE S256 challenge.
    pub challenge: String,
    /// When the approval page's time runs out.
    pub expires_at: u64,
}

/// What `/token` has to match to redeem a code.
#[derive(Debug, Clone, Copy)]
pub struct Redemption<'a> {
    /// Hash of the presented code.
    pub code: &'a str,
    /// The client presenting it.
    pub client_id: &'a str,
    /// The callback it was delivered to.
    pub redirect_uri: &'a str,
    /// The resource asked for.
    pub resource: &'a str,
    /// The S256 of the presented verifier.
    pub challenge: &'a str,
    /// The clock.
    pub now: u64,
}

/// Storage for [`crate::oauth`], shared by D1 and native SQLite.
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait AuthorizationStore {
    /// Record a new authorization, dropping any that have lapsed.
    async fn begin_authorization(
        &self,
        row: &PendingAuthorization,
        now: u64,
    ) -> Result<(), StoreError>;
    /// The authorization `request` names, while it awaits approval.
    async fn pending_authorization(
        &self,
        request: &str,
        now: u64,
    ) -> Result<Option<PendingAuthorization>, StoreError>;
    /// Bind a sealed token to a code, once, for the browser that began
    /// the request. The code lives until `expires_at`.
    async fn approve_authorization(
        &self,
        request: &str,
        browser: &str,
        code: &str,
        token: &str,
        now: u64,
        expires_at: u64,
    ) -> Result<bool, StoreError>;
    /// Drop a pending authorization the user refused.
    async fn deny_authorization(
        &self,
        request: &str,
        browser: &str,
        now: u64,
    ) -> Result<bool, StoreError>;
    /// Remove the authorization a code names and answer its sealed
    /// token, only when everything in `redemption` matches. One
    /// statement, so a code is redeemed at most once however many
    /// requests race for it; a mismatch leaves it in place.
    async fn redeem_authorization(
        &self,
        redemption: Redemption<'_>,
    ) -> Result<Option<String>, StoreError>;
}

pub(crate) const PRUNE: &str = "DELETE FROM oauth_authorization WHERE expires_at<=?1";
pub(crate) const BEGIN: &str = "INSERT INTO oauth_authorization (request,browser,client_id,client_name,registration,redirect_uri,state,resource,challenge,expires_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)";
pub(crate) const PENDING: &str = "SELECT request,browser,client_id,client_name,registration,redirect_uri,state,resource,challenge,expires_at FROM oauth_authorization WHERE request=?1 AND code IS NULL AND expires_at>?2";
pub(crate) const APPROVE: &str = "UPDATE oauth_authorization SET code=?3,token=?4,expires_at=?6 WHERE request=?1 AND browser=?2 AND code IS NULL AND expires_at>?5";
pub(crate) const DENY: &str = "DELETE FROM oauth_authorization WHERE request=?1 AND browser=?2 AND code IS NULL AND expires_at>?3";
pub(crate) const REDEEM: &str = "DELETE FROM oauth_authorization WHERE code=?1 AND client_id=?2 AND redirect_uri=?3 AND resource=?4 AND challenge=?5 AND expires_at>?6 RETURNING token";
