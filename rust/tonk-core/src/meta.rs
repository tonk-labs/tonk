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

/// Newtype for the attribute backing the [`Name`] concept's
/// `entity` field. Submodule so the struct name `Referent`
/// kebab-cases into `referent` (yielding the relation
/// `db.name/referent`) without colliding with [`Name`].
pub mod name {
    use super::{Attribute, Entity};

    /// The `db.name/referent` attribute — the entity a name
    /// currently points at. Cardinality `one` (the derive
    /// default), so re-pointing a name supersedes the prior
    /// claim instead of accumulating.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("db.name")]
    pub struct Referent(pub Entity);
}

/// A validated published name (`&anchor` in notation, the `name`
/// slot on a claim). Constructing one parses `id:<name>` into the
/// [`Entity`] it desugars to, so the parse happens exactly once, at
/// the boundary; everything downstream converts to the entity
/// infallibly via [`AnchorName::entity`]. An invalid name is a hard
/// error at construction (and at serde deserialization), never a
/// silently-dropped publication later.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct AnchorName {
    name: String,
    entity: Entity,
}

/// Why a string isn't a usable [`AnchorName`].
#[derive(Debug, thiserror::Error)]
#[error(
    "anchor name {name:?} can't be published — `id:{name}` is not a valid entity URI: {reason}"
)]
pub struct AnchorNameError {
    /// The rejected name.
    pub name: String,
    /// The underlying `id:<name>` parse error.
    pub reason: String,
}

impl AnchorName {
    /// The published name string (no `id:` prefix), as written
    /// after `&`.
    pub fn as_str(&self) -> &str {
        &self.name
    }
}

/// Validate a name by parsing `id:<name>` into its entity. The only
/// fallible step in the name's lifetime.
impl TryFrom<&str> for AnchorName {
    type Error = AnchorNameError;

    fn try_from(name: &str) -> Result<Self, Self::Error> {
        let entity = format!("id:{name}")
            .parse::<Entity>()
            .map_err(|e| AnchorNameError {
                name: name.to_owned(),
                reason: e.to_string(),
            })?;
        Ok(Self {
            name: name.to_owned(),
            entity,
        })
    }
}

impl TryFrom<String> for AnchorName {
    type Error = AnchorNameError;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        AnchorName::try_from(name.as_str())
    }
}

/// The `id:<name>` entity this name publishes against. Infallible:
/// the entity was parsed and stored when the [`AnchorName`] was
/// validated into existence.
impl From<AnchorName> for Entity {
    fn from(anchor: AnchorName) -> Self {
        anchor.entity
    }
}

impl From<&AnchorName> for Entity {
    fn from(anchor: &AnchorName) -> Self {
        anchor.entity.clone()
    }
}

/// Wire form is the bare name string (compatible with the prior
/// `Option<String>` name slot); `#[serde(into = "String")]` uses
/// this, and `#[serde(try_from = "String")]` validates on the way
/// back in.
impl From<AnchorName> for String {
    fn from(anchor: AnchorName) -> Self {
        anchor.name
    }
}

/// A user-published name — an `id:<n>` entity carrying a
/// single `entity` claim that points at the target the name
/// currently identifies.
///
/// `this` is derived from the name string by prefixing `id:`.
/// `entity` is the target. Asserting two `Name` claims for the
/// same `this` with different `entity` values supersedes the
/// prior claim because the backing attribute is cardinality
/// one.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Name {
    /// The name entity — `id:<n>` for user-published names,
    /// `db:<n>` for built-ins.
    pub this: Entity,
    /// The target this name currently identifies.
    pub entity: name::Referent,
}

/// Human-readable description for any entity.
///
/// Stored as `db.meta/description` with cardinality `one`.
/// Conventionally a short prose paragraph explaining what the
/// entity represents; not used in identity derivation by either
/// `dialog_query::AttributeDescriptor` or
/// `dialog_query::ConceptDescriptor`, so changing it is safe and
/// does not fork the entity.
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("db.meta")]
pub struct Description(pub String);

/// Newtypes for the `dialog.attribute` namespace.
///
/// Submodule so each newtype's *struct name* ends up
/// kebab-cased into the right relation slot (`Id` →
/// `db.attribute/id`, etc.) without colliding with the
/// `Attribute` derive trait re-imported by anyone using
/// `tonk_schema::meta::*`.
pub mod attribute {
    use super::Attribute;

    /// The selector value of an attribute entity —
    /// `db.attribute/id`. Carries the human-readable
    /// `domain/name` form (e.g. `io.gozala.person/name`); one
    /// claim per attribute entity, cardinality `one`. Written by
    /// the interpreter so "find the attribute entity for
    /// selector `xyz/foo`" runs as a normal EAV match.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("db.attribute")]
    pub struct Id(pub String);

    /// The dialog `Type` discriminant of an attribute entity —
    /// `db.attribute/type`. The string form (e.g. `"Text"`,
    /// `"UnsignedInteger"`) is what
    /// `dialog_query::AttributeDescriptor` round-trips through
    /// serde, not the underlying `ValueDataType` variant name.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("db.attribute")]
    pub struct Type(pub String);

    /// The cardinality of an attribute entity —
    /// `db.attribute/cardinality`. Takes `"one"` or
    /// `"many"`; the textual form matches what
    /// `dialog_query::Cardinality` serialises to.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("db.attribute")]
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
