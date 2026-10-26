//! How the person's profile hands a space's host the key of the space's
//! session. The profile derives the key from its own, for the space, so it
//! is the same key each time the host asks, and conceals its seed to a key
//! the host made to receive it. The host keeps the key it reveals as a signer
//! nothing reads the seed of.

/// The context a profile derives the key of a space's session in, with the
/// space's DID as the label.
pub const DERIVATION: &str = "tonk.kernel/session/v1";

/// The context the seed of a session's key is concealed in, to the key the
/// host made to receive it.
pub const HANDOVER: &str = "tonk.kernel/session-handover/v1";
