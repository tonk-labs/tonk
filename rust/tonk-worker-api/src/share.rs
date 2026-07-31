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

/// The spot has no upstream, so an invite would land its recipient in a spot
/// that can never fill. Repairable: the bar offers to attach this server.
pub const BLOCKED_NOT_SYNCED: &str = "not-synced";

/// The spot's sync server cannot be shared (a local-only or non-UCAN remote).
/// Terminal: nothing the user can do from the bar.
pub const BLOCKED_UNSHAREABLE_REMOTE: &str = "unshareable-remote";

/// The remote carries no revocation relay, so a minted invite would have
/// nowhere to publish its revocation. Repairable: the bar offers to attach
/// the relay the spot's own sync server advertises, which is an upsert onto
/// the existing remote rather than a second one.
pub const BLOCKED_MISSING_REVOCATION_RELAY: &str = "missing-revocation-relay";

/// This replica is a guest visit: it holds bounded invite authority, not the
/// durable membership a mint delegates from. Repairable: the bar offers to
/// join the spot, which is what raises the passkey prompt.
pub const BLOCKED_NEEDS_MEMBERSHIP: &str = "needs-membership";
