//! Refusal codes a share attempt can answer with.
//!
//! A refused share does not error: the worker records a `ShareBlocked` fact
//! carrying one of these codes plus a sentence for the user, and the bar reads
//! it off its live subscription. The code is the part the bar branches on —
//! which refusals it can offer a repair for, and which it can only report —
//! so both sides have to agree on the exact string.
//!
//! They live here, in the wire crate both sides already depend on, because
//! they are wire vocabulary. Held as literals at each end they would agree
//! only by luck, and a disagreement is silent: the bar would fall through to
//! its terminal branch and report a refusal it could have repaired.

/// The space has no upstream, so an invite would land its recipient in a space
/// that can never fill. Repairable: the bar offers to attach this server.
pub const BLOCKED_NOT_SYNCED: &str = "not-synced";

/// The active profile is not attached to an account. An invite derives its
/// durable authority from that account, so the bar hands off to login rather
/// than minting from a transient local profile.
pub const BLOCKED_ACCOUNT_REQUIRED: &str = "account-required";

/// The space's sync server cannot be shared (a local-only or non-UCAN remote).
/// Terminal: nothing the user can do from the bar.
pub const BLOCKED_UNSHAREABLE_REMOTE: &str = "unshareable-remote";

/// The service refused to provision this space under the account, so a
/// link would point at a space the service will not serve. Terminal
/// from the bar: the refusal names its reason in `detail`, and sharing
/// again after it is addressed re-runs provisioning.
pub const BLOCKED_NOT_PROVISIONED: &str = "not-provisioned";

/// The space has no upstream and this device has no account registered
/// with a provider, so there is nothing to attach it to. Repairable, but
/// not by attaching a remote: the bar asks the user to register.
///
/// Distinct from [`BLOCKED_NOT_SYNCED`] because the remedy differs. A
/// device has an account from first boot, so "no provider" here means
/// nobody has signed up yet, not that something failed.
pub const BLOCKED_NEEDS_ACCOUNT: &str = "needs-account";

/// The account enrolled but has not confirmed the emailed activation
/// link, so the access service serves it nothing yet. Repairable by the
/// user, in their inbox rather than in the bar.
///
/// Offering "turn on sync" here would attach a remote the service
/// refuses, which is the failure this class exists to name instead.
pub const BLOCKED_NEEDS_ACTIVATION: &str = "needs-activation";

/// The account's service was withdrawn. Terminal: no email confirms this
/// away and nothing in the bar repairs it.
pub const BLOCKED_SUSPENDED: &str = "suspended";
