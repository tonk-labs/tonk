//! Signing in, which the host runs: the page that boots the kernels asks
//! the person who they are and runs the passkey. The profile only knows
//! that it asked, so what waits on the sign-in can tell when it is over.
//!
//! Overlay only: a sign-in is this device's, which other devices have no
//! use for.

use dialog_artifacts::Entity;
use dialog_query::Concept;

use crate::domain::sign_in::Asked;

/// The entity a sign-in under way is recorded on.
pub const ENTITY: &str = "state:sign-in";

/// Which passkey ceremony a sign-in ran.
pub mod kind {
    /// Creating an account with a new passkey.
    pub const CREATE: &str = "create";
    /// Logging in with a passkey the account already has.
    pub const LOG_IN: &str = "log-in";
    /// Signing in through another Tonk that holds the account.
    pub const SIGN_IN_VIA: &str = "sign-in-via";
}

/// The host is signing the person in, as the profile asked.
#[derive(Concept, Debug, Clone, PartialEq, PartialOrd)]
pub struct SigningIn {
    /// Always [`ENTITY`].
    pub this: Entity,
    /// When the sign-in was asked for, unix milliseconds.
    pub asked: Asked,
}
