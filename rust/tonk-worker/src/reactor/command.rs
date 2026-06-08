//! Command handlers — the typed-Rust sibling of the declarative
//! `rule!:` effects in [`tonk_evaluator::effects`].
//!
//! A *command* is a transient [`Concept`] asserted to trigger an
//! effect. The induce loop already dispatches transients to
//! declarative rules (body query → head assertion); commands extend
//! that to effects that run arbitrary `async` Rust — repo creation,
//! key generation, network IO — which a rule body can't express.
//!
//! The shape is Bevy-ECS-flavoured: a handler is an `async fn` whose
//! parameters declare what it reads (`&Cmd` = the transient trigger,
//! `Current<C>` = durable state joined on `this`) and a `Transaction`
//! it writes outcomes into. See `project_effect_command_design` and
//! the journal entry `@gozala/2026-06-05.md` for the full rationale.
//!
//! This module is the registry + dispatch seam. The typed handler
//! and its trigger concept are captured behind a dyn-safe
//! [`CommandHandler`] at registration: the registry only ever sees
//! "which attribute names trigger this" and "run it against a matched
//! transient set", so dispatch stays dynamic while decode stays
//! statically typed inside the captured closure.

use std::collections::HashMap;

use dialog_artifacts::{Artifact, Changes, Entity, Instruction};
use dialog_capability::{Command, Provider};
use dialog_common::ConditionalSync;
use dialog_query::concept::Concept;
use dialog_query::{Application, ConceptDescriptor, Conclusion, Match, Term};

/// The asserted facts for a single transient entity — the `(the, of,
/// is)` triples grouped under one `of`. [`Artifact`] is `Clone`
/// (unlike [`Instruction`]), so a matched entity's facts can be
/// handed to several handlers and decoded independently.
pub type EntityFacts = Vec<Artifact>;

/// The dynamic→static bridge: decode a typed concept `C` from the raw
/// `(the, of, is)` facts of one transient entity, reusing the derived
/// query machinery rather than reimplementing field decode.
///
/// `Query::<C>::default()` (derive-provided) is the all-variables
/// query: `this` and every field bound to `Term::var(<field>)`. We
/// build a [`Match`] binding each of those variables to the value the
/// facts carry for the field's attribute, then call the derived
/// `realize` — the same decode read-subscriptions use. A missing or
/// type-mismatched required field makes `realize` fail, which is the
/// natural "this concept doesn't match these facts" signal.
///
/// The transient is gone from the durable tree by dispatch time (the
/// induce sweep cancelled it at commit), so `&C` MUST decode from the
/// facts in hand — it can't be re-queried. This is that decode.
pub fn decode_concept<C>(this: Entity, facts: &EntityFacts) -> Option<C>
where
    C: Concept<Conclusion = C> + Conclusion,
    C::Application: Default + Application<Conclusion = C>,
    ConceptDescriptor: From<C::Application>,
{
    let query = C::Application::default();

    // field name → fully-qualified attribute name (`domain/name`),
    // from the concept's descriptor.
    let descriptor = ConceptDescriptor::from(C::Application::default());
    let attribute_to_field: HashMap<String, String> = descriptor
        .with()
        .iter()
        .map(|(field, attribute)| (attribute.the().to_string(), field.to_string()))
        .collect();

    let mut source = Match::new();
    // `this` is the entity itself.
    source.bind(&Term::var("this"), this.into()).ok()?;
    // Each fact binds its field's variable. Facts whose attribute
    // isn't part of `C` are ignored (the entity may carry unrelated
    // attributes); fields with no matching fact stay unbound, so
    // `realize` fails for a required field — the right "no match".
    for artifact in facts {
        if let Some(field) = attribute_to_field.get(&artifact.the.to_string()) {
            source.bind(&Term::var(field), artifact.is.clone()).ok()?;
        }
    }

    query.realize(source).ok()
}

/// A transient trigger concept: the command type a [`Provider`] runs.
/// Supplies the reverse-index keys (its field attribute names) and a
/// decode from a transient entity's facts.
///
/// Blanket-implemented for any decodable [`Concept`]; the bounds are
/// exactly what [`decode_concept`] needs, so a plain
/// `#[derive(Concept)]` is `Decode` with no extra code. (Named `Decode`
/// rather than `Command` to leave that name to
/// [`dialog_capability::Command`], which a command type also implements.)
///
/// [`Provider`]: dialog_capability::Provider
pub trait Decode: Sized {
    /// Fully-qualified attribute URIs (`domain/name`) of this
    /// trigger's fields. An entity in the transient set is a
    /// candidate when it carries any of these.
    fn trigger_attributes() -> Vec<String>;

    /// Decode this trigger from one transient entity's facts, or
    /// `None` if they don't satisfy it (missing/mistyped field).
    fn decode(this: Entity, facts: &EntityFacts) -> Option<Self>;
}

impl<C> Decode for C
where
    C: Concept<Conclusion = C> + Conclusion,
    C::Application: Default + Application<Conclusion = C>,
    ConceptDescriptor: From<C::Application>,
{
    fn trigger_attributes() -> Vec<String> {
        ConceptDescriptor::from(C::Application::default())
            .with()
            .iter()
            .map(|(_, attribute)| attribute.the().to_string())
            .collect()
    }

    fn decode(this: Entity, facts: &EntityFacts) -> Option<Self> {
        decode_concept::<C>(this, facts)
    }
}

/// The environment a command runs against — the [`Provider`] the
/// dispatcher calls `execute` on. Supplied at dispatch time (the
/// registry can't bake it in: it lives *inside* the state, an `Arc`
/// cycle). In practice the worker's `CommandEnv` wrapping `AppState`;
/// the alias keeps this module from naming the worker type directly.
///
/// [`Provider`]: dialog_capability::Provider
pub type Env = crate::router::CommandEnv;

/// A `'static` boxed future for one command's execution, `Send` only
/// off wasm — matches the reactor's
/// [`ConditionalSync`](dialog_common::ConditionalSync) convention so a
/// command runs on the single-threaded SW executor. `'static` so the
/// dispatcher can build it, release the state lock, then await it —
/// command IO never runs while a lock is held.
#[cfg(not(target_arch = "wasm32"))]
pub type RunFuture = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>>;
#[cfg(target_arch = "wasm32")]
pub type RunFuture = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + 'static>>;

/// Dyn-safe entry stored in the [`CommandRegistry`]. One per registered
/// command *type*. The concrete command `C` is erased behind this
/// object; the registry interacts only through the methods here.
///
/// `ConditionalSync` (Send + Sync off wasm, nothing on wasm) keeps a
/// `Box<dyn CommandHandler>` — and therefore [`TonkState`] /
/// [`AppState`] — `Send + Sync` on native, which axum requires.
///
/// [`TonkState`]: crate::worker::TonkState
/// [`AppState`]: crate::router::AppState
pub trait CommandHandler: ConditionalSync {
    /// Attribute names whose presence on a transient entity makes this
    /// command a candidate. Drives the reverse index.
    fn trigger_attributes(&self) -> &[String];

    /// Whether `facts` (the asserted artifacts for a single transient
    /// entity) decode as this command.
    fn matches(&self, facts: &EntityFacts) -> bool;

    /// Decode the command from `facts` and execute it via the
    /// [`Provider`](dialog_capability::Provider) impl on a clone of
    /// `env`. The returned future owns the decoded command and the env
    /// clone, so it is `'static` and can run detached past the
    /// triggering commit — the dispatcher releases the state lock
    /// before awaiting it. A no-op future when the facts don't decode.
    /// Boxed so the trait stays dyn-safe for `Box<dyn CommandHandler>`.
    fn run(&self, facts: &EntityFacts, env: &Env) -> RunFuture;
}

/// The `of` (entity) shared by a transient entity's facts. Every
/// artifact in an [`EntityFacts`] carries the same `of` (they were
/// grouped by it), so the first one names the entity.
fn facts_entity(facts: &EntityFacts) -> Option<Entity> {
    facts.first().map(|artifact| artifact.of.clone())
}

/// A registry entry for the command type `C`. Holds no handler — the
/// behaviour is the [`Provider<C>`](dialog_capability::Provider) impl on
/// [`Env`]. Erases `C` behind [`CommandHandler`]: `matches` is the
/// derived decode of `C`, `trigger_attributes` comes off `C`'s
/// descriptor, and `run` decodes then calls `Env::execute`.
///
/// A command is registrable iff `Env: Provider<C>` — i.e. the env has
/// the capability to run it. That bound on [`Self::new`] is the
/// (compile-time) capability gate; the UCAN-style runtime gate layers on
/// top of it later.
pub struct TypedCommand<C> {
    attributes: Vec<String>,
    _command: std::marker::PhantomData<fn() -> C>,
}

impl<C> TypedCommand<C>
where
    C: Decode + Command<Input = C> + 'static,
    Env: Provider<C>,
{
    /// Register command `C`, caching its trigger attribute names. The
    /// `Env: Provider<C>` bound means a command can only be registered
    /// if the env can execute it.
    pub fn new() -> Self {
        Self {
            attributes: C::trigger_attributes(),
            _command: std::marker::PhantomData,
        }
    }
}

impl<C> Default for TypedCommand<C>
where
    C: Decode + Command<Input = C> + 'static,
    Env: Provider<C>,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<C> CommandHandler for TypedCommand<C>
where
    C: Decode + Command<Input = C, Output = ()> + ConditionalSync + 'static,
    Env: Provider<C> + Clone + ConditionalSync + 'static,
{
    fn trigger_attributes(&self) -> &[String] {
        &self.attributes
    }

    fn matches(&self, facts: &EntityFacts) -> bool {
        let Some(this) = facts_entity(facts) else {
            return false;
        };
        C::decode(this, facts).is_some()
    }

    fn run(&self, facts: &EntityFacts, env: &Env) -> RunFuture {
        // Decode synchronously here (the caller still holds the lock),
        // then hand the owned command + an env clone to a `'static`
        // future so the dispatcher can drop the lock before awaiting.
        let decoded = facts_entity(facts).and_then(|this| C::decode(this, facts));
        let env = env.clone();
        Box::pin(async move {
            if let Some(command) = decoded {
                env.execute(command).await;
            }
        })
    }
}

/// Registry of command handlers, with a reverse index from trigger
/// attribute name to the handlers it can fire. Mirrors the
/// `dialog.effect/on` index the induce loop walks, but over
/// registered Rust handlers instead of installed `rule!:` effects.
#[derive(Default)]
pub struct CommandRegistry {
    /// Registered handlers, owned. Indices into this vec are the
    /// values in [`Self::by_attribute`].
    handlers: Vec<Box<dyn CommandHandler>>,
    /// `attribute name → handler indices`. A transient touching this
    /// attribute makes every listed handler a candidate.
    by_attribute: HashMap<String, Vec<usize>>,
}

impl CommandRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register the command type `C`. Lighter than passing a handler:
    /// the behaviour is the [`Provider<C>`](dialog_capability::Provider)
    /// impl on [`Env`], so registration is just the type. The
    /// `Env: Provider<C>` bound means a command can only be registered if
    /// the env has the capability to run it. Chainable.
    ///
    /// ```ignore
    /// let registry = CommandRegistry::new()
    ///     .command::<CreateSpace>()   // Env: Provider<CreateSpace>
    ///     .command::<RenameSpace>();
    /// ```
    pub fn command<C>(mut self) -> Self
    where
        C: Decode + Command<Input = C, Output = ()> + ConditionalSync + 'static,
        Env: Provider<C> + Clone + ConditionalSync + 'static,
    {
        self.register(Box::new(TypedCommand::<C>::new()));
        self
    }

    /// Register a boxed handler, indexing it by each of its trigger
    /// attribute names. Prefer [`Self::command`] for typed handlers.
    pub fn register(&mut self, handler: Box<dyn CommandHandler>) {
        let index = self.handlers.len();
        for name in handler.trigger_attributes() {
            self.by_attribute
                .entry(name.clone())
                .or_default()
                .push(index);
        }
        self.handlers.push(handler);
    }

    /// `true` when no handlers are registered — lets the reactor skip
    /// the whole dispatch pass (group-by-entity, candidate lookup)
    /// when commands aren't in use.
    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }

    /// For a committed transient batch, find every `(handler, entity
    /// facts)` pair that should fire: group the transients by entity,
    /// look up candidate handlers via the touched attributes, and
    /// keep those whose trigger concept actually decodes.
    ///
    /// Returns references into `self` paired with a clone of the
    /// matched entity's facts; the caller (step 4) decodes + spawns
    /// each.
    pub fn match_transients<'a>(
        &'a self,
        transients: &Changes,
    ) -> Vec<(&'a dyn CommandHandler, EntityFacts)> {
        let by_entity = group_by_entity(transients.clone());

        let mut fired = Vec::new();
        for (_entity, facts) in by_entity {
            // Candidate handlers: any indexed under an attribute this
            // entity touches. Dedup so a handler keyed on two of the
            // entity's attributes is considered once.
            let mut candidates: Vec<usize> = facts
                .iter()
                .filter_map(|artifact| self.by_attribute.get(&artifact.the.to_string()))
                .flatten()
                .copied()
                .collect();
            candidates.sort_unstable();
            candidates.dedup();

            for index in candidates {
                let handler = self.handlers[index].as_ref();
                // All matches fire (commands are subscription-like),
                // so no tiebreak — every handler whose trigger decodes
                // gets its own fact slice.
                if handler.matches(&facts) {
                    fired.push((handler, facts.clone()));
                }
            }
        }
        fired
    }
}

/// Group a transient [`Changes`] batch into per-entity [`Artifact`]
/// lists so each candidate entity is matched/decoded as a unit.
/// Polarity is dropped — a transient command is asserted, and the
/// decoder reads `(the, of, is)`, which every [`Instruction`] variant
/// carries.
fn group_by_entity(changes: Changes) -> HashMap<dialog_artifacts::Entity, EntityFacts> {
    let mut by_entity: HashMap<dialog_artifacts::Entity, EntityFacts> = HashMap::new();
    for instruction in changes.into_instructions() {
        let artifact = match instruction {
            Instruction::Assert(a) | Instruction::Replace(a) | Instruction::Retract(a) => a,
        };
        by_entity
            .entry(artifact.of.clone())
            .or_default()
            .push(artifact);
    }
    by_entity
}

#[cfg(test)]
mod tests {
    #![allow(missing_docs)]

    use super::*;
    use dialog_artifacts::Statement;
    use dialog_query::{Attribute, Concept, the};

    // A command concept whose `Provider` lives at the router layer; here
    // we test only the decode + match machinery, which needs no env.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.command")]
    pub struct RepoName(pub String);

    #[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
    pub struct CreateRepo {
        pub this: Entity,
        pub name: RepoName,
    }

    /// A one-fact transient `CreateRepo{this: of, name}` batch. The
    /// attribute is `RepoName`'s name (snake-cased from its struct name),
    /// which is what the reverse index keys on.
    fn create_repo_transient(of: &str, name: &str) -> Changes {
        let mut changes = Changes::new();
        the!("xyz.tonk.command/repo-name")
            .of(of.parse::<Entity>().expect("entity URI"))
            .is(name.to_string())
            .assert(&mut changes);
        changes
    }

    #[dialog_common::test]
    fn it_decodes_a_command_from_its_transient_facts() {
        let changes = create_repo_transient("did:key:zCmd", "pictures");
        let by_entity = group_by_entity(changes);
        let (entity, facts) = by_entity.into_iter().next().expect("one entity");

        let decoded = CreateRepo::decode(entity.clone(), &facts).expect("decodes");
        assert_eq!(decoded.this, entity);
        assert_eq!(decoded.name.0, "pictures");
    }

    #[dialog_common::test]
    fn it_does_not_decode_when_a_required_field_is_absent() {
        // An entity carrying an unrelated attribute (not `name`)
        // shouldn't decode as a CreateRepo.
        let mut changes = Changes::new();
        the!("xyz.tonk.unrelated/noise")
            .of("did:key:zNoise".parse::<Entity>().expect("entity URI"))
            .is("x".to_string())
            .assert(&mut changes);
        let by_entity = group_by_entity(changes);
        let (entity, facts) = by_entity.into_iter().next().expect("one entity");

        assert!(
            CreateRepo::decode(entity, &facts).is_none(),
            "missing required `name` field should fail to decode"
        );
    }

    #[dialog_common::test]
    fn it_groups_transients_by_entity() {
        // Two facts on one entity group together; an unrelated entity is
        // its own group.
        let mut changes = create_repo_transient("did:key:zA", "alpha");
        the!("xyz.tonk.unrelated/noise")
            .of("did:key:zB".parse::<Entity>().expect("entity URI"))
            .is("x".to_string())
            .assert(&mut changes);
        let by_entity = group_by_entity(changes);
        assert_eq!(by_entity.len(), 2, "two distinct entities → two groups");
    }
}
