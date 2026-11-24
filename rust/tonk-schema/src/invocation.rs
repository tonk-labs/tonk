// The `#[derive(Concept)]` and `#[derive(Attribute)]` macros generate
// helper types without doc comments; suppress `missing_docs` here.
#![allow(missing_docs)]
// `#[derive(Attribute)]` names a constructor after the attribute, so
// `Of`'s is `of`, its own name lowercased.
#![allow(clippy::self_named_constructors)]

//! Invocations: what became of a command someone ran.
//!
//! A command lands in its stack's `control:` layer for one tick, and dialog
//! writes each change in its life there as a `dialog.command/*` row of the
//! command's entity, again for one tick: `invoked` when it lands, then
//! `answered`, `failed` or `unhandled` once the providers ran, and
//! `progress` whenever a provider reports some. Nothing keeps them.
//!
//! An invoker that wants to follow a command stamps it with
//! [`command::InvokedAt`] (when it was run) and [`command::Concept`] (which
//! command it is), and gives it an entity of its own. Rules in `core.yaml`
//! and `profile.yaml` (`invocation` and the concepts beside it) then copy
//! that command's lifecycle rows into the stack's `session:` layer, where
//! the invocation stays for the session: [`Invoked`], [`Of`], and in time
//! [`Answered`], [`Failed`] or [`Unhandled`], with the [`State`] its
//! provider last reported. A provider's [`Done`] output is written there
//! too, as [`Output`]. Nothing about an invocation is committed or
//! replicated; it goes with the session.
//!
//! An untracked command (a keystroke's `intent/interpret`, a page's
//! `tonk:load`) leaves nothing behind: the rules and the provider wrapper
//! act only on stamped commands.

use dialog_corpus::Entity;
use dialog_query::Attribute;

/// What the invoker stamps on the command it runs, beside the command's
/// own fields, in `control:`.
pub mod command {
    use super::{Attribute, Entity};

    /// When the command was run, in milliseconds since the epoch, by the
    /// invoker's clock. Its presence is what asks for the command to be
    /// followed.
    #[derive(Attribute, Clone, PartialEq, PartialOrd)]
    #[domain("xyz.tonk.command")]
    #[pick(last)]
    pub struct InvokedAt(pub f64);

    /// The concept of the command that was run, so a view can render the
    /// invocation through that command's own `status` view.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.command")]
    #[pick(last)]
    pub struct Concept(pub Entity);
}

/// When the followed command was run, copied from its
/// [`command::InvokedAt`]: what makes an entity of `session:` an
/// invocation.
#[derive(Attribute, Clone, PartialEq, PartialOrd)]
#[domain("xyz.tonk.invocation")]
#[pick(last)]
pub struct Invoked(pub f64);

/// The concept of the command, copied from its [`command::Concept`].
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("xyz.tonk.invocation")]
#[pick(last)]
pub struct Of(pub Entity);

/// The concept whose provider answered the command, copied from
/// `dialog.command/answered`.
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("xyz.tonk.invocation")]
#[pick(last)]
pub struct Answered(pub Entity);

/// Why the command failed, copied from `dialog.command/failed`.
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("xyz.tonk.invocation")]
#[pick(last)]
pub struct Failed(pub String);

/// That no provider claimed the command, copied from
/// `dialog.command/unhandled`. A command a rule handled at commit time is
/// unhandled too, as dialog sees it, and has an [`Output`] its rule wrote.
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("xyz.tonk.invocation")]
#[pick(last)]
pub struct Unhandled(pub bool);

/// What the command is doing, as its provider last reported it, copied
/// from `dialog.command/progress`: an entity with a [`Status`].
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("xyz.tonk.invocation")]
#[pick(last)]
pub struct State(pub Entity);

/// What the command produced: the entity it made or acted on, written by
/// its provider's [`Done`] or by the rule that handled it.
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("xyz.tonk.invocation")]
#[pick(last)]
pub struct Output(pub Entity);

/// The words for a [`State`]: what a generic view shows when the
/// command's own view says nothing.
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("xyz.tonk.state")]
#[pick(last)]
pub struct Status(pub String);

/// The words for a [`State`] as its provider reported it, beside the
/// command's progress in `control:` for one tick; a rule keeps them as the
/// state's [`Status`].
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("xyz.tonk.state")]
#[pick(last)]
pub struct Reported(pub String);

/// Whether the command whose rows are `rows` is followed: whether its
/// invoker stamped it with [`command::InvokedAt`].
pub fn followed<'a>(rows: impl IntoIterator<Item = &'a dialog_corpus::Fact>) -> bool {
    let stamp = command::InvokedAt::the().to_string();
    rows.into_iter().any(|row| row.the.to_string() == stamp)
}

/// What a command's provider says came of it.
///
/// Every command's [`dialog_capability::Command::Output`] is this, so the
/// dispatcher can record success and failure without asking each provider
/// how it reports.
pub type Outcome = Result<Done, Failure>;

/// A command that succeeded, and what it produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Done {
    /// The entity the command made or acted on, when there is one worth
    /// showing (the new space, the renamed notebook).
    pub output: Option<Entity>,
}

impl Done {
    /// Done, with nothing to show.
    pub fn nothing() -> Outcome {
        Ok(Self::default())
    }

    /// Done, producing `output`.
    pub fn with(output: Entity) -> Outcome {
        Ok(Self {
            output: Some(output),
        })
    }
}

/// Why a command failed, in words the person can be shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure(pub String);

impl Failure {
    /// A failure saying `message`.
    pub fn new(message: impl std::fmt::Display) -> Self {
        Self(message.to_string())
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self(message)
    }
}

impl From<&str> for Failure {
    fn from(message: &str) -> Self {
        Self(message.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn it_names_the_attributes_the_yaml_declares() {
        assert_eq!(
            command::InvokedAt::the().to_string(),
            "xyz.tonk.command/invoked-at"
        );
        assert_eq!(
            command::Concept::the().to_string(),
            "xyz.tonk.command/concept"
        );
        assert_eq!(Invoked::the().to_string(), "xyz.tonk.invocation/invoked");
        assert_eq!(Of::the().to_string(), "xyz.tonk.invocation/of");
        assert_eq!(Answered::the().to_string(), "xyz.tonk.invocation/answered");
        assert_eq!(Failed::the().to_string(), "xyz.tonk.invocation/failed");
        assert_eq!(
            Unhandled::the().to_string(),
            "xyz.tonk.invocation/unhandled"
        );
        assert_eq!(State::the().to_string(), "xyz.tonk.invocation/state");
        assert_eq!(Output::the().to_string(), "xyz.tonk.invocation/output");
        assert_eq!(Status::the().to_string(), "xyz.tonk.state/status");
        assert_eq!(Reported::the().to_string(), "xyz.tonk.state/reported");
    }

    /// The invocation block of a library document, between its markers.
    fn block(document: &str) -> &str {
        let start = document
            .find("# invocation-schema:begin")
            .expect("the document declares invocations");
        let end = document
            .find("# invocation-schema:end")
            .expect("the block is closed");
        &document[start..end]
    }

    const CORE: &str = include_str!("../../tonk-core/assets/library/core.yaml");
    const PROFILE: &str = include_str!("../../tonk-core/assets/library/profile.yaml");

    #[dialog_common::test]
    fn the_space_and_the_profile_declare_the_same_invocations() {
        assert_eq!(
            block(CORE),
            block(PROFILE),
            "a command lands on whichever branch it ran on, so core.yaml and \
             profile.yaml follow it alike"
        );
    }

    #[dialog_common::test]
    fn the_library_declares_every_attribute_named_here() {
        let declared = block(CORE);
        for the in [
            command::InvokedAt::the().to_string(),
            command::Concept::the().to_string(),
            Invoked::the().to_string(),
            Of::the().to_string(),
            Answered::the().to_string(),
            Failed::the().to_string(),
            Unhandled::the().to_string(),
            State::the().to_string(),
            Output::the().to_string(),
            Status::the().to_string(),
            Reported::the().to_string(),
        ] {
            assert!(
                declared.contains(&format!("the: {the}\n")),
                "{the} is named here but not declared in core.yaml"
            );
        }
    }

    #[dialog_common::test]
    fn a_command_is_followed_only_when_stamped() {
        let of: Entity = "urn:uuid:run".parse().unwrap();
        let fact = |the: &str, is: dialog_corpus::Value| dialog_corpus::Fact {
            the: the.parse().unwrap(),
            of: of.clone(),
            is,
            cause: None,
        };
        let stamped = [
            fact(
                "xyz.tonk.command.rename/name",
                dialog_corpus::Value::String("Plans".into()),
            ),
            fact(
                "xyz.tonk.command/invoked-at",
                dialog_corpus::Value::Float(1.0),
            ),
        ];
        assert!(followed(&stamped));
        assert!(
            !followed(&stamped[..1]),
            "a command nobody stamped is not followed"
        );
    }

    #[dialog_common::test]
    fn a_failure_says_what_it_was_given() {
        let failure: Failure = "nope".into();
        assert_eq!(failure.to_string(), "nope");
        assert_eq!(Failure::new(42).0, "42");
        assert_eq!(Done::nothing(), Ok(Done { output: None }));
    }
}
