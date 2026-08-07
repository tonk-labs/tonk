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
//! it writes outcomes into.
//!
//! This module is the registry + dispatch seam. The typed handler
//! and its trigger concept are captured behind a dyn-safe
//! [`CommandHandler`] at registration: the registry only ever sees
//! "which attribute names trigger this" and "run it against a matched
//! transient set", so dispatch stays dynamic while decode stays
//! statically typed inside the captured closure.

use std::collections::HashMap;

use dialog_artifacts::{Artifact, Attribute, Changes, Entity, Instruction};
use dialog_capability::{Command, Provider};
use dialog_common::ConditionalSync;
use dialog_query::concept::Concept;
use dialog_query::{Application, ConceptDescriptor, Conclusion, Descriptor, Match, Term};
use tonk_core::command::CommandOccurrence;

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
    C: Concept<Conclusion = C> + Conclusion + Descriptor<ConceptDescriptor>,
    C::Application: Default + Application<Conclusion = C>,
{
    let query = C::Application::default();

    // field name → fully-qualified attribute name (`domain/name`),
    // from the concept's descriptor (read via the derive-generated
    // `Descriptor<ConceptDescriptor>` impl).
    let descriptor = <C as Descriptor<ConceptDescriptor>>::descriptor();
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
    C: Concept<Conclusion = C> + Conclusion + Descriptor<ConceptDescriptor>,
    C::Application: Default + Application<Conclusion = C>,
{
    fn trigger_attributes() -> Vec<String> {
        // The derive exposes the concept's descriptor via the
        // `Descriptor<ConceptDescriptor>` trait (it no longer emits
        // `From<Query> for ConceptDescriptor`).
        <C as Descriptor<ConceptDescriptor>>::descriptor()
            .with()
            .iter()
            .map(|(_, attribute)| attribute.the().to_string())
            .collect()
    }

    fn decode(this: Entity, facts: &EntityFacts) -> Option<Self> {
        decode_concept::<C>(this, facts)
    }
}

/// A `'static` boxed future for one command's execution, `Send` only
/// off wasm — matches the reactor's
/// [`ConditionalSync`](dialog_common::ConditionalSync) convention so a
/// command runs on the single-threaded SW executor. `'static` so the
/// dispatcher can build it, release the state lock, then await it —
/// command IO never runs while a lock is held.
#[cfg(not(target_arch = "wasm32"))]
pub type LegacyRunFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>>;
/// A `'static` boxed future for one command's execution (the wasm
/// single-threaded variant — no `Send` bound). See the native variant
/// above for the rationale.
#[cfg(target_arch = "wasm32")]
pub type LegacyRunFuture = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + 'static>>;

/// Dyn-safe entry stored in the [`CommandRegistry`]. One per registered
/// command *type*. The concrete command `C` is erased behind this
/// object; the registry interacts only through the methods here.
///
/// Generic over `Env` — the environment a command runs against, the
/// [`Provider`](dialog_capability::Provider) the dispatcher calls
/// `execute` on. The env is supplied at dispatch time (the registry
/// can't bake it in: it lives *inside* the consumer's state, an `Arc`
/// cycle). The worker instantiates `Env = CommandEnv` wrapping its
/// `AppState`.
///
/// `ConditionalSync` (Send + Sync off wasm, nothing on wasm) keeps a
/// `Box<dyn CommandHandler<Env>>` — and therefore the consumer's
/// state — `Send + Sync` on native, which axum requires.
pub trait LegacyCommandHandler<Env>: ConditionalSync {
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
    /// Boxed so the trait stays dyn-safe for `Box<dyn CommandHandler<Env>>`.
    fn run(&self, facts: &EntityFacts, env: &Env) -> LegacyRunFuture;
}

/// The `of` (entity) shared by a transient entity's facts. Every
/// artifact in an [`EntityFacts`] carries the same `of` (they were
/// grouped by it), so the first one names the entity.
fn facts_entity(facts: &EntityFacts) -> Option<Entity> {
    facts.first().map(|artifact| artifact.of.clone())
}

/// A registry entry for the command type `C`, run against env `Env`.
/// Holds no handler — the behaviour is the
/// [`Provider<C>`](dialog_capability::Provider) impl on `Env`. Erases
/// `C` (and `Env`) behind [`CommandHandler<Env>`]: `matches` is the
/// derived decode of `C`, `trigger_attributes` comes off `C`'s
/// descriptor, and `run` decodes then calls `Env::execute`.
///
/// A command is registrable iff `Env: Provider<C>` — i.e. the env has
/// the capability to run it. That bound on [`Self::new`] is the
/// (compile-time) capability gate; the UCAN-style runtime gate layers on
/// top of it later.
pub struct LegacyTypedCommand<C, Env> {
    attributes: Vec<String>,
    _command: std::marker::PhantomData<fn() -> (C, Env)>,
}

impl<C, Env> LegacyTypedCommand<C, Env>
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

impl<C, Env> Default for LegacyTypedCommand<C, Env>
where
    C: Decode + Command<Input = C> + 'static,
    Env: Provider<C>,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<C, Env> LegacyCommandHandler<Env> for LegacyTypedCommand<C, Env>
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

    fn run(&self, facts: &EntityFacts, env: &Env) -> LegacyRunFuture {
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

/// Structured failure returned by a scheduled nominal handler.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct CommandFailure {
    /// Stable machine-readable failure code.
    pub code: String,
    /// Human-readable failure detail.
    pub message: String,
}

/// A nominal handler's owned asynchronous run.
#[cfg(not(target_arch = "wasm32"))]
pub type RunFuture = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<(), CommandFailure>> + Send + 'static>,
>;
/// Wasm single-threaded sibling of [`RunFuture`].
#[cfg(target_arch = "wasm32")]
pub type RunFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), CommandFailure>> + 'static>>;

/// Deferred nominal run decoded without synthesizing semantic facts.
#[cfg(not(target_arch = "wasm32"))]
pub type BoxedCommandRun<Env> = Box<dyn FnOnce(&Env) -> RunFuture + Send + 'static>;
/// Wasm single-threaded sibling of [`BoxedCommandRun`].
#[cfg(target_arch = "wasm32")]
pub type BoxedCommandRun<Env> = Box<dyn FnOnce(&Env) -> RunFuture + 'static>;

/// Dyn-safe stable-kind nominal command handler.
pub trait CommandHandler<Env>: ConditionalSync {
    /// Stable command kind selected before decoding.
    fn kind(&self) -> &Entity;
    /// Diagnostic handler name.
    fn name(&self) -> &'static str;
    /// Decode an occurrence into an owned run, or reject its payload.
    fn decode(&self, occurrence: &CommandOccurrence) -> Option<BoxedCommandRun<Env>>;
}

/// One nominal handler scheduled after a successful commit.
pub struct ScheduledHandler {
    /// Diagnostic handler name.
    pub name: &'static str,
    /// Source occurrence that selected this handler.
    pub occurrence: Entity,
    future: RunFuture,
}

impl ScheduledHandler {
    /// Await the handler and retain its structured failure.
    pub async fn perform(self) -> Result<(), CommandFailure> {
        self.future.await
    }
}

/// Decode a Rust `Concept` directly from nominal argument names and
/// bind `this` from the runtime occurrence.
pub fn decode_occurrence<C>(occurrence: &CommandOccurrence) -> Option<C>
where
    C: Concept<Conclusion = C> + Conclusion + Descriptor<ConceptDescriptor>,
    C::Application: Default + Application<Conclusion = C>,
{
    let query = C::Application::default();
    let descriptor = <C as Descriptor<ConceptDescriptor>>::descriptor();
    let mut source = Match::new();
    source
        .bind(&Term::var("this"), occurrence.occurrence().clone().into())
        .ok()?;
    for (field, attribute) in descriptor.with().iter() {
        let term = Term::var(field);
        match occurrence.arguments().get(field) {
            Some(value) => source.bind(&term, value.clone()).ok()?,
            None if attribute.is_optional() => source.bind_absent(&term).ok()?,
            None => return None,
        }
    }
    query.realize(source).ok()
}

/// Typed nominal handler backed by an `Env: Provider<C>` capability.
pub struct NominalTypedCommand<C, Env> {
    kind: Entity,
    _command: std::marker::PhantomData<fn() -> (C, Env)>,
}

/// Transitional exact-kind adapter for operational handlers that still share
/// their implementation with the structural compatibility lane. Argument
/// names are converted only inside the adapter into the legacy facts expected
/// by that implementation; they never enter the transaction overlay or legacy
/// registry matching.
pub struct CompatibilityNominalAdapter<Env> {
    kind: Entity,
    name: &'static str,
    handler: std::sync::Arc<dyn LegacyCommandHandler<Env>>,
    attributes: HashMap<String, Attribute>,
    constants: Vec<(Attribute, dialog_artifacts::Value)>,
}

impl<Env> CompatibilityNominalAdapter<Env> {
    /// Map the fields of a derived Rust concept to their legacy attributes.
    pub fn for_concept<C>(
        kind: Entity,
        name: &'static str,
        handler: std::sync::Arc<dyn LegacyCommandHandler<Env>>,
    ) -> Self
    where
        C: Descriptor<ConceptDescriptor>,
    {
        let attributes = <C as Descriptor<ConceptDescriptor>>::descriptor()
            .with()
            .iter()
            .map(|(field, descriptor)| {
                (
                    field.to_owned(),
                    descriptor
                        .the()
                        .to_string()
                        .parse()
                        .expect("derived attribute"),
                )
            })
            .collect();
        Self {
            kind,
            name,
            handler,
            attributes,
            constants: Vec::new(),
        }
    }

    /// Add an argument consumed by a custom raw-fact reader but not present in
    /// the Rust concept used for legacy matching.
    pub fn argument(mut self, field: impl Into<String>, attribute: Attribute) -> Self {
        self.attributes.insert(field.into(), attribute);
        self
    }

    /// Add a compatibility-only fact that is not part of the nominal payload.
    /// This is used solely to satisfy an old structural decoder's shape
    /// discriminator while exact-kind selection supplies the real authority.
    pub fn constant(mut self, attribute: Attribute, value: dialog_artifacts::Value) -> Self {
        self.constants.push((attribute, value));
        self
    }
}

impl<Env> CommandHandler<Env> for CompatibilityNominalAdapter<Env>
where
    Env: ConditionalSync + 'static,
{
    fn kind(&self) -> &Entity {
        &self.kind
    }

    fn name(&self) -> &'static str {
        self.name
    }

    fn decode(&self, occurrence: &CommandOccurrence) -> Option<BoxedCommandRun<Env>> {
        let mut facts = occurrence
            .arguments()
            .iter()
            .filter_map(|(field, value)| {
                self.attributes.get(field).map(|attribute| Artifact {
                    the: attribute.clone(),
                    of: occurrence.occurrence().clone(),
                    is: value.clone(),
                    cause: None,
                })
            })
            .collect::<EntityFacts>();
        facts.extend(self.constants.iter().map(|(attribute, value)| Artifact {
            the: attribute.clone(),
            of: occurrence.occurrence().clone(),
            is: value.clone(),
            cause: None,
        }));
        if !self.handler.matches(&facts) {
            return None;
        }
        let handler = self.handler.clone();
        Some(Box::new(move |env| {
            let future = handler.run(&facts, env);
            Box::pin(async move {
                future.await;
                Ok(())
            })
        }))
    }
}

impl<C, Env> NominalTypedCommand<C, Env> {
    /// Bind Rust command type `C` to a stable nominal kind.
    pub fn new(kind: Entity) -> Self {
        Self {
            kind,
            _command: std::marker::PhantomData,
        }
    }
}

impl<C, Env> CommandHandler<Env> for NominalTypedCommand<C, Env>
where
    C: Concept<Conclusion = C>
        + Conclusion
        + Descriptor<ConceptDescriptor>
        + Command<Input = C, Output = ()>
        + ConditionalSync
        + 'static,
    C::Application: Default + Application<Conclusion = C>,
    Env: Provider<C> + Clone + ConditionalSync + 'static,
{
    fn kind(&self) -> &Entity {
        &self.kind
    }

    fn name(&self) -> &'static str {
        std::any::type_name::<C>()
    }

    fn decode(&self, occurrence: &CommandOccurrence) -> Option<BoxedCommandRun<Env>> {
        let command = decode_occurrence::<C>(occurrence)?;
        Some(Box::new(move |env| {
            let env = env.clone();
            Box::pin(async move {
                env.execute(command).await;
                Ok(())
            })
        }))
    }
}

/// Registry of command handlers, with a reverse index from trigger
/// attribute name to the handlers it can fire. Mirrors the
/// `dialog.effect/on` index the induce loop walks, but over
/// registered Rust handlers instead of installed `rule!:` effects.
pub struct CommandRegistry<Env> {
    /// Stable kind to nominal handler indices.
    by_kind: HashMap<String, Vec<usize>>,
    /// Nominal handlers selected exclusively by kind.
    nominal_handlers: Vec<Box<dyn CommandHandler<Env>>>,
    /// Registered handlers, owned. Indices into this vec are the
    /// values in [`Self::by_attribute`].
    legacy_handlers: Vec<Box<dyn LegacyCommandHandler<Env>>>,
    /// `attribute name → handler indices`. A transient touching this
    /// attribute makes every listed handler a candidate.
    by_attribute: HashMap<String, Vec<usize>>,
}

impl<Env> Default for CommandRegistry<Env> {
    fn default() -> Self {
        Self {
            by_kind: HashMap::new(),
            nominal_handlers: Vec::new(),
            legacy_handlers: Vec::new(),
            by_attribute: HashMap::new(),
        }
    }
}

impl<Env> CommandRegistry<Env> {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register nominal command type `C` for the stable `kind`.
    pub fn nominal<C>(mut self, kind: Entity) -> Self
    where
        C: Concept<Conclusion = C>
            + Conclusion
            + Descriptor<ConceptDescriptor>
            + Command<Input = C, Output = ()>
            + ConditionalSync
            + 'static,
        C::Application: Default + Application<Conclusion = C>,
        Env: Provider<C> + Clone + ConditionalSync + 'static,
    {
        self.register_nominal(Box::new(NominalTypedCommand::<C, Env>::new(kind)));
        self
    }

    /// Register a nominal handler and index it exclusively by stable kind.
    pub fn register_nominal(&mut self, handler: Box<dyn CommandHandler<Env>>) {
        let index = self.nominal_handlers.len();
        self.by_kind
            .entry(handler.kind().to_string())
            .or_default()
            .push(index);
        self.nominal_handlers.push(handler);
    }

    /// Number of nominal handlers registered for `kind`.
    pub fn registrations(&self, kind: &Entity) -> usize {
        self.by_kind.get(&kind.to_string()).map_or(0, Vec::len)
    }

    /// Decode and schedule every nominal handler registered for the
    /// occurrence's exact stable kind.
    pub fn schedule(&self, occurrence: &CommandOccurrence, env: &Env) -> Vec<ScheduledHandler> {
        self.by_kind
            .get(&occurrence.command().to_string())
            .into_iter()
            .flatten()
            .filter_map(|index| {
                let handler = self.nominal_handlers[*index].as_ref();
                let run = handler.decode(occurrence)?;
                Some(ScheduledHandler {
                    name: handler.name(),
                    occurrence: occurrence.occurrence().clone(),
                    future: run(env),
                })
            })
            .collect()
    }

    /// Register the legacy structural command type `C`. Lighter than passing a handler:
    /// the behaviour is the [`Provider<C>`](dialog_capability::Provider)
    /// impl on [`Env`], so registration is just the type. The
    /// `Env: Provider<C>` bound means a command can only be registered if
    /// the env has the capability to run it. Chainable.
    ///
    /// ```ignore
    /// let registry = CommandRegistry::new()
    ///     .legacy::<CreateSpace>()   // Env: Provider<CreateSpace>
    ///     .legacy::<RenameSpace>();
    /// ```
    pub fn legacy<C>(mut self) -> Self
    where
        C: Decode + Command<Input = C, Output = ()> + ConditionalSync + 'static,
        Env: Provider<C> + Clone + ConditionalSync + 'static,
    {
        self.register_legacy(Box::new(LegacyTypedCommand::<C, Env>::new()));
        self
    }

    /// Register a boxed handler, indexing it by each of its trigger
    /// attribute names. Prefer [`Self::legacy`] for typed legacy handlers.
    pub fn register_legacy(&mut self, handler: Box<dyn LegacyCommandHandler<Env>>) {
        let index = self.legacy_handlers.len();
        for name in handler.trigger_attributes() {
            self.by_attribute
                .entry(name.clone())
                .or_default()
                .push(index);
        }
        self.legacy_handlers.push(handler);
    }

    /// `true` when no handlers are registered — lets the reactor skip
    /// the whole dispatch pass (group-by-entity, candidate lookup)
    /// when commands aren't in use.
    pub fn is_empty(&self) -> bool {
        self.nominal_handlers.is_empty() && self.legacy_handlers.is_empty()
    }

    /// For a committed transient batch, find every `(handler, entity
    /// facts)` pair that should fire: group the transients by entity,
    /// look up candidate handlers via the touched attributes, and
    /// keep those whose trigger concept actually decodes.
    ///
    /// Returns references into `self` paired with a clone of the
    /// matched entity's facts; the caller (step 4) decodes + spawns
    /// each.
    pub fn match_legacy_transients<'a>(
        &'a self,
        transients: &Changes,
    ) -> Vec<(&'a dyn LegacyCommandHandler<Env>, EntityFacts)> {
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
                let handler = self.legacy_handlers[index].as_ref();
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

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    use super::*;
    use dialog_artifacts::{Statement, Value};
    use dialog_query::{Attribute, AttributeDescriptor, Cardinality, Concept, Type, the};
    use tonk_core::claim::ValueMap;
    use tonk_core::command::{CommandBatch, CommandSchema, InvocationMetadata, SourceInvocation};

    // --- Test command concepts ------------------------------------------
    // Their `Provider`s live at the router layer; here we test the decode,
    // grouping, and registry/matching machinery, which needs no env.

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.command")]
    pub struct RepoName(pub String);

    #[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
    pub struct CreateRepo {
        pub this: Entity,
        pub name: RepoName,
    }

    // A two-field command: `name` (text) + `owner` (entity), to cover
    // multi-field decode and a typed (entity) field.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.command")]
    pub struct Owner(pub Entity);

    #[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
    pub struct Share {
        pub this: Entity,
        pub name: RepoName,
        pub owner: Owner,
    }

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.command")]
    pub struct Note(pub String);

    #[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
    pub struct OptionalCreate {
        pub this: Entity,
        pub name: RepoName,
        pub note: Option<Note>,
    }

    fn entity(uri: &str) -> Entity {
        uri.parse().expect("entity URI")
    }

    /// A one-fact transient `CreateRepo{this: of, name}` batch. The
    /// attribute is `RepoName`'s name (snake-cased from its struct name),
    /// which is what the reverse index keys on.
    fn create_repo_transient(of: &str, name: &str) -> Changes {
        let mut changes = Changes::new();
        the!("xyz.tonk.command/repo-name")
            .of(entity(of))
            .is(name.to_string())
            .assert(&mut changes);
        changes
    }

    fn noise_fact(changes: &mut Changes, of: &str) {
        the!("xyz.tonk.unrelated/noise")
            .of(entity(of))
            .is("x".to_string())
            .assert(changes);
    }

    fn facts_for(changes: Changes) -> (Entity, EntityFacts) {
        group_by_entity(changes)
            .into_iter()
            .next()
            .expect("one entity")
    }

    fn string_field(field: &str) -> AttributeDescriptor {
        AttributeDescriptor::new(
            format!("xyz.tonk.command/{field}")
                .parse()
                .expect("valid attribute"),
            "",
            Cardinality::One,
            Some(Type::String),
        )
    }

    fn nominal_occurrence(
        kind: &str,
        occurrence: &str,
        required: &[&str],
        optional: &[&str],
        arguments: ValueMap,
    ) -> CommandOccurrence {
        let schema = CommandSchema {
            required: required
                .iter()
                .map(|field| ((*field).to_string(), string_field(field)))
                .collect(),
            optional: optional
                .iter()
                .map(|field| ((*field).to_string(), string_field(field)))
                .collect(),
        };
        let invocation = schema
            .validate(SourceInvocation {
                command: entity(kind),
                arguments,
            })
            .expect("valid nominal invocation");
        CommandOccurrence::new(
            invocation,
            InvocationMetadata::new(entity(occurrence), format!("test:{occurrence}")),
        )
    }

    // --- decode ---------------------------------------------------------

    #[dialog_common::test]
    fn it_decodes_a_command_from_its_transient_facts() {
        let (entity, facts) = facts_for(create_repo_transient("did:key:zCmd", "pictures"));
        let decoded = CreateRepo::decode(entity.clone(), &facts).expect("decodes");
        assert_eq!(decoded.this, entity);
        assert_eq!(decoded.name.0, "pictures");
    }

    #[dialog_common::test]
    fn it_decodes_a_multi_field_command() {
        let owner = entity("did:key:zOwner");
        let mut changes = Changes::new();
        the!("xyz.tonk.command/repo-name")
            .of(entity("did:key:zShare"))
            .is("pics".to_string())
            .assert(&mut changes);
        the!("xyz.tonk.command/owner")
            .of(entity("did:key:zShare"))
            .is(owner.clone())
            .assert(&mut changes);
        let (this, facts) = facts_for(changes);

        let decoded = Share::decode(this, &facts).expect("two-field concept decodes");
        assert_eq!(decoded.name.0, "pics");
        assert_eq!(decoded.owner.0, owner);
    }

    /// Regression: the real [`CreateSpace`] command must decode from
    /// facts that carry only `name`. A profile branch seeded by an older
    /// version has the name-only `space/create` descriptor, so its form
    /// asserts a name-only transient. A `CreateSpace` that required an
    /// extra field (e.g. a remote URL) would silently fail to match
    /// there — `dispatch` commits the transient and runs nothing — and
    /// break *all* space creation for existing profiles. Keep create-time
    /// fields off the matched concept.
    #[dialog_common::test]
    fn it_decodes_create_space_from_name_only_facts() {
        use tonk_schema::command::CreateSpace;

        let this = entity("did:key:zCreateSpace");
        let mut changes = Changes::new();
        the!("dom.event.current-target.elements.name/value")
            .of(this.clone())
            .is("pictures".to_string())
            .assert(&mut changes);
        let (this, facts) = facts_for(changes);

        let decoded = CreateSpace::decode(this, &facts)
            .expect("CreateSpace must decode from name-only facts (older descriptor / blank form)");
        assert_eq!(decoded.name.0, "pictures");
    }

    /// The Hub's per-row delete confirm asserts a `space/remove`
    /// transient carrying only `data-remove` (the subject DID).
    #[dialog_common::test]
    fn it_decodes_remove_space_from_a_data_remove_fact() {
        use tonk_schema::command::RemoveSpace;

        let this = entity("did:key:zRemoveSpace");
        let subject = entity("did:key:zSpaceSubject");
        let mut changes = Changes::new();
        the!("dom.event.current-target.dataset/remove")
            .of(this.clone())
            .is(subject.clone())
            .assert(&mut changes);
        let (this, facts) = facts_for(changes);

        let decoded = RemoveSpace::decode(this, &facts)
            .expect("RemoveSpace must decode from a data-remove-only transient");
        assert_eq!(decoded.subject.0, subject);
    }

    /// The FAB's repository-name chip dispatches `tonk:rename-repository`
    /// from the PROFILE branch, so the handler cannot read the target
    /// repository from the dispatch origin — it must read it off the
    /// command itself. Assert both the new `name` and the target `space`
    /// DID decode from the transient's raw facts.
    #[dialog_common::test]
    fn it_decodes_rename_repository_naming_its_target_space() {
        use tonk_schema::command::RenameRepository;

        let this = entity("did:key:zRenameRepository");
        let target_space = entity("did:key:zTargetSpace");
        let mut changes = Changes::new();
        the!("dom.event.current-target/value")
            .of(this.clone())
            .is("Renamed".to_string())
            .assert(&mut changes);
        the!("xyz.tonk.rename-repository/space")
            .of(this.clone())
            .is(target_space.clone())
            .assert(&mut changes);
        the!("dom.event.current-target.dataset/rename-repository")
            .of(this.clone())
            .is(entity("tonk:repository"))
            .assert(&mut changes);
        let (this, facts) = facts_for(changes);

        let decoded = RenameRepository::decode(this, &facts)
            .expect("RenameRepository must decode from its raw facts");
        assert_eq!(decoded.name.0, "Renamed");
        assert_eq!(
            decoded.space.0, target_space,
            "the handler must rename the space named by the command, not the dispatch origin"
        );
    }

    /// Regression: a `tonk/rename-repository` transient carries
    /// `dataset/subject` plus the new name. It must NOT decode as
    /// `RemoveSpace` — a remove command keyed on `dataset/subject`
    /// would turn every banner rename into a space deletion.
    #[dialog_common::test]
    fn it_does_not_decode_a_rename_transient_as_remove_space() {
        use tonk_schema::command::RemoveSpace;

        let mut changes = Changes::new();
        the!("dom.event.current-target.dataset/subject")
            .of(entity("did:key:zRename"))
            .is(entity("did:key:zSpaceSubject"))
            .assert(&mut changes);
        the!("dom.event.current-target/value")
            .of(entity("did:key:zRename"))
            .is("new name".to_string())
            .assert(&mut changes);
        let (this, facts) = facts_for(changes);

        assert!(
            RemoveSpace::decode(this, &facts).is_none(),
            "a rename-shaped transient must not decode as RemoveSpace"
        );
    }

    /// Regression for the bug this pair of tests exists to prevent:
    /// renaming a space's repository was ALSO renaming the user's profile.
    /// `ProfileRename` and `RenameRepository` both used to derive the same
    /// `dom.event.current-target.dataset/rename` marker attribute, making
    /// `ProfileRename`'s `{value, marker}` shape a strict SUBSET of a
    /// repo-rename transient's `{value, marker, space}` — so a repo-rename
    /// decoded as BOTH commands and both handlers fired. Command decoding
    /// matches on which attributes are PRESENT, never their values, so a
    /// shared marker value (`tonk:repository` vs `tonk:profile`) never
    /// disambiguated anything. The fix is a DISTINCT ATTRIBUTE per command
    /// — `dataset/rename-repository` here — the same pattern
    /// `remove::Remove` already uses (see
    /// `it_does_not_decode_a_rename_transient_as_remove_space` above).
    #[dialog_common::test]
    fn it_does_not_decode_a_repo_rename_as_a_profile_rename() {
        use tonk_schema::command::{ProfileRename, RenameRepository};

        let this = entity("did:key:zRepoRename");
        let target_space = entity("did:key:zTargetSpace");
        let mut changes = Changes::new();
        the!("dom.event.current-target/value")
            .of(this.clone())
            .is("Renamed".to_string())
            .assert(&mut changes);
        the!("xyz.tonk.rename-repository/space")
            .of(this.clone())
            .is(target_space.clone())
            .assert(&mut changes);
        the!("dom.event.current-target.dataset/rename-repository")
            .of(this.clone())
            .is(entity("tonk:repository"))
            .assert(&mut changes);
        let (this, facts) = facts_for(changes);

        assert!(
            RenameRepository::decode(this.clone(), &facts).is_some(),
            "a repo-rename transient must decode as RenameRepository"
        );
        assert!(
            ProfileRename::decode(this, &facts).is_none(),
            "a repo-rename transient must NOT also decode as ProfileRename — \
             that is the bug: renaming a spot was also renaming the profile"
        );
    }

    /// Converse of the above: a profile-rename transient (no `space`, and
    /// the `profile/rename` marker attribute) must not decode as
    /// `RenameRepository`, which requires `space` regardless.
    #[dialog_common::test]
    fn it_does_not_decode_a_profile_rename_as_a_repo_rename() {
        use tonk_schema::command::{ProfileRename, RenameRepository};

        let this = entity("did:key:zProfileRename");
        let mut changes = Changes::new();
        the!("dom.event.current-target/value")
            .of(this.clone())
            .is("Ada".to_string())
            .assert(&mut changes);
        the!("dom.event.current-target.dataset/rename")
            .of(this.clone())
            .is(entity("tonk:profile"))
            .assert(&mut changes);
        let (this, facts) = facts_for(changes);

        assert!(
            ProfileRename::decode(this.clone(), &facts).is_some(),
            "a profile-rename transient must decode as ProfileRename"
        );
        assert!(
            RenameRepository::decode(this, &facts).is_none(),
            "a profile-rename transient must not decode as RenameRepository \
             (it carries no `space`)"
        );
    }

    #[dialog_common::test]
    fn it_does_not_decode_when_a_required_field_is_absent() {
        let mut changes = Changes::new();
        noise_fact(&mut changes, "did:key:zNoise");
        let (entity, facts) = facts_for(changes);
        assert!(
            CreateRepo::decode(entity, &facts).is_none(),
            "missing required `name` field should fail to decode"
        );
    }

    #[dialog_common::test]
    fn it_does_not_decode_a_partial_multi_field_command() {
        // `Share` needs name AND owner; only `name` present → no decode.
        let (this, facts) = facts_for(create_repo_transient("did:key:zShare", "pics"));
        assert!(
            Share::decode(this, &facts).is_none(),
            "a multi-field command missing one field must not decode"
        );
    }

    #[dialog_common::test]
    fn it_does_not_decode_on_a_type_mismatch() {
        // `Share.owner` is an entity field; give it a string. The
        // `try_into` in the derived realize fails → no decode.
        let mut changes = Changes::new();
        the!("xyz.tonk.command/repo-name")
            .of(entity("did:key:zBad"))
            .is("pics".to_string())
            .assert(&mut changes);
        the!("xyz.tonk.command/owner")
            .of(entity("did:key:zBad"))
            .is("not-an-entity".to_string()) // wrong type for `Owner(Entity)`
            .assert(&mut changes);
        let (this, facts) = facts_for(changes);
        assert!(
            Share::decode(this, &facts).is_none(),
            "an entity field given a string must fail to decode"
        );
    }

    #[dialog_common::test]
    fn it_ignores_unrelated_attributes_when_decoding() {
        // The command's own fields decode even though the entity carries
        // an extra, unrelated attribute.
        let mut changes = create_repo_transient("did:key:zExtra", "pictures");
        noise_fact(&mut changes, "did:key:zExtra");
        let (this, facts) = facts_for(changes);
        let decoded = CreateRepo::decode(this, &facts).expect("decodes despite extra attr");
        assert_eq!(decoded.name.0, "pictures");
    }

    #[dialog_common::test]
    fn it_reports_a_commands_trigger_attributes() {
        let attrs = CreateRepo::trigger_attributes();
        assert_eq!(attrs, vec!["xyz.tonk.command/repo-name".to_string()]);

        let mut share = Share::trigger_attributes();
        share.sort();
        assert_eq!(
            share,
            vec![
                "xyz.tonk.command/owner".to_string(),
                "xyz.tonk.command/repo-name".to_string(),
            ]
        );
    }

    // --- group_by_entity ------------------------------------------------

    #[dialog_common::test]
    fn it_groups_facts_of_one_entity_together() {
        let mut changes = create_repo_transient("did:key:zA", "alpha");
        noise_fact(&mut changes, "did:key:zA"); // same entity, 2nd fact
        let by_entity = group_by_entity(changes);
        assert_eq!(by_entity.len(), 1, "one entity → one group");
        assert_eq!(
            by_entity.values().next().unwrap().len(),
            2,
            "both facts land in the group"
        );
    }

    #[dialog_common::test]
    fn it_groups_distinct_entities_separately() {
        let mut changes = create_repo_transient("did:key:zA", "alpha");
        noise_fact(&mut changes, "did:key:zB");
        assert_eq!(
            group_by_entity(changes).len(),
            2,
            "two distinct entities → two groups"
        );
    }

    #[dialog_common::test]
    fn it_groups_an_empty_batch_to_nothing() {
        assert!(group_by_entity(Changes::new()).is_empty());
    }

    // --- registry + matching (via a stub handler) -----------------------
    // A stub `LegacyCommandHandler` lets us exercise the reverse-index match and
    // the "all matches fire" / "candidate but doesn't decode" edges
    // without a real `Provider` env (which lives at the router layer).

    // A trivial env for the registry/match tests. The matching path
    // never touches the env (only `run` does, which these tests don't
    // call), so a unit type is enough to pin the `Env` parameter.
    #[derive(Clone)]
    struct TestEnv;

    struct NominalStub {
        kind: Entity,
        name: &'static str,
        failure: Option<CommandFailure>,
    }

    impl NominalStub {
        fn succeeds(kind: &str, name: &'static str) -> Box<dyn CommandHandler<TestEnv>> {
            Box::new(Self {
                kind: entity(kind),
                name,
                failure: None,
            })
        }
    }

    impl CommandHandler<TestEnv> for NominalStub {
        fn kind(&self) -> &Entity {
            &self.kind
        }

        fn name(&self) -> &'static str {
            self.name
        }

        fn decode(&self, _occurrence: &CommandOccurrence) -> Option<BoxedCommandRun<TestEnv>> {
            let failure = self.failure.clone();
            Some(Box::new(move |_env| {
                Box::pin(async move {
                    match failure {
                        Some(failure) => Err(failure),
                        None => Ok(()),
                    }
                })
            }))
        }
    }

    struct StubHandler {
        attributes: Vec<String>,
        // Decode succeeds only when the entity carries this attribute.
        decode_on: String,
    }

    impl StubHandler {
        fn boxed(attribute: &str) -> Box<dyn LegacyCommandHandler<TestEnv>> {
            Box::new(StubHandler {
                attributes: vec![attribute.to_string()],
                decode_on: attribute.to_string(),
            })
        }
    }

    impl LegacyCommandHandler<TestEnv> for StubHandler {
        fn trigger_attributes(&self) -> &[String] {
            &self.attributes
        }
        fn matches(&self, facts: &EntityFacts) -> bool {
            facts.iter().any(|a| a.the.to_string() == self.decode_on)
        }
        fn run(&self, _facts: &EntityFacts, _env: &TestEnv) -> LegacyRunFuture {
            Box::pin(async {})
        }
    }

    #[dialog_common::test]
    fn it_reports_empty_until_a_command_is_registered() {
        let mut registry = CommandRegistry::new();
        assert!(registry.is_empty());
        registry.register_legacy(StubHandler::boxed("xyz.tonk.command/repo-name"));
        assert!(!registry.is_empty());
    }

    #[dialog_common::test]
    fn it_matches_a_registered_command_on_its_trigger() {
        let mut registry = CommandRegistry::new();
        registry.register_legacy(StubHandler::boxed("xyz.tonk.command/repo-name"));
        let fired =
            registry.match_legacy_transients(&create_repo_transient("did:key:zCmd", "pics"));
        assert_eq!(fired.len(), 1);
    }

    #[dialog_common::test]
    fn it_does_not_match_an_unrelated_transient() {
        let mut registry = CommandRegistry::new();
        registry.register_legacy(StubHandler::boxed("xyz.tonk.command/repo-name"));
        let mut changes = Changes::new();
        noise_fact(&mut changes, "did:key:zNoise");
        assert!(registry.match_legacy_transients(&changes).is_empty());
    }

    #[dialog_common::test]
    fn it_fires_all_matching_commands_subscription_style() {
        // Two commands registered on the same trigger attribute: BOTH
        // fire (commands are subscription-like, no tiebreak).
        let mut registry = CommandRegistry::new();
        registry.register_legacy(StubHandler::boxed("xyz.tonk.command/repo-name"));
        registry.register_legacy(StubHandler::boxed("xyz.tonk.command/repo-name"));
        let fired =
            registry.match_legacy_transients(&create_repo_transient("did:key:zCmd", "pics"));
        assert_eq!(fired.len(), 2, "both commands on the attribute fire");
    }

    #[dialog_common::test]
    fn it_drops_a_candidate_whose_trigger_does_not_decode() {
        // A command keyed on an attribute the entity has, but whose
        // `matches` (decode) returns false, is a candidate but does NOT
        // fire. We register a stub keyed on `repo-name` but that only
        // decodes on `owner`, then submit a `repo-name`-only entity.
        let mut registry = CommandRegistry::new();
        registry.register_legacy(Box::new(StubHandler {
            attributes: vec!["xyz.tonk.command/repo-name".to_string()],
            decode_on: "xyz.tonk.command/owner".to_string(),
        }));
        let fired =
            registry.match_legacy_transients(&create_repo_transient("did:key:zCmd", "pics"));
        assert!(
            fired.is_empty(),
            "a candidate that fails to decode must not fire"
        );
    }

    #[dialog_common::test]
    fn it_considers_a_command_once_when_two_of_its_attributes_match() {
        // A command keyed on two attributes, both present on the entity,
        // is still considered once (candidate dedup).
        let mut registry = CommandRegistry::new();
        registry.register_legacy(Box::new(StubHandler {
            attributes: vec![
                "xyz.tonk.command/repo-name".to_string(),
                "xyz.tonk.command/owner".to_string(),
            ],
            decode_on: "xyz.tonk.command/repo-name".to_string(),
        }));
        let mut changes = create_repo_transient("did:key:zCmd", "pics");
        the!("xyz.tonk.command/owner")
            .of(entity("did:key:zCmd"))
            .is(entity("did:key:zOwner"))
            .assert(&mut changes);
        let fired = registry.match_legacy_transients(&changes);
        assert_eq!(fired.len(), 1, "the command fires once, not per-attribute");
    }

    #[dialog_common::test]
    fn it_matches_each_command_entity_in_a_batch() {
        // Two distinct command entities in one batch each match.
        let mut registry = CommandRegistry::new();
        registry.register_legacy(StubHandler::boxed("xyz.tonk.command/repo-name"));
        let mut changes = create_repo_transient("did:key:zA", "alpha");
        the!("xyz.tonk.command/repo-name")
            .of(entity("did:key:zB"))
            .is("beta".to_string())
            .assert(&mut changes);
        let fired = registry.match_legacy_transients(&changes);
        assert_eq!(fired.len(), 2, "each command entity fires once");
    }

    #[dialog_common::test]
    fn it_hands_each_match_the_full_entity_facts() {
        // The facts handed to a matched handler are that entity's facts
        // (so the handler can decode every field).
        let mut registry = CommandRegistry::new();
        registry.register_legacy(StubHandler::boxed("xyz.tonk.command/repo-name"));
        let fired =
            registry.match_legacy_transients(&create_repo_transient("did:key:zCmd", "pics"));
        let (_, facts) = &fired[0];
        let name = facts
            .iter()
            .find(|a| a.the.to_string() == "xyz.tonk.command/repo-name")
            .and_then(|a| match &a.is {
                Value::String(s) => Some(s.clone()),
                _ => None,
            });
        assert_eq!(name.as_deref(), Some("pics"));
    }

    // --- nominal registry + decoding ----------------------------------

    #[dialog_common::test]
    fn nominal_decode_binds_runtime_this_and_preserves_empty_strings() {
        let occurrence = nominal_occurrence(
            "id:repo/create",
            "did:key:zOccurrence",
            &["name"],
            &[],
            [("name".into(), Value::String(String::new()))]
                .into_iter()
                .collect(),
        );

        let decoded = decode_occurrence::<CreateRepo>(&occurrence).expect("decodes");
        assert_eq!(decoded.this, entity("did:key:zOccurrence"));
        assert_eq!(decoded.name.0, "", "empty string is a supplied value");
    }

    #[dialog_common::test]
    fn nominal_decode_binds_absent_optional_fields() {
        let occurrence = nominal_occurrence(
            "id:repo/create",
            "did:key:zOptional",
            &["name"],
            &["note"],
            [("name".into(), Value::String("pictures".into()))]
                .into_iter()
                .collect(),
        );

        let decoded =
            decode_occurrence::<OptionalCreate>(&occurrence).expect("optional field decodes");
        assert_eq!(decoded.name.0, "pictures");
        assert_eq!(decoded.note, None);
    }

    #[dialog_common::test]
    fn nominal_decode_rejects_a_missing_rust_required_field() {
        // The occurrence is valid under its current nominal schema, but the
        // selected Rust handler expects an additional required `owner` field.
        // Decode must reject it instead of manufacturing a value.
        let occurrence = nominal_occurrence(
            "id:repo/share",
            "did:key:zPartial",
            &["name"],
            &[],
            [("name".into(), Value::String("pictures".into()))]
                .into_iter()
                .collect(),
        );

        assert!(decode_occurrence::<Share>(&occurrence).is_none());
    }

    #[dialog_common::test]
    async fn nominal_dispatch_selects_exact_kind_before_shape() {
        let kind_a = entity("id:repo/create-a");
        let kind_b = entity("id:repo/create-b");
        let mut registry = CommandRegistry::new();
        registry.register_nominal(NominalStub::succeeds("id:repo/create-a", "handler-a"));
        registry.register_nominal(NominalStub::succeeds("id:repo/create-b", "handler-b"));
        let occurrence = nominal_occurrence(
            "id:repo/create-a",
            "did:key:zA",
            &["name"],
            &[],
            [("name".into(), Value::String("same shape".into()))]
                .into_iter()
                .collect(),
        );

        assert_eq!(registry.registrations(&kind_a), 1);
        assert_eq!(registry.registrations(&kind_b), 1);
        let scheduled = registry.schedule(&occurrence, &TestEnv);
        assert_eq!(scheduled.len(), 1);
        assert_eq!(scheduled[0].name, "handler-a");
        scheduled
            .into_iter()
            .next()
            .unwrap()
            .perform()
            .await
            .unwrap();
    }

    #[dialog_common::test]
    async fn nominal_dispatch_preserves_structured_handler_failures() {
        let failure = CommandFailure {
            code: "repo-unavailable".into(),
            message: "repository is offline".into(),
        };
        let mut registry = CommandRegistry::new();
        registry.register_nominal(Box::new(NominalStub {
            kind: entity("id:repo/create"),
            name: "failing-handler",
            failure: Some(failure.clone()),
        }));
        let occurrence = nominal_occurrence(
            "id:repo/create",
            "did:key:zFailure",
            &["name"],
            &[],
            [("name".into(), Value::String("pictures".into()))]
                .into_iter()
                .collect(),
        );

        let scheduled = registry.schedule(&occurrence, &TestEnv);
        assert_eq!(scheduled.len(), 1);
        assert_eq!(
            scheduled.into_iter().next().unwrap().perform().await,
            Err(failure)
        );
    }

    #[dialog_common::test]
    fn nominal_private_overlay_does_not_enter_the_legacy_lane() {
        let occurrence = nominal_occurrence(
            "id:repo/create",
            "did:key:zIsolated",
            &["name"],
            &[],
            [("name".into(), Value::String("pictures".into()))]
                .into_iter()
                .collect(),
        );
        let encoded = CommandBatch::new(vec![occurrence]).encode();
        let mut registry = CommandRegistry::new();
        registry.register_legacy(StubHandler::boxed("xyz.tonk.command/repo-name"));

        assert!(registry.match_legacy_transients(&encoded).is_empty());
    }
}
