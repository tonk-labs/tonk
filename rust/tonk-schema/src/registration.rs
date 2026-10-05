//! Adding an account to this profile, one stage at a time.
//!
//! Each stage is a concept of its own on the profile's overlay, all on
//! one entity ([`ENTITY`]), and at most one is there at a time: the worker
//! clears the entity before it records the next. A view matches the stage
//! it draws by shape. No stage at all is the panel put away, and a finished
//! account is what the account's own facts say, so neither has a concept.
//!
//! Overlay only: the stages describe a panel on this device, which other
//! devices have no use for, and the address typed into it is not to sync.

use dialog_artifacts::Entity;
use dialog_query::Concept;

use crate::domain::registration::{address, ceremony, confirming, failed, naming, via};

/// The entity every stage is recorded on.
pub const ENTITY: &str = "state:registration";

/// Which passkey ceremony a stage is about.
pub mod kind {
    /// Creating an account with a new passkey.
    pub const CREATE: &str = "create";
    /// Logging in with a passkey the account already has.
    pub const LOG_IN: &str = "log-in";
    /// Signing in through another Tonk that holds the account.
    pub const SIGN_IN_VIA: &str = "sign-in-via";
}

/// The panel asks for an address.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RegistrationAddress {
    /// Always [`ENTITY`].
    pub this: Entity,
    /// The address typed so far, empty at first.
    pub email: address::Email,
}

/// The panel asks which Tonk holds the account, to sign this browser in
/// through it.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RegistrationVia {
    /// Always [`ENTITY`].
    pub this: Entity,
    /// The address typed so far, empty at first.
    pub origin: via::Origin,
}

/// The address is free, and the panel asks what to call the account.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RegistrationNaming {
    /// Always [`ENTITY`].
    pub this: Entity,
    /// The address the account will have.
    pub email: naming::Email,
}

/// The page has been asked for the passkey.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RegistrationCeremony {
    /// Always [`ENTITY`].
    pub this: Entity,
    /// One of [`kind`].
    pub kind: ceremony::Kind,
}

/// The account waits for its emailed link to be opened.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RegistrationConfirming {
    /// Always [`ENTITY`].
    pub this: Entity,
    /// One of [`kind`]: which ceremony brought the account here.
    pub kind: confirming::Kind,
}

/// The last step did not finish.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RegistrationFailed {
    /// Always [`ENTITY`].
    pub this: Entity,
    /// One of [`kind`]: which ceremony failed.
    pub kind: failed::Kind,
    /// What went wrong, in words a person can act on.
    pub message: failed::Message,
}
