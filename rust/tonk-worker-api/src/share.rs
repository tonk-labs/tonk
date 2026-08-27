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

/// The active profile is not attached to an account. An invite derives its
/// durable authority from that account, so the bar hands off to login rather
/// than minting from a transient local profile.
pub const BLOCKED_ACCOUNT_REQUIRED: &str = "account-required";

/// The spot's sync server cannot be shared (a local-only or non-UCAN remote).
/// Terminal: nothing the user can do from the bar.
pub const BLOCKED_UNSHAREABLE_REMOTE: &str = "unshareable-remote";
