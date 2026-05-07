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

/// The `dialog.meta/name` attribute — the published target a
/// name URI points at.
///
/// Cardinality `one`. Lives on the *name entity* (an `id:<n>`
/// URI), not on the named target. The value is the entity the
/// name currently identifies. Re-pointing a name to a different
/// entity is a cardinality-one supersession on this attribute.
///
/// This is the inverted shape of the pre-Stage-2 model, where
/// `dialog.meta/name` lived on the target with a string value
/// — see `transact::emit_name_assertion` for how the analyzer
/// emits the new direction from anchor-form heads.
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("dialog.meta")]
pub struct Name(pub Entity);

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

/// A typed view over an attribute entity carrying the
/// always-present fact set: `id`, `type`, `cardinality`,
/// `description`. Matches every attribute on a branch
/// regardless of whether the user gave it a published name.
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
