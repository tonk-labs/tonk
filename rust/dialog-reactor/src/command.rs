//! Command decode — the typed-Rust sibling of the declarative
//! `rule!:` effects in [`tonk_evaluator::effects`].
//!
//! A *command* is a transient [`Concept`] asserted to trigger an
//! effect. The induce loop already dispatches transients to
//! declarative rules (body query → head assertion); commands extend
//! that to effects that run arbitrary `async` Rust — repo creation,
//! key generation, network IO — which a rule body can't express.
//!
//! The commands a dispatch lands in a stack's `control:` layer are run
//! by the stack's own providers
//! ([`dialog_repository::Provider`]), one per concept, registered on
//! the stack by the embedder. What this module keeps is the decode
//! seam a provider's handler runs: a landed command is its entity's
//! rows, and [`Decode`] turns those rows into the typed command the
//! [`dialog_capability::Provider`] impl executes, reusing the derived
//! query machinery rather than reimplementing field decode. A
//! command whose shape changed decodes through [`Migrated`], which
//! accepts the deprecated shape on the current one's behalf.
//!
//! The asking client, and the branch a command was sent for when it
//! was sent through another branch's stack, are rows of the `control:`
//! entity itself for the tick ([`control`]), written `.at(control_layer())`
//! beside the commands, so a rule can join on them and a provider's
//! embedder reads them from the tick.

use std::collections::HashMap;

use dialog_corpus::{Entity, Fact};
use dialog_query::concept::Concept;
use dialog_query::{Application, ConceptDescriptor, Conclusion, Descriptor, Match, Term};

/// The `tonk.session/*` rows of the `control:` entity for one tick,
/// written `.at(control_layer())` with the tick's commands so they land
/// in the same tick: who asked, and for which branch. They are rows of
/// the channel's own entity, not of any command, so a rule premised on a
/// command joins them by the tick and a provider reads them from the
/// tick's rows rather than the command's
/// ([`dialog_repository::Command::rows`] carries the command's alone).
pub mod control {
    use dialog_corpus::{Entity, Relation};

    /// `tonk.session/client`: the service-worker client the request that
    /// invoked the tick's commands came from, as the entity
    /// `client:<id>` ([`client_entity`]). A handler whose effect is a
    /// page capability (navigation) posts back to it, and a rule can
    /// name a channel of the same entity.
    pub fn client() -> Relation {
        dialog_query::the!("tonk.session/client").into()
    }

    /// The entity a client id is written as: `client:<id>`. `None` when
    /// the id does not make a URI.
    pub fn client_entity(id: &str) -> Option<Entity> {
        format!("client:{id}").parse().ok()
    }

    /// The client id a [`client_entity`] names, or `None` for another
    /// entity.
    pub fn client_id(entity: &Entity) -> Option<String> {
        entity
            .to_string()
            .strip_prefix("client:")
            .map(str::to_owned)
    }

    /// `tonk.session/repo`: the repository the tick's commands were sent
    /// for, when they were sent through another repository's stack: a
    /// command for a branch nobody has made yet goes through the
    /// profile's. Absent, the stack's own repository is the origin.
    pub fn repo() -> Relation {
        dialog_query::the!("tonk.session/repo").into()
    }

    /// `tonk.session/branch`: the branch the tick's commands were sent
    /// for, beside [`repo`].
    pub fn branch() -> Relation {
        dialog_query::the!("tonk.session/branch").into()
    }
}

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
pub fn decode_concept<C>(this: Entity, facts: &[Fact]) -> Option<C>
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
    // attributes); fields with no
    // matching fact stay unbound, so `realize` fails for a required
    // field — the right "no match".
    for artifact in facts {
        if let Some(field) = attribute_to_field.get(&artifact.the.to_string()) {
            source.bind(&Term::var(field), artifact.is.clone()).ok()?;
        }
    }

    query.realize(source).ok()
}

/// A transient trigger concept: the command type a
/// [`Provider`](dialog_capability::Provider) runs. Supplies the
/// concepts whose rows it decodes from — what a stack's provider claims
/// a landed command by — and the decode itself.
///
/// Blanket-implemented for any decodable [`Concept`]; the bounds are
/// exactly what [`decode_concept`] needs, so a plain
/// `#[derive(Concept)]` is `Decode` with no extra code. (Named `Decode`
/// rather than `Command` to leave that name to
/// [`dialog_capability::Command`], which a command type also implements.)
pub trait Decode: Sized {
    /// The concept descriptors this trigger decodes from: one for a
    /// plain concept, the current and the legacy shape for a
    /// [`Migrated`] one. A stack provider is registered for each
    /// ([`dialog_repository::Provider::new`]), and claims a landed
    /// command by the attributes of the concept's fields.
    fn concepts() -> Vec<&'static ConceptDescriptor>;

    /// Fully-qualified attribute URIs (`domain/name`) of this
    /// trigger's fields, over every concept of
    /// [`concepts`](Self::concepts): the shape a landed entity must
    /// carry for [`decode`](Self::decode) to succeed.
    fn trigger_attributes() -> Vec<String> {
        let mut attributes = Vec::new();
        for concept in Self::concepts() {
            for (_, attribute) in concept.with().iter() {
                let attribute = attribute.the().to_string();
                if !attributes.contains(&attribute) {
                    attributes.push(attribute);
                }
            }
        }
        attributes
    }

    /// Decode this trigger from one command entity's facts, or
    /// `None` if they don't satisfy it (missing/mistyped field).
    fn decode(this: Entity, facts: &[Fact]) -> Option<Self>;
}

impl<C> Decode for C
where
    C: Concept<Conclusion = C> + Conclusion + Descriptor<ConceptDescriptor>,
    C::Application: Default + Application<Conclusion = C>,
{
    fn concepts() -> Vec<&'static ConceptDescriptor> {
        // The derive exposes the concept's descriptor via the
        // `Descriptor<ConceptDescriptor>` trait (it no longer emits
        // `From<Query> for ConceptDescriptor`).
        vec![<C as Descriptor<ConceptDescriptor>>::descriptor()]
    }

    fn decode(this: Entity, facts: &[Fact]) -> Option<Self> {
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
/// `Migrated<Current, Legacy>`; it accepts either shape, and
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
/// # use dialog_corpus::Entity;
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
/// // Both `xyz.example.rename/name` and `dom.event.current-target/value`
/// // are shapes it accepts.
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
    /// Cache the union of both shapes' trigger attributes: everything
    /// either shape can assert.
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

    /// The concepts either shape decodes from: the current ones first.
    pub fn concepts() -> Vec<&'static ConceptDescriptor> {
        let mut concepts = Current::concepts();
        concepts.extend(Legacy::concepts());
        concepts
    }

    /// The attribute names either shape carries.
    pub fn trigger_attributes(&self) -> &[String] {
        &self.attributes
    }

    /// Decode one transient entity's facts as `Current`, converting a
    /// legacy shape when that is what arrived. `None` when the facts
    /// satisfy neither.
    pub fn decode(&self, facts: &[Fact]) -> Option<Current> {
        let this = facts_entity(facts)?;
        Current::decode(this.clone(), facts).or_else(|| Legacy::decode(this, facts).map(Into::into))
    }

    /// Whether these facts decode as either shape.
    pub fn matches(&self, facts: &[Fact]) -> bool {
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

/// The `of` (entity) shared by a transient entity's facts. Every
/// fact of a landed command carries the same `of` (they were grouped
/// by it), so the first one names the entity.
pub fn facts_entity(facts: &[Fact]) -> Option<Entity> {
    facts.first().map(|artifact| artifact.of.clone())
}
