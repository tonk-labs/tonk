//! Clearance: which key is allowed to wrap which secret.
//!
//! Every secret this device stores sits at one of two levels, and a key
//! at one level wraps only secrets at that level. The levels are ordered
//! by blast radius, so what a compromise costs is exactly the subtree
//! beneath the key that leaked:
//!
//! ```text
//!   Recovery   passkey / pre-passkey custodian
//!      │       wraps: the account secret
//!      ▼
//!   Account    HKDF(account secret)
//!              wraps: space seeds, invite seeds
//! ```
//!
//! # What is not a level
//!
//! Session state and the local root grant look like they want a third,
//! device-scoped level, but neither is a secret: a session record holds
//! a KDF *context* whose other half is the profile seed, and a local
//! root is a delegation, which is a proof rather than a key. Nothing is
//! recoverable-by-this-profile-alone today, so there is no such level.
//! Adding one is a few lines here if that changes.
//!
//! # Why this is types rather than documentation
//!
//! [`Kek<C>`](crate::envelope::Kek) and [`Envelope<C>`] carry their
//! clearance in the type, so wrapping a space seed with a profile key is
//! a compile error rather than something review has to catch. A
//! mis-tiered wrap is otherwise invisible: it encrypts fine and only
//! fails much later, when the wrong key cannot open it.

use std::fmt::Debug;

/// A clearance level. Sealed: the two levels below are the whole set,
/// and adding a third is a deliberate change to this module rather than
/// something a downstream crate can do.
pub trait Clearance: Debug + Copy + private::Sealed {
    /// HKDF info binding a derived key to this level. Distinct per
    /// level, so the same input material can never expand to the same
    /// bytes at two levels.
    const CONTEXT: &'static [u8];

    /// Wire tag recorded in the envelope header. Read back on open, so
    /// a blob sealed at one level is refused at another even when the
    /// caller's types would have allowed it.
    const TAG: u8;

    /// Name for error messages.
    const NAME: &'static str;
}

/// The top level: whatever custodies the account secret itself — a
/// passkey, a recovery phrase, or the pre-passkey device custodian that
/// stands in for one during onboarding.
///
/// Compromise here reaches everything, which is why accreditation
/// rotates the account secret rather than re-wrapping it: a custodian
/// that was compromised before accreditation must not retain reach
/// afterwards.
#[derive(Debug, Clone, Copy)]
pub struct Recovery;

/// The account level: keys derived from the account secret, wrapping
/// the secrets the account custodies — space signing seeds and invite
/// seeds.
///
/// Compromise costs the spaces and invites, not the account. Because
/// this key derives from the account secret, rotating that secret
/// rotates this key, and every seed wrapped under it must be re-wrapped
/// during accreditation. That re-wrap is the point: it is what lets a
/// space be re-issued under the new account instead of leaving the old
/// one in the chain forever.
#[derive(Debug, Clone, Copy)]
pub struct Account;

impl Clearance for Recovery {
    const CONTEXT: &'static [u8] = b"tonk/kek/recovery/v1";
    const TAG: u8 = 0;
    const NAME: &'static str = "recovery";
}

impl Clearance for Account {
    const CONTEXT: &'static [u8] = b"tonk/kek/account/v1";
    const TAG: u8 = 1;
    const NAME: &'static str = "account";
}

mod private {
    pub trait Sealed {}
    impl Sealed for super::Recovery {}
    impl Sealed for super::Account {}
}
