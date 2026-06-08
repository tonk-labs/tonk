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
use dialog_common::{ConditionalSend, ConditionalSync};
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

/// A transient trigger concept — the `&C` member of a command
/// handler's query. Supplies the reverse-index keys (its field
/// attribute names) and a decode from a transient entity's facts.
///
/// Blanket-implemented for any decodable [`Concept`]; the bounds are
/// exactly what [`decode_concept`] needs, so a plain
/// `#[derive(Concept)]` is a `Command` with no extra code.
pub trait Command: Sized {
    /// Fully-qualified attribute URIs (`domain/name`) of this
    /// trigger's fields. An entity in the transient set is a
    /// candidate when it carries any of these.
    fn trigger_attributes() -> Vec<String>;

    /// Decode this trigger from one transient entity's facts, or
    /// `None` if they don't satisfy it (missing/mistyped field).
    fn decode(this: Entity, facts: &EntityFacts) -> Option<Self>;
}

impl<C> Command for C
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

/// The write side of a command handler — the Bevy `Commands` analog.
/// A deferred buffer: the handler calls [`Self::assert`] /
/// [`Self::retract`] as it runs, and the reactor commits the
/// accumulated [`Changes`] as one durable transaction after the
/// handler future resolves (step 4). The handler does NOT commit and
/// can't read back its own writes mid-run.
///
/// Outcomes are durable (so UIs react over their subscriptions); the
/// transient trigger self-retracts at the originating commit, so a
/// handler never retracts its own command.
#[derive(Default)]
pub struct Transaction {
    changes: Changes,
}

impl Transaction {
    /// An empty outcome buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Buffer an assertion. Accepts anything assertable — typically a
    /// `#[derive(Concept)]` outcome (`StatusChange`, `Failed`, …),
    /// which implements [`Statement`](dialog_artifacts::Statement).
    pub fn assert<S: dialog_artifacts::Statement>(&mut self, claim: S) -> &mut Self {
        self.changes.assert(claim);
        self
    }

    /// Buffer a retraction.
    pub fn retract<S: dialog_artifacts::Statement>(&mut self, claim: S) -> &mut Self {
        self.changes.retract(claim);
        self
    }

    /// Consume the buffer, yielding the accumulated [`Changes`] for
    /// the reactor to commit.
    pub fn into_changes(self) -> Changes {
        self.changes
    }
}

/// A `'static` boxed future yielding the outcome `Changes` (or `None`
/// if the trigger didn't decode), `Send` only off wasm. `'static` so
/// the dispatcher can collect these, release the state lock, then await
/// them — handler IO never runs while a lock is held.
#[cfg(not(target_arch = "wasm32"))]
pub type RunFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = Option<Changes>> + Send + 'static>>;
#[cfg(target_arch = "wasm32")]
pub type RunFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = Option<Changes>> + 'static>>;

/// A `'static` future yielding the populated outcome buffer, `Send`
/// only off wasm — matches the reactor's
/// [`ConditionalSync`](dialog_common::ConditionalSync) convention so a
/// handler runs on the single-threaded SW executor.
#[cfg(not(target_arch = "wasm32"))]
pub type HandlerFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = Transaction> + Send + 'static>>;
#[cfg(target_arch = "wasm32")]
pub type HandlerFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = Transaction> + 'static>>;

/// The source extractors pull their context from — the shared state,
/// supplied to a handler at dispatch time. Mirrors axum's per-request
/// state injection: the registry doesn't bake state in (the registry
/// lives *inside* the state, so it can't hold an `Arc` to itself), so
/// the dispatcher passes the source when it runs a handler.
///
/// `Source` is the worker's [`AppState`](crate::router::AppState) in
/// practice; the trait keeps this module from depending on it directly.
pub type Source = crate::router::AppState;

/// Extract `Self` from the shared [`Source`] — the command analog of
/// axum's `FromRef`/`FromRequestParts`. A handler parameter that is
/// `FromContext` is a *declared capability*: the handler names what it
/// needs (`State<AppState>`, …) and the dispatcher provides it from the
/// source. Pure handlers declare none.
pub trait FromContext: Sized {
    /// Pull this value from the source. Cheap — typically an `Arc`
    /// clone — so the resulting handler future stays `'static`.
    fn from_context(source: &Source) -> Self;
}

/// Capability extractor: a cheap handle onto the shared state, named by
/// `T` (axum's `State<T>`). For now `T = Source`; `FromRef`-style
/// sub-handle extraction can layer on later without changing handler
/// signatures. The handler reaches the operator/reactor by re-locking
/// through this handle when it needs them — declared in the signature,
/// not handed wholesale.
///
/// TODO(stm): reads through `State<T>` are not tracked, so the
/// dispatcher's outcome commit can't tell whether what the handler read
/// still holds. The transactional goal is a read-tracking extractor
/// (e.g. a `Snapshot<C>`/`Current<C>` that records the revision or
/// datoms it observed) so dispatch can commit-or-conflict against the
/// handler's read set. See the `TODO(stm)` notes on
/// `router::command::dispatch`/`commit_outcome`.
pub struct State<T>(pub T);

impl FromContext for State<Source> {
    fn from_context(source: &Source) -> Self {
        State(source.clone())
    }
}

/// A typed command handler function: an `async fn` taking the decoded
/// trigger (owned), zero or more declared-capability extractors, and an
/// outcome [`Transaction`] to fill (returned).
///
/// One blanket impl per extractor arity (axum/bevy style). A handler
/// with no extractor is a pure transform — no capability — and can only
/// decide outcomes. A handler that needs IO declares it as a
/// `FromContext` parameter (e.g. `State<AppState>`); the dispatcher
/// supplies it from the source. Outcome facts always flow only through
/// the returned `Transaction`, committed by the dispatcher to the
/// command's branch — declaring a capability grants IO, never commit
/// authority.
///
/// All values are owned so the future is `'static` and can run detached
/// past the triggering commit.
pub trait CommandFn<C, Args>: 'static {
    /// Run the handler against the dispatch `source`: extract the
    /// declared capabilities, then invoke the handler with the trigger
    /// and outcome buffer.
    fn run(&self, command: C, tx: Transaction, source: &Source) -> HandlerFuture;
}

// Arity 0: pure transform `async fn(C, Transaction) -> Transaction`.
impl<C, F, Fut> CommandFn<C, ()> for F
where
    F: Fn(C, Transaction) -> Fut + 'static,
    Fut: std::future::Future<Output = Transaction> + ConditionalSend + 'static,
    C: 'static,
{
    fn run(&self, command: C, tx: Transaction, _source: &Source) -> HandlerFuture {
        Box::pin(self(command, tx))
    }
}

// Arity 1: one declared capability `async fn(C, E1, Transaction) ->
// Transaction`. The extractor sits between the trigger and the
// transaction, matching the read-context-then-write shape.
impl<C, F, Fut, E1> CommandFn<C, (E1,)> for F
where
    F: Fn(C, E1, Transaction) -> Fut + 'static,
    Fut: std::future::Future<Output = Transaction> + ConditionalSend + 'static,
    C: 'static,
    E1: FromContext,
{
    fn run(&self, command: C, tx: Transaction, source: &Source) -> HandlerFuture {
        let e1 = E1::from_context(source);
        Box::pin(self(command, e1, tx))
    }
}

/// Dyn-safe handler stored in the [`CommandRegistry`]. One per
/// registered command. The concrete handler fn and its trigger
/// concept `C` are erased behind this object; the registry interacts
/// only through the methods here.
///
/// `ConditionalSync` (Send + Sync off wasm, nothing on wasm) keeps a
/// `Box<dyn CommandHandler>` — and therefore [`TonkState`] /
/// [`AppState`] — `Send + Sync` on native, which axum requires.
///
/// [`TonkState`]: crate::worker::TonkState
/// [`AppState`]: crate::router::AppState
pub trait CommandHandler: ConditionalSync {
    /// Attribute names whose presence on a transient entity makes
    /// this handler a candidate. Drives the reverse index.
    fn trigger_attributes(&self) -> &[String];

    /// Whether `facts` (the asserted artifacts for a single transient
    /// entity) satisfy this handler's trigger concept — i.e. the `&C`
    /// member decodes.
    fn matches(&self, facts: &EntityFacts) -> bool;

    /// Decode the trigger from `facts`, run the handler against
    /// `source` (the dispatch-time capability source), and return the
    /// outcome [`Changes`] for the dispatcher to commit. `None` if the
    /// facts don't decode (treated as "didn't fire").
    ///
    /// The returned future owns the decoded trigger, the outcome
    /// buffer, and any extracted capabilities, so it is `'static` and
    /// can run detached past the triggering commit. Crucially this lets
    /// the dispatcher release the state lock before awaiting it, so
    /// handler IO never runs under a held lock. The dispatcher drains
    /// the returned `Changes` into a durable commit on the command's
    /// branch. Boxed so the trait stays dyn-safe for
    /// `Box<dyn CommandHandler>` storage.
    fn run(&self, facts: &EntityFacts, source: &Source) -> RunFuture;
}

/// The `of` (entity) shared by a transient entity's facts. Every
/// artifact in an [`EntityFacts`] carries the same `of` (they were
/// grouped by it), so the first one names the entity.
fn facts_entity(facts: &EntityFacts) -> Option<Entity> {
    facts.first().map(|artifact| artifact.of.clone())
}

/// A handler whose trigger is the typed command concept `C`, running
/// the captured `async fn` `F` whose declared capabilities are `Args`.
/// Erases all three behind [`CommandHandler`]: `matches` is the derived
/// decode of `C`, `trigger_attributes` comes off `C`'s descriptor, and
/// `run` decodes, extracts the capabilities from the source, then
/// invokes `F`.
pub struct TypedHandler<C, Args, F> {
    attributes: Vec<String>,
    handler: F,
    _command: std::marker::PhantomData<fn() -> (C, Args)>,
}

impl<C, Args, F> TypedHandler<C, Args, F>
where
    C: Command,
    F: CommandFn<C, Args>,
{
    /// Build a handler for command `C` from the `async fn` to run.
    /// Caches `C`'s trigger attribute names.
    pub fn new(handler: F) -> Self {
        Self {
            attributes: C::trigger_attributes(),
            handler,
            _command: std::marker::PhantomData,
        }
    }
}

impl<C, Args, F> CommandHandler for TypedHandler<C, Args, F>
where
    C: Command,
    Args: 'static,
    F: CommandFn<C, Args> + ConditionalSync,
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

    fn run(&self, facts: &EntityFacts, source: &Source) -> RunFuture {
        // Decode the trigger and extract capabilities from the source
        // synchronously, here, while the caller still holds the lock.
        // `self.handler.run(...)` returns a `'static` `HandlerFuture`
        // (it owns the trigger, buffer, and capability clones), so the
        // future below borrows neither `self` nor `source` — the
        // dispatcher can drop the lock before awaiting it. `None`
        // (didn't decode) yields no outcome.
        let decoded = facts_entity(facts).and_then(|this| C::decode(this, facts));
        let handler_future =
            decoded.map(|command| self.handler.run(command, Transaction::new(), source));
        Box::pin(async move {
            match handler_future {
                Some(future) => Some(future.await.into_changes()),
                None => None,
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

    /// Register a typed handler for command `C`, axum `.route`-style.
    /// `C` is inferred from the handler's first parameter; declared
    /// capabilities (`State<…>`) are inferred as `Args`. Chainable.
    ///
    /// ```ignore
    /// let registry = CommandRegistry::new()
    ///     .command(create_repo)   // async fn(CreateRepo, State<AppState>, Transaction) -> Transaction
    ///     .command(rename_space);
    /// ```
    pub fn command<C, Args, F>(mut self, handler: F) -> Self
    where
        C: Command + 'static,
        Args: 'static,
        F: CommandFn<C, Args> + ConditionalSync,
    {
        self.register(Box::new(TypedHandler::<C, Args, F>::new(handler)));
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

    // A real command concept: the typed trigger a handler reads
    // through `&CreateRepo`. Plain derives — no marker trait — so the
    // blanket `Command` impl (and `TypedHandler`) apply for free.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.command")]
    pub struct RepoName(pub String);

    #[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
    pub struct CreateRepo {
        pub this: Entity,
        pub name: RepoName,
    }

    /// The attribute `CreateRepo`'s `name` field stores under — the
    /// `RepoName` attribute's name (snake-cased from its struct name),
    /// which is what the trigger reverse-index keys on.
    fn create_repo_name_attr() -> dialog_query::attribute::The {
        the!("xyz.tonk.command/repo-name")
    }

    /// A one-fact transient `CreateRepo{this: of, name}` batch.
    fn create_repo_transient(of: &str, name: &str) -> Changes {
        let mut changes = Changes::new();
        create_repo_name_attr()
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

    // An outcome concept the test handler asserts: `Created{this}`.
    // Stands in for a real `StatusChange`/`Failed` outcome.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.command")]
    pub struct CreatedName(pub String);

    #[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
    pub struct Created {
        pub this: Entity,
        pub name: CreatedName,
    }

    /// A test handler: on a `CreateRepo`, assert a `Created` outcome
    /// echoing the requested name. A pure transform — no IO — proving
    /// the decode → run → outcome path.
    async fn record_created(command: CreateRepo, mut tx: Transaction) -> Transaction {
        tx.assert(Created {
            this: command.this.clone(),
            name: CreatedName(command.name.0.clone()),
        });
        tx
    }

    /// Build a `TypedHandler` over [`record_created`] (arity-0 `Args`
    /// — a pure transform, no declared capability).
    fn created_handler() -> Box<dyn CommandHandler> {
        Box::new(TypedHandler::<CreateRepo, (), _>::new(record_created))
    }

    #[dialog_common::test]
    fn it_matches_a_handler_by_its_trigger_concept() {
        let mut registry = CommandRegistry::new();
        registry.register(created_handler());

        let changes = create_repo_transient("did:key:zCmd", "pictures");
        assert_eq!(
            registry.match_transients(&changes).len(),
            1,
            "the CreateRepo trigger should fire once"
        );
    }

    #[dialog_common::test]
    fn it_ignores_a_transient_no_handler_triggers_on() {
        let mut registry = CommandRegistry::new();
        registry.register(created_handler());

        let mut changes = Changes::new();
        the!("xyz.tonk.unrelated/noise")
            .of("did:key:zNoise".parse::<Entity>().expect("entity URI"))
            .is("x".to_string())
            .assert(&mut changes);
        assert!(
            registry.match_transients(&changes).is_empty(),
            "an unrelated transient should not fire"
        );
    }

    #[dialog_common::test]
    fn it_fires_every_matching_handler_subscription_style() {
        // Two handlers on the same command: both fire (commands are
        // subscription-like, no tiebreak).
        let mut registry = CommandRegistry::new();
        registry.register(created_handler());
        registry.register(created_handler());

        let changes = create_repo_transient("did:key:zCmd", "pictures");
        assert_eq!(
            registry.match_transients(&changes).len(),
            2,
            "both handlers on the command fire"
        );
    }

    #[dialog_common::test]
    fn it_reports_empty_until_a_handler_is_registered() {
        let registry = CommandRegistry::new();
        assert!(registry.is_empty(), "a fresh registry has no handlers");
    }

    #[dialog_common::test]
    async fn it_runs_a_pure_handler_and_returns_the_outcome_changes() {
        // A pure handler (arity-0) ignores the source, so we can run it
        // directly by invoking the captured fn — the `CommandHandler::run`
        // path that takes a `Source` is exercised end-to-end at the
        // router layer where a real `AppState` exists.
        let by_entity = group_by_entity(create_repo_transient("did:key:zCmd", "pictures"));
        let (entity, facts) = by_entity.into_iter().next().expect("one entity");
        let command = CreateRepo::decode(entity, &facts).expect("decodes");

        let tx = record_created(command, Transaction::new()).await;
        let names: Vec<String> = tx
            .into_changes()
            .into_instructions()
            .into_iter()
            .filter_map(|inst| match inst {
                Instruction::Assert(a) | Instruction::Replace(a) => match a.is {
                    dialog_artifacts::Value::String(s) => Some(s),
                    _ => None,
                },
                Instruction::Retract(_) => None,
            })
            .collect();
        assert_eq!(
            names,
            vec!["pictures".to_string()],
            "handler should have asserted a Created outcome echoing the name"
        );
    }
}
