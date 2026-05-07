//! Cross-cutting metadata attributes in the `dialog.meta` domain.
//!
//! These attributes attach human-authored metadata — a display name,
//! a free-form description — to any entity in the database. They are
//! deliberately scoped to the `dialog.meta` domain (rather than
//! `xyz.tonk`) so that facts written by tonk and facts written by
//! other dialog tooling — notably the `carry` CLI — name and
//! describe the same entities through the same relations and stay
//! mutually queryable.

// `#[derive(Attribute)]` expands to helper items without doc
// comments; suppress the crate-level `missing_docs` lint here.
#![allow(missing_docs)]

use dialog_artifacts::Entity;
use dialog_query::{Attribute, Concept};

/// Human-readable name for any entity.
///
/// Stored as `dialog.meta/name` with cardinality `one`. Used to
/// label attribute and concept entities defined in user-authored
/// schemas, but applicable to any entity that benefits from a
/// display name.
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("dialog.meta")]
pub struct Name(pub String);

/// Human-readable description for any entity.
///
/// Stored as `dialog.meta/description` with cardinality `one`.
/// Conventionally a short prose paragraph explaining what the
/// entity represents; not used in identity derivation by either
/// `dialog_query::AttributeDescriptor` or
/// `dialog_query::ConceptDescriptor`, so changing it is safe and
/// does not fork the entity.
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("dialog.meta")]
pub struct Description(pub String);

/// Newtypes for the `dialog.attribute` namespace.
///
/// Submodule so each newtype's *struct name* ends up
/// kebab-cased into the right relation slot (`Id` →
/// `dialog.attribute/id`, etc.) without colliding with the
/// `Attribute` derive trait re-imported by anyone using
/// `tonk_schema::meta::*`.
pub mod attribute {
    use super::Attribute;

    /// The selector value of an attribute entity —
    /// `dialog.attribute/id`. Carries the human-readable
    /// `domain/name` form (e.g. `io.gozala.person/name`); one
    /// claim per attribute entity, cardinality `one`. Written by
    /// the interpreter so "find the attribute entity for
    /// selector `xyz/foo`" runs as a normal EAV match.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("dialog.attribute")]
    pub struct Id(pub String);

    /// The dialog `Type` discriminant of an attribute entity —
    /// `dialog.attribute/type`. The string form (e.g. `"Text"`,
    /// `"UnsignedInteger"`) is what
    /// `dialog_query::AttributeDescriptor` round-trips through
    /// serde, not the underlying `ValueDataType` variant name.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("dialog.attribute")]
    pub struct Type(pub String);

    /// The cardinality of an attribute entity —
    /// `dialog.attribute/cardinality`. Takes `"one"` or
    /// `"many"`; the textual form matches what
    /// `dialog_query::Cardinality` serialises to.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("dialog.attribute")]
    pub struct Cardinality(pub String);
}

/// A typed view over an entity that carries a name.
///
/// `Named` exists so callers can query "find me the entity with
/// `dialog.meta/name = X`" through dialog's typed concept-query
/// API instead of dropping into raw `ArtifactSelector` calls.
/// Two-field concept on purpose: anything more would be carry-
/// flavoured policy that doesn't belong here.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Named {
    /// The named entity.
    pub this: Entity,
    /// The name attached as `dialog.meta/name`.
    pub name: Name,
}

/// A typed view over an attribute entity carrying the
/// always-present fact set: `id`, `type`, `cardinality`,
/// `description`. Matches every attribute on a branch
/// regardless of whether the user gave it a bookmark name.
///
/// Use this when you need an attribute by entity URI and
/// don't care whether it was named — for example, when
/// reconstructing a [`dialog_query::AttributeDescriptor`] for
/// an attribute referenced by URI from inside a `concept!`
/// definition.
///
/// `description` is required — entries without one receive an
/// empty string at write time, so the schema-level invariant
/// "every stored attribute has a description claim" holds.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AnonymousAttribute {
    /// The attribute entity (a `the:…` URI).
    pub this: Entity,
    /// Selector — `domain/name` form.
    pub id: attribute::Id,
    /// Value-type descriptor name.
    pub r#type: attribute::Type,
    /// `"one"` or `"many"`.
    pub cardinality: attribute::Cardinality,
    /// Human-readable description.
    pub description: Description,
}

/// Same as [`AnonymousAttribute`] but with the `name` field —
/// the published name the attribute was registered under.
/// Matches only attributes the user explicitly anchored
/// (`attribute!: &foo` syntax); anonymous and variable-bound
/// attributes are excluded.
///
/// Use this when you need to surface "what name did the user
/// give this attribute" — for example, in the editor's
/// attribute list, or to resolve a bare-symbol reference.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct NamedAttribute {
    /// The attribute entity (a `the:…` URI).
    pub this: Entity,
    /// Selector — `domain/name` form.
    pub id: attribute::Id,
    /// Value-type descriptor name.
    pub r#type: attribute::Type,
    /// `"one"` or `"many"`.
    pub cardinality: attribute::Cardinality,
    /// Human-readable description.
    pub description: Description,
    /// The bookmark name this attribute was registered under.
    pub name: Name,
}
