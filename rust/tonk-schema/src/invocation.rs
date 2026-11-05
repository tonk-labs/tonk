// The `#[derive(Concept)]` and `#[derive(Attribute)]` macros generate
// helper types without doc comments; suppress `missing_docs` here.
#![allow(missing_docs)]

//! Invocations: what became of a command someone ran.
//!
//! A command is a transient, so it is gone once its commit lands. An
//! invoker that wants to know how it went stamps the command with
//! [`command::InvokedAt`] (the moment it was run, which also keeps two runs of the
//! same command apart) and the [`command::Concept`] it is an
//! instance of. The worker then keeps an *invocation* on the command's own
//! entity, in the session overlay of the branch the command was committed
//! on: the command's fields as they were given, a [`case`] saying where it
//! got to, a [`State`] a provider reports while it works, and an
//! [`Output`] once it is done. Nothing about an invocation is ever
//! committed or replicated, and it is retracted once it [`Expires`].
//!
//! The vocabulary is declared in `core.yaml` and `profile.yaml`
//! (`invocation`, `state`, `command/completion`, `command/retention`);
//! these types follow it.

use dialog_artifacts::Entity;
use dialog_query::{Attribute, Concept};

/// What the invoker stamps on the command it runs.
pub mod command {
    use super::{Attribute, Entity};

    /// When the command was run, in milliseconds since the epoch, by the
    /// invoker's clock. Stamped by the invoker on the command entity; its
    /// presence is what asks the worker to keep an invocation.
    #[derive(Attribute, Clone, PartialEq, PartialOrd)]
    #[domain("xyz.tonk.command")]
    #[pick(last)]
    pub struct InvokedAt(pub f64);

    /// The concept of the command that was run, so a view can render the
    /// invocation through that command's own `status` view. Stamped by the
    /// invoker, which knows what it ran; the worker only knows Rust types.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.command")]
    #[pick(last)]
    pub struct Concept(pub Entity);
}

/// Where the invocation got to: one of the [`case`] entities.
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("xyz.tonk.invocation")]
#[pick(last)]
pub struct Case(pub Entity);

/// What the invocation is doing, as the provider last reported it. An
/// entity that has a [`Status`].
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("xyz.tonk.invocation")]
#[pick(last)]
pub struct State(pub Entity);

/// What the invocation produced: the entity a successful command made or
/// acted on, or, for a failed one, an error entity whose [`Status`] says
/// why.
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("xyz.tonk.invocation")]
#[pick(last)]
pub struct Output(pub Entity);

/// When the worker started the command, in milliseconds since the epoch.
#[derive(Attribute, Clone, PartialEq, PartialOrd)]
#[domain("xyz.tonk.invocation")]
#[pick(last)]
pub struct Started(pub f64);

/// When the command finished, in milliseconds since the epoch.
#[derive(Attribute, Clone, PartialEq, PartialOrd)]
#[domain("xyz.tonk.invocation")]
#[pick(last)]
pub struct Finished(pub f64);

/// When the invocation is retracted from the overlay, in milliseconds
/// since the epoch. Set once it has finished.
#[derive(Attribute, Clone, PartialEq, PartialOrd)]
#[domain("xyz.tonk.invocation")]
#[pick(last)]
pub struct Expires(pub f64);

/// The text that says what a state or an error is. Every [`State`] and
/// every error [`Output`] has one, so a generic view can always render
/// something.
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("xyz.tonk.state")]
#[pick(last)]
pub struct Status(pub String);

/// An invocation as the overlay holds it: where it got to and when it
/// started. Its optional parts ([`State`], [`Output`], [`Finished`],
/// [`Expires`]) are read and written attribute by attribute.
#[derive(Concept, Debug, Clone, PartialEq, PartialOrd)]
pub struct Invocation {
    /// The command entity.
    pub this: Entity,
    /// One of the [`case`] entities.
    pub case: Case,
    /// When the worker started it.
    pub started: Started,
}

/// A state or error entity and the text that says what it is.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct StateStatus {
    /// The state entity.
    pub this: Entity,
    /// What it says.
    pub status: Status,
}

/// The `case:` entities an invocation's [`Case`] names.
pub mod case {
    use dialog_artifacts::Entity;

    /// A provider is running the command.
    pub const ACTIVE: &str = "case:active";
    /// The command did what it does.
    pub const DONE: &str = "case:done";
    /// The command ran and failed; its output says why.
    pub const FAILED: &str = "case:failed";
    /// The worker running the command went away before it finished.
    pub const ABORTED: &str = "case:aborted";
    /// Nothing handled the command: no provider matched it, and no rule
    /// concluded a completion for it.
    pub const UNHANDLED: &str = "case:unhandled";

    /// Every case, for readers that list them.
    pub const ALL: [&str; 5] = [ACTIVE, DONE, FAILED, ABORTED, UNHANDLED];

    /// The entity for `case`.
    pub fn entity(case: &str) -> Entity {
        case.parse().expect("a case is a valid entity URI")
    }

    /// Whether `case` is one an invocation stays in for good.
    pub fn is_final(case: &str) -> bool {
        case != ACTIVE
    }
}

/// What a rule concludes about a command it handled at commit time.
///
/// A command with no provider is handled, if at all, by a rule that
/// fires on it inside the commit. Such a rule says so by concluding a
/// `command/completion` on the command entity, naming what it produced.
/// The concept is transient: the conclusion is read off the commit and
/// never stored.
pub mod completion {
    use super::{Attribute, Entity};

    /// What the rule-handled command produced.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.command.completion")]
    #[pick(last)]
    pub struct Output(pub Entity);
}

/// How long finished invocations are kept: the account's
/// `command/retention`, or these defaults.
pub mod retention {
    use super::Attribute;

    /// A successful invocation, by default: long enough for the palette
    /// to show it.
    pub const DONE_SECONDS: f64 = 30.0;
    /// A failed, aborted or unhandled one, by default: long enough to go
    /// and find it.
    pub const FAILED_SECONDS: f64 = 600.0;

    /// How long a successful invocation is kept after it finishes, in
    /// seconds.
    #[derive(Attribute, Clone, PartialEq, PartialOrd)]
    #[domain("xyz.tonk.command.retention")]
    #[pick(last)]
    pub struct Done(pub f64);

    /// How long a failed, aborted or unhandled invocation is kept after
    /// it finishes, in seconds.
    #[derive(Attribute, Clone, PartialEq, PartialOrd)]
    #[domain("xyz.tonk.command.retention")]
    #[pick(last)]
    pub struct Failed(pub f64);
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
        assert_eq!(Case::the().to_string(), "xyz.tonk.invocation/case");
        assert_eq!(State::the().to_string(), "xyz.tonk.invocation/state");
        assert_eq!(Output::the().to_string(), "xyz.tonk.invocation/output");
        assert_eq!(Started::the().to_string(), "xyz.tonk.invocation/started");
        assert_eq!(Finished::the().to_string(), "xyz.tonk.invocation/finished");
        assert_eq!(Expires::the().to_string(), "xyz.tonk.invocation/expires");
        assert_eq!(Status::the().to_string(), "xyz.tonk.state/status");
        assert_eq!(
            completion::Output::the().to_string(),
            "xyz.tonk.command.completion/output"
        );
        assert_eq!(
            retention::Done::the().to_string(),
            "xyz.tonk.command.retention/done"
        );
        assert_eq!(
            retention::Failed::the().to_string(),
            "xyz.tonk.command.retention/failed"
        );
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
            "an invocation lives on whichever branch its command ran on, so \
             core.yaml and profile.yaml declare it alike"
        );
    }

    #[dialog_common::test]
    fn the_library_declares_every_attribute_written_here() {
        let declared = block(CORE);
        for the in [
            command::InvokedAt::the().to_string(),
            command::Concept::the().to_string(),
            Case::the().to_string(),
            State::the().to_string(),
            Output::the().to_string(),
            Started::the().to_string(),
            Finished::the().to_string(),
            Expires::the().to_string(),
            Status::the().to_string(),
            completion::Output::the().to_string(),
        ] {
            assert!(
                declared.contains(&format!("the: {the}\n")),
                "{the} is written by the worker but not declared in core.yaml"
            );
        }
        for the in [
            retention::Done::the().to_string(),
            retention::Failed::the().to_string(),
        ] {
            assert!(
                PROFILE.contains(&format!("the: {the}\n")),
                "{the} is read by the worker but not declared in profile.yaml"
            );
        }
    }

    #[dialog_common::test]
    fn every_case_but_active_is_final() {
        for name in case::ALL {
            assert_eq!(case::is_final(name), name != case::ACTIVE, "{name}");
            let _ = case::entity(name);
        }
    }

    #[dialog_common::test]
    fn a_failure_says_what_it_was_given() {
        let failure: Failure = "nope".into();
        assert_eq!(failure.to_string(), "nope");
        assert_eq!(Failure::new(42).0, "42");
        assert_eq!(Done::nothing(), Ok(Done { output: None }));
    }
}
