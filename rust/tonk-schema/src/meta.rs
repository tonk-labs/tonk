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

use dialog_query::Attribute;

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
