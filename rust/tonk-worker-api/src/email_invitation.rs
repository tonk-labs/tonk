//! Fixed outcome messages of a refused email-invitation claim.
//!
//! The worker publishes these as the claim's outcome `message`, and the
//! invitation page reads them back to choose which state to show, so both
//! sides name the same strings.

/// The access service refused the account (HTTP 403).
pub const WRONG_ACCOUNT: &str = "Sign in with the account that received this invitation.";

/// The account's email is unconfirmed, or the invitation changed (HTTP 409).
pub const UNCONFIRMED: &str = "Confirm your email, then try again.";

/// No live invitation remains (HTTP 410).
pub const GONE: &str = "This invitation has expired or was cancelled. Ask for another invitation.";
