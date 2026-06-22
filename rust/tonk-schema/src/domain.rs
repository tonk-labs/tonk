//! Per-concept attribute types in the `xyz.tonk.*` sub-domains.
//!
//! Each concept owns its own attribute namespace
//! (`xyz.tonk.replica`, `xyz.tonk.branch`, `xyz.tonk.remote`) so
//! its descriptor never matches entities of another shape — a
//! `Branch:` query would otherwise return [`Remote`] entities
//! since both have a `name` and an `origin` claim under the
//! shared `xyz.tonk` namespace.
//!
//! [`TrackingBranch`] reuses the `xyz.tonk.branch` namespace
//! because a tracking branch *is* a local branch with one extra
//! relation; its entities should still surface in a `branch:`
//! query.
//!
//! [`Remote`]: crate::Remote
//! [`TrackingBranch`]: crate::TrackingBranch

// The `#[derive(Attribute)]` macro generates helper types and
// associated functions without doc comments. Suppress the
// crate-level `missing_docs` lint for this module so the macros
// compile under `-D warnings`.
#![allow(missing_docs)]

use dialog_artifacts::Entity;
use dialog_query::Attribute;
use dialog_repository::SiteAddress;

/// Attributes that live on [`Replica`] entities only.
///
/// [`Replica`]: crate::Replica
pub mod replica {
    use super::{Attribute, Entity};

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.replica")]
    pub struct Name(pub String);

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.replica")]
    pub struct Subject(pub Entity);

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.replica")]
    pub struct Profile(pub Entity);

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.replica")]
    pub struct Kind(pub Entity);

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.replica")]
    pub struct Status(pub Entity);
}

/// Attributes for transient *command* concepts — the effect triggers
/// dispatched to typed-Rust handlers after a commit. A command is a
/// plain concept marked transient; these are the fields its triggers
/// carry.
pub mod command {
    use super::Attribute;

    /// The space name read from the Add Space form's submit event:
    /// `event.currentTarget.elements.name.value` (the `<wa-input
    /// name="name">` inside the `<form onsubmit=space/create>`).
    ///
    /// The `the:` is a `dom.event.*` read-path so the notation event
    /// layer populates it from the form on submit. The handler decodes
    /// the command by this same attribute — form source and handler
    /// decode agree on one attribute. Written kebab-case
    /// (`current-target`); the event layer converts to `currentTarget`
    /// at read time. The struct is named `Value` because the attribute
    /// is `…elements.name/value` (domain + `/value`).
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("dom.event.current-target.elements.name")]
    pub struct Value(pub String);

    /// The remote URL read from a space form's submit event:
    /// `event.currentTarget.elements.remote.value` (the `<wa-input
    /// name="remote">` inside the create / enable-sync forms).
    ///
    /// Single word `remote` (not `remote-url`) deliberately: every
    /// path segment is kebab→camel-cased at read time, so a hyphen
    /// would force the input's `name` to be `remoteUrl`. Keeping it one
    /// word lets the form field and the read-path agree on `remote`.
    ///
    /// Optional in practice: an empty input coerces to `""` (the input
    /// element still resolves, so the field is never *unresolved*), and
    /// the handler reads `""` as "no remote — local-only".
    pub mod remote {
        use super::Attribute;

        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("dom.event.current-target.elements.remote")]
        pub struct Value(pub String);
    }
}

/// Attributes that live on a repository's own `tonk/repository`
/// concept — the repository's self-describing name, stored on its
/// content branch and keyed by the subject DID.
///
/// [`RepositoryName`]: crate::RepositoryName
pub mod repo {
    use super::Attribute;

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.repo")]
    pub struct Name(pub String);
}

/// Attributes that live on [`Branch`] entities (and
/// [`TrackingBranch`], which extends `Branch`).
///
/// [`Branch`]: crate::Branch
/// [`TrackingBranch`]: crate::TrackingBranch
pub mod branch {
    use super::{Attribute, Entity};

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.branch")]
    pub struct Name(pub String);

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.branch")]
    pub struct Origin(pub Entity);

    /// The upstream branch a local branch is tracking. Direction-
    /// explicit counterpart to [`Origin`]: asserting
    /// `local -upstream-> remote_branch` records that the local
    /// branch tracks the remote branch.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.branch")]
    pub struct Upstream(pub Entity);
}

/// Attributes that live on [`Membership`] entities only.
///
/// [`Membership`]: crate::Membership
pub mod membership {
    use super::{Attribute, Entity};

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.membership")]
    pub struct Subject(pub Entity);

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.membership")]
    pub struct Member(pub Entity);

    /// The invitation a membership was claimed through — the
    /// [`InvitedVia`] stamp's payload.
    ///
    /// [`InvitedVia`]: crate::InvitedVia
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.membership")]
    pub struct Invitation(pub Entity);
}

/// Attributes that live on [`Invitation`] entities only.
///
/// [`Invitation`]: crate::Invitation
pub mod invitation {
    use super::{Attribute, Entity};

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.invitation")]
    pub struct Subject(pub Entity);

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.invitation")]
    pub struct Inviter(pub Entity);

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.invitation")]
    pub struct Audience(pub Entity);
}

/// Attributes that live on [`Remote`] entities only.
///
/// [`Remote`]: crate::Remote
pub mod remote {
    use super::{Attribute, Entity, SiteAddress};

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.remote")]
    pub struct Name(pub String);

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.remote")]
    pub struct Origin(pub Entity);

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.remote")]
    pub struct Subject(pub Entity);

    /// Serialized [`SiteAddress`] bytes — the opaque payload a
    /// remote uses to locate a peer.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.remote")]
    pub struct Address(pub Vec<u8>);

    impl Address {
        /// Encode a [`SiteAddress`] as dag-cbor bytes.
        pub fn encode(address: &SiteAddress) -> Self {
            let bytes = serde_ipld_dagcbor::to_vec(address)
                .expect("SiteAddress is serde-serializable and dag-cbor-compatible");
            Self(bytes)
        }

        /// Decode the stored dag-cbor bytes back into a
        /// [`SiteAddress`].
        pub fn decode(
            &self,
        ) -> Result<SiteAddress, serde_ipld_dagcbor::DecodeError<std::convert::Infallible>>
        {
            serde_ipld_dagcbor::from_slice(&self.0)
        }
    }
}
