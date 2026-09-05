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

use dialog_artifacts::{Artifact, Changes, Entity, Instruction};
use dialog_capability::{Command, Provider};
use dialog_common::ConditionalSync;
use dialog_query::concept::Concept;
use dialog_query::{Application, ConceptDescriptor, Conclusion, Descriptor, Match, Term};

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

/// A command's decode surface: the shape it has **now**, plus one
/// deprecated predecessor that converts into it.
///
/// A command is matched structurally — the set of attribute names a
/// transient carries is its whole identity. That makes changing a
/// command's shape a compatibility problem: a branch seeded before the
/// change still asserts the old attributes, and a handler keyed on the
/// new ones stops firing for it.
///
/// This is the seam that makes such a change survivable without the old
/// shape becoming permanent. A handler declares
/// `Migrated<Current, Legacy>`; the registry indexes it under the union
/// of both attribute sets, so either shape reaches it, and
/// [`decode`](Self::decode) hands the handler a `Current` either way —
/// the legacy shape arriving through `Legacy: Into<Current>`.
///
/// The handler is therefore written against `Current` only. Retiring the
/// legacy shape is deleting its concept, its `From` impl, and the second
/// type parameter here: no handler body changes.
///
/// `Current` is tried first, so a transient that satisfies both shapes
/// (one carrying every attribute of each) decodes as the current one.
///
/// ```no_run
/// # use dialog_reactor::Migrated;
/// # use dialog_artifacts::Entity;
/// # use dialog_query::{Attribute, Concept};
/// # #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
/// # #[domain("xyz.example.rename")] pub struct Name(pub String);
/// # #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
/// # #[domain("dom.event.current-target")] pub struct Value(pub String);
/// # #[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
/// # pub struct Rename { pub this: Entity, pub name: Name }
/// # #[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
/// # pub struct LegacyRename { pub this: Entity, pub value: Value }
/// impl From<LegacyRename> for Rename {
///     fn from(old: LegacyRename) -> Self {
///         Self { this: old.this, name: Name(old.value.0) }
///     }
/// }
///
/// let command: Migrated<Rename, LegacyRename> = Migrated::new();
/// // Indexed under both `xyz.example.rename/name` and
/// // `dom.event.current-target/value`.
/// assert_eq!(command.trigger_attributes().len(), 2);
/// ```
pub struct Migrated<Current, Legacy> {
    attributes: Vec<String>,
    _shapes: std::marker::PhantomData<fn() -> (Current, Legacy)>,
}

impl<Current, Legacy> Migrated<Current, Legacy>
where
    Current: Decode,
    Legacy: Decode + Into<Current>,
{
    /// Cache the union of both shapes' trigger attributes, so the
    /// registry indexes the handler under everything either shape can
    /// assert.
    pub fn new() -> Self {
        let mut attributes = Current::trigger_attributes();
        for attribute in Legacy::trigger_attributes() {
            if !attributes.contains(&attribute) {
                attributes.push(attribute);
            }
        }
        Self {
            attributes,
            _shapes: std::marker::PhantomData,
        }
    }

    /// The attribute names that make a transient a candidate: every one
    /// either shape carries.
    pub fn trigger_attributes(&self) -> &[String] {
        &self.attributes
    }

    /// Decode one transient entity's facts as `Current`, converting a
    /// legacy shape when that is what arrived. `None` when the facts
    /// satisfy neither.
    pub fn decode(&self, facts: &EntityFacts) -> Option<Current> {
        let this = facts_entity(facts)?;
        Current::decode(this.clone(), facts).or_else(|| Legacy::decode(this, facts).map(Into::into))
    }

    /// Whether these facts decode as either shape.
    pub fn matches(&self, facts: &EntityFacts) -> bool {
        self.decode(facts).is_some()
    }
}

impl<Current, Legacy> Default for Migrated<Current, Legacy>
where
    Current: Decode,
    Legacy: Decode + Into<Current>,
{
    fn default() -> Self {
        Self::new()
    }
}

/// A `'static` boxed future for one command's execution, `Send` only
/// off wasm — matches the reactor's
/// [`ConditionalSync`](dialog_common::ConditionalSync) convention so a
/// command runs on the single-threaded SW executor. `'static` so the
/// dispatcher can build it, release the state lock, then await it —
/// command IO never runs while a lock is held.
#[cfg(not(target_arch = "wasm32"))]
pub type RunFuture = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>>;
/// A `'static` boxed future for one command's execution (the wasm
/// single-threaded variant — no `Send` bound). See the native variant
/// above for the rationale.
#[cfg(target_arch = "wasm32")]
pub type RunFuture = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + 'static>>;

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
pub trait CommandHandler<Env>: ConditionalSync {
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
    fn run(&self, facts: &EntityFacts, env: &Env) -> RunFuture;
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
pub struct TypedCommand<C, Env> {
    attributes: Vec<String>,
    _command: std::marker::PhantomData<fn() -> (C, Env)>,
}

impl<C, Env> TypedCommand<C, Env>
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

impl<C, Env> Default for TypedCommand<C, Env>
where
    C: Decode + Command<Input = C> + 'static,
    Env: Provider<C>,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<C, Env> CommandHandler<Env> for TypedCommand<C, Env>
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

/// A registry entry for a command whose shape changed: runs the
/// [`Provider<Current>`](dialog_capability::Provider) for either the
/// current shape or the deprecated predecessor that converts into it.
///
/// The [`TypedCommand`] sibling for [`Migrated`]. The provider is
/// implemented for `Current` alone — `Legacy` reaches it through
/// `Into<Current>`, so nothing downstream of decode knows the old shape
/// exists, and retiring it touches only the registration.
pub struct MigratedCommand<Current, Legacy, Env> {
    command: Migrated<Current, Legacy>,
    _env: std::marker::PhantomData<fn() -> Env>,
}

impl<Current, Legacy, Env> MigratedCommand<Current, Legacy, Env>
where
    Current: Decode + Command<Input = Current> + 'static,
    Legacy: Decode + Into<Current> + 'static,
    Env: Provider<Current>,
{
    /// Register `Current`, also accepting `Legacy` on its behalf. The
    /// `Env: Provider<Current>` bound is the same compile-time
    /// capability gate [`TypedCommand::new`] applies — the legacy shape
    /// grants no capability of its own.
    pub fn new() -> Self {
        Self {
            command: Migrated::new(),
            _env: std::marker::PhantomData,
        }
    }
}

impl<Current, Legacy, Env> Default for MigratedCommand<Current, Legacy, Env>
where
    Current: Decode + Command<Input = Current> + 'static,
    Legacy: Decode + Into<Current> + 'static,
    Env: Provider<Current>,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<Current, Legacy, Env> CommandHandler<Env> for MigratedCommand<Current, Legacy, Env>
where
    Current: Decode + Command<Input = Current, Output = ()> + ConditionalSync + 'static,
    Legacy: Decode + Into<Current> + ConditionalSync + 'static,
    Env: Provider<Current> + Clone + ConditionalSync + 'static,
{
    fn trigger_attributes(&self) -> &[String] {
        self.command.trigger_attributes()
    }

    fn matches(&self, facts: &EntityFacts) -> bool {
        self.command.matches(facts)
    }

    fn run(&self, facts: &EntityFacts, env: &Env) -> RunFuture {
        // Decode (and convert) synchronously, as `TypedCommand` does —
        // the caller still holds the lock.
        let decoded = self.command.decode(facts);
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
pub struct CommandRegistry<Env> {
    /// Registered handlers, owned. Indices into this vec are the
    /// values in [`Self::by_attribute`].
    handlers: Vec<Box<dyn CommandHandler<Env>>>,
    /// `attribute name → handler indices`. A transient touching this
    /// attribute makes every listed handler a candidate.
    by_attribute: HashMap<String, Vec<usize>>,
}

impl<Env> Default for CommandRegistry<Env> {
    fn default() -> Self {
        Self {
            handlers: Vec::new(),
            by_attribute: HashMap::new(),
        }
    }
}

impl<Env> CommandRegistry<Env> {
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
        self.register(Box::new(TypedCommand::<C, Env>::new()));
        self
    }

    /// Register the command type `Current`, also accepting the
    /// deprecated `Legacy` shape on its behalf and converting it. The
    /// [`Self::command`] counterpart for a command whose shape changed;
    /// see [`Migrated`]. Chainable.
    pub fn migrated<Current, Legacy>(mut self) -> Self
    where
        Current: Decode + Command<Input = Current, Output = ()> + ConditionalSync + 'static,
        Legacy: Decode + Into<Current> + ConditionalSync + 'static,
        Env: Provider<Current> + Clone + ConditionalSync + 'static,
    {
        self.register(Box::new(MigratedCommand::<Current, Legacy, Env>::new()));
        self
    }

    /// Register a boxed handler, indexing it by each of its trigger
    /// attribute names. Prefer [`Self::command`] for typed handlers.
    pub fn register(&mut self, handler: Box<dyn CommandHandler<Env>>) {
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
    ) -> Vec<(&'a dyn CommandHandler<Env>, EntityFacts)> {
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

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    use super::*;
    use dialog_artifacts::{Statement, Value};
    use dialog_query::{Attribute, Concept, the};

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
        use tonk_schema::command::{CreateSpace, legacy};

        let this = entity("did:key:zCreateSpace");
        let mut changes = Changes::new();
        the!("dom.event.current-target.elements.name/value")
            .of(this.clone())
            .is("pictures".to_string())
            .assert(&mut changes);
        let (_, facts) = facts_for(changes);

        // Through `Migrated`, exactly as the handler decodes it: the
        // attribute here is the DOM read path an older descriptor still
        // asserts, and it must arrive as the current shape.
        let decoded = Migrated::<CreateSpace, legacy::CreateSpace>::new()
            .decode(&facts)
            .expect("CreateSpace must decode from name-only facts (older descriptor / blank form)");
        assert_eq!(decoded.name.0, "pictures");
    }

    /// The Hub's per-row delete confirm asserts a `space/remove`
    /// transient carrying only `data-remove` (the subject DID).
    #[dialog_common::test]
    fn it_decodes_remove_space_from_a_data_remove_fact() {
        use tonk_schema::command::{RemoveSpace, legacy};

        let this = entity("did:key:zRemoveSpace");
        let subject = entity("did:key:zSpaceSubject");
        let mut changes = Changes::new();
        the!("dom.event.current-target.dataset/remove")
            .of(this.clone())
            .is(subject.clone())
            .assert(&mut changes);
        let (_, facts) = facts_for(changes);

        let decoded = Migrated::<RemoveSpace, legacy::RemoveSpace>::new()
            .decode(&facts)
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
        use tonk_schema::command::{RenameRepository, legacy};

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
        let (_, facts) = facts_for(changes);

        let decoded = Migrated::<RenameRepository, legacy::RenameRepository>::new()
            .decode(&facts)
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
        use tonk_schema::command::{RemoveSpace, legacy};

        let mut changes = Changes::new();
        the!("dom.event.current-target.dataset/subject")
            .of(entity("did:key:zRename"))
            .is(entity("did:key:zSpaceSubject"))
            .assert(&mut changes);
        the!("dom.event.current-target/value")
            .of(entity("did:key:zRename"))
            .is("new name".to_string())
            .assert(&mut changes);
        let (_, facts) = facts_for(changes);

        assert!(
            !Migrated::<RemoveSpace, legacy::RemoveSpace>::new().matches(&facts),
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
    /// disambiguated anything. The fix was a DISTINCT ATTRIBUTE per
    /// command — the marker `dataset/rename-repository` this transient
    /// still carries.
    ///
    /// The current shapes need no marker: each command's fields live in
    /// its own `xyz.tonk.command.<verb>` namespace, so the shapes are
    /// disjoint by construction (`tonk-worker/tests/command_migration.rs`
    /// pins that). What this test still pins is that the DEPRECATED
    /// shapes stay disjoint too — a branch seeded before the migration is
    /// exactly where this bug would come back.
    #[dialog_common::test]
    fn it_does_not_decode_a_repo_rename_as_a_profile_rename() {
        use tonk_schema::command::{ProfileRename, RenameRepository, legacy};

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
        let (_, facts) = facts_for(changes);

        assert!(
            Migrated::<RenameRepository, legacy::RenameRepository>::new().matches(&facts),
            "a repo-rename transient must decode as RenameRepository"
        );
        assert!(
            !Migrated::<ProfileRename, legacy::ProfileRename>::new().matches(&facts),
            "a repo-rename transient must NOT also decode as ProfileRename — \
             that is the bug: renaming a space was also renaming the profile"
        );
    }

    /// Converse of the above: a profile-rename transient (no `space`, and
    /// the `profile/rename` marker attribute) must not decode as
    /// `RenameRepository`, which requires `space` regardless.
    #[dialog_common::test]
    fn it_does_not_decode_a_profile_rename_as_a_repo_rename() {
        use tonk_schema::command::{ProfileRename, RenameRepository, legacy};

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
        let (_, facts) = facts_for(changes);

        assert!(
            Migrated::<ProfileRename, legacy::ProfileRename>::new().matches(&facts),
            "a profile-rename transient must decode as ProfileRename"
        );
        assert!(
            !Migrated::<RenameRepository, legacy::RenameRepository>::new().matches(&facts),
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
    // A stub `CommandHandler` lets us exercise the reverse-index match and
    // the "all matches fire" / "candidate but doesn't decode" edges
    // without a real `Provider` env (which lives at the router layer).

    // A trivial env for the registry/match tests. The matching path
    // never touches the env (only `run` does, which these tests don't
    // call), so a unit type is enough to pin the `Env` parameter.
    #[derive(Clone)]
    struct TestEnv;

    struct StubHandler {
        attributes: Vec<String>,
        // Decode succeeds only when the entity carries this attribute.
        decode_on: String,
    }

    impl StubHandler {
        fn boxed(attribute: &str) -> Box<dyn CommandHandler<TestEnv>> {
            Box::new(StubHandler {
                attributes: vec![attribute.to_string()],
                decode_on: attribute.to_string(),
            })
        }
    }

    impl CommandHandler<TestEnv> for StubHandler {
        fn trigger_attributes(&self) -> &[String] {
            &self.attributes
        }
        fn matches(&self, facts: &EntityFacts) -> bool {
            facts.iter().any(|a| a.the.to_string() == self.decode_on)
        }
        fn run(&self, _facts: &EntityFacts, _env: &TestEnv) -> RunFuture {
            Box::pin(async {})
        }
    }

    #[dialog_common::test]
    fn it_reports_empty_until_a_command_is_registered() {
        let mut registry = CommandRegistry::new();
        assert!(registry.is_empty());
        registry.register(StubHandler::boxed("xyz.tonk.command/repo-name"));
        assert!(!registry.is_empty());
    }

    #[dialog_common::test]
    fn it_matches_a_registered_command_on_its_trigger() {
        let mut registry = CommandRegistry::new();
        registry.register(StubHandler::boxed("xyz.tonk.command/repo-name"));
        let fired = registry.match_transients(&create_repo_transient("did:key:zCmd", "pics"));
        assert_eq!(fired.len(), 1);
    }

    #[dialog_common::test]
    fn it_does_not_match_an_unrelated_transient() {
        let mut registry = CommandRegistry::new();
        registry.register(StubHandler::boxed("xyz.tonk.command/repo-name"));
        let mut changes = Changes::new();
        noise_fact(&mut changes, "did:key:zNoise");
        assert!(registry.match_transients(&changes).is_empty());
    }

    #[dialog_common::test]
    fn it_fires_all_matching_commands_subscription_style() {
        // Two commands registered on the same trigger attribute: BOTH
        // fire (commands are subscription-like, no tiebreak).
        let mut registry = CommandRegistry::new();
        registry.register(StubHandler::boxed("xyz.tonk.command/repo-name"));
        registry.register(StubHandler::boxed("xyz.tonk.command/repo-name"));
        let fired = registry.match_transients(&create_repo_transient("did:key:zCmd", "pics"));
        assert_eq!(fired.len(), 2, "both commands on the attribute fire");
    }

    #[dialog_common::test]
    fn it_drops_a_candidate_whose_trigger_does_not_decode() {
        // A command keyed on an attribute the entity has, but whose
        // `matches` (decode) returns false, is a candidate but does NOT
        // fire. We register a stub keyed on `repo-name` but that only
        // decodes on `owner`, then submit a `repo-name`-only entity.
        let mut registry = CommandRegistry::new();
        registry.register(Box::new(StubHandler {
            attributes: vec!["xyz.tonk.command/repo-name".to_string()],
            decode_on: "xyz.tonk.command/owner".to_string(),
        }));
        let fired = registry.match_transients(&create_repo_transient("did:key:zCmd", "pics"));
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
        registry.register(Box::new(StubHandler {
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
        let fired = registry.match_transients(&changes);
        assert_eq!(fired.len(), 1, "the command fires once, not per-attribute");
    }

    #[dialog_common::test]
    fn it_matches_each_command_entity_in_a_batch() {
        // Two distinct command entities in one batch each match.
        let mut registry = CommandRegistry::new();
        registry.register(StubHandler::boxed("xyz.tonk.command/repo-name"));
        let mut changes = create_repo_transient("did:key:zA", "alpha");
        the!("xyz.tonk.command/repo-name")
            .of(entity("did:key:zB"))
            .is("beta".to_string())
            .assert(&mut changes);
        let fired = registry.match_transients(&changes);
        assert_eq!(fired.len(), 2, "each command entity fires once");
    }

    #[dialog_common::test]
    fn it_hands_each_match_the_full_entity_facts() {
        // The facts handed to a matched handler are that entity's facts
        // (so the handler can decode every field).
        let mut registry = CommandRegistry::new();
        registry.register(StubHandler::boxed("xyz.tonk.command/repo-name"));
        let fired = registry.match_transients(&create_repo_transient("did:key:zCmd", "pics"));
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

    // --- migrated commands ----------------------------------------------
    // A command's shape is the set of attributes it carries, so changing
    // the shape breaks every branch seeded before the change. `Migrated`
    // is how a handler accepts both without the old shape becoming
    // permanent: the handler only ever sees `Current`.

    /// The DOM-shaped predecessor of [`CreateRepo`]: same command, but
    /// its field is the DOM read path the form used to post. Its own
    /// module so the struct can be named `Value` — the attribute's last
    /// segment IS the struct name, and the read path ends `name/value`.
    pub mod form {
        use dialog_query::Attribute;

        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("dom.event.current-target.elements.name")]
        pub struct Value(pub String);
    }

    #[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
    pub struct LegacyCreateRepo {
        pub this: Entity,
        pub name: form::Value,
    }

    impl From<LegacyCreateRepo> for CreateRepo {
        fn from(legacy: LegacyCreateRepo) -> Self {
            Self {
                this: legacy.this,
                name: RepoName(legacy.name.0),
            }
        }
    }

    fn legacy_create_repo_transient(of: &str, name: &str) -> Changes {
        let mut changes = Changes::new();
        the!("dom.event.current-target.elements.name/value")
            .of(entity(of))
            .is(name.to_string())
            .assert(&mut changes);
        changes
    }

    #[dialog_common::test]
    fn a_migrated_command_is_indexed_under_both_shapes() {
        let command: Migrated<CreateRepo, LegacyCreateRepo> = Migrated::new();
        let attributes: Vec<&str> = command
            .trigger_attributes()
            .iter()
            .map(String::as_str)
            .collect();
        assert!(attributes.contains(&"xyz.tonk.command/repo-name"));
        assert!(attributes.contains(&"dom.event.current-target.elements.name/value"));
    }

    #[dialog_common::test]
    fn a_migrated_command_decodes_the_current_shape() {
        let command: Migrated<CreateRepo, LegacyCreateRepo> = Migrated::new();
        let (_, facts) = facts_for(create_repo_transient("did:key:zCmd", "pictures"));
        let decoded = command.decode(&facts).expect("the current shape decodes");
        assert_eq!(decoded.name.0, "pictures");
    }

    #[dialog_common::test]
    fn a_migrated_command_converts_the_legacy_shape() {
        // The whole point: a branch seeded before the change still posts
        // the DOM-shaped transient, and the handler still receives a
        // `CreateRepo` — it never learns the old shape exists.
        let command: Migrated<CreateRepo, LegacyCreateRepo> = Migrated::new();
        let (_, facts) = facts_for(legacy_create_repo_transient("did:key:zCmd", "pictures"));
        let decoded = command.decode(&facts).expect("the legacy shape converts");
        assert_eq!(decoded.name.0, "pictures");
    }

    #[dialog_common::test]
    fn a_migrated_command_prefers_the_current_shape() {
        // A transient carrying both (a page mid-migration, or a branch
        // reseeded while a tab held the old descriptor) decodes as the
        // current shape rather than round-tripping through the
        // conversion.
        let command: Migrated<CreateRepo, LegacyCreateRepo> = Migrated::new();
        let mut changes = create_repo_transient("did:key:zCmd", "current");
        the!("dom.event.current-target.elements.name/value")
            .of(entity("did:key:zCmd"))
            .is("legacy".to_string())
            .assert(&mut changes);
        let (_, facts) = facts_for(changes);
        assert_eq!(command.decode(&facts).expect("decodes").name.0, "current");
    }

    #[dialog_common::test]
    fn a_migrated_command_rejects_facts_matching_neither_shape() {
        let command: Migrated<CreateRepo, LegacyCreateRepo> = Migrated::new();
        let mut changes = Changes::new();
        noise_fact(&mut changes, "did:key:zCmd");
        let (_, facts) = facts_for(changes);
        assert!(!command.matches(&facts));
    }

    #[dialog_common::test]
    fn the_registry_fires_a_migrated_handler_for_either_shape() {
        // End to end through the registry: both transients reach the
        // same registered handler.
        let mut registry = CommandRegistry::<()>::new();
        registry.register(Box::new(MigratedStub {
            command: Migrated::<CreateRepo, LegacyCreateRepo>::new(),
        }));

        for changes in [
            create_repo_transient("did:key:zCmd", "pictures"),
            legacy_create_repo_transient("did:key:zCmd", "pictures"),
        ] {
            assert_eq!(
                registry.match_transients(&changes).len(),
                1,
                "both shapes reach the migrated handler"
            );
        }
    }

    /// A handler that only knows `Migrated` — the shape every real
    /// command handler takes once migrated.
    struct MigratedStub {
        command: Migrated<CreateRepo, LegacyCreateRepo>,
    }

    impl CommandHandler<()> for MigratedStub {
        fn trigger_attributes(&self) -> &[String] {
            self.command.trigger_attributes()
        }

        fn matches(&self, facts: &EntityFacts) -> bool {
            self.command.matches(facts)
        }

        fn run(&self, _facts: &EntityFacts, _env: &()) -> RunFuture {
            Box::pin(async {})
        }
    }
}
