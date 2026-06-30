//! [`Type`] — a param's value type: the validator the engine matches *through*.
//!
//! A param has two orthogonal axes (see the crate docs): its **extent**
//! ([`Kind`](crate::Kind) — how far it reads) and its **type** ([`Type`] — what
//! the captured text must be). Extent is intrinsic to the path grammar and lives
//! here in the engine; type is owned by the route *model field* the param binds
//! to (`as: entity` / `as: unsigned` / …) and is supplied by the binding layer.
//!
//! The engine validates *through* the type during matching, so two structurally
//! identical routes can be told apart by param type: `/space/{s}/{page}` (a
//! `unsigned`) and `/space/{s}/{model}` (an `entity`) both match one segment, but
//! only one accepts `42`. To keep `tonk-router` dependency-free, [`Type`] is a
//! trait object — the engine ships [`text()`] (accepts anything) and the binding
//! layer plugs in `entity`/`unsigned`/`float` validators derived from the model's
//! field descriptors.

use std::fmt;
use std::sync::Arc;

/// A param value type: decides whether a captured string is admissible.
///
/// Implementors are plugged in by the binding layer (one per `as:` type). The
/// engine only ever calls [`ValueType::validate`] — it never interprets the
/// value, so the engine stays ignorant of dialog's type system.
pub trait ValueType: fmt::Debug + Send + Sync {
    /// A stable name for this type, used for equality and diagnostics
    /// (`"text"`, `"entity"`, `"unsigned"`, …). Two [`Type`]s are equal iff their
    /// names match, so a validator's name must be unique per type.
    fn name(&self) -> &str;

    /// Whether `value` is an admissible value of this type. Called during
    /// matching: returning `false` makes the param (and thus the route) reject
    /// this input, so an alternative route can be tried.
    fn validate(&self, value: &str) -> bool;
}

/// A param's value type — a cheaply-cloneable handle to a [`ValueType`].
///
/// Equality is by [`ValueType::name`] (so a [`Route`](crate::Route) stays
/// `PartialEq` for tests and dedup despite holding trait objects).
#[derive(Clone)]
pub struct Type(Arc<dyn ValueType>);

impl Type {
    /// Wrap a [`ValueType`] implementation.
    pub fn new(value_type: impl ValueType + 'static) -> Self {
        Self(Arc::new(value_type))
    }

    /// This type's name.
    pub fn name(&self) -> &str {
        self.0.name()
    }

    /// Whether `value` is admissible.
    pub fn validate(&self, value: &str) -> bool {
        self.0.validate(value)
    }
}

impl fmt::Debug for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Type({})", self.0.name())
    }
}

impl PartialEq for Type {
    fn eq(&self, other: &Self) -> bool {
        self.0.name() == other.0.name()
    }
}

impl Eq for Type {}

/// The default param type: accepts any non-empty string. Used when a pattern
/// names no type (`{model}` rather than a typed binding) and as the base the
/// binding layer overrides with `entity`/`unsigned`/etc.
#[derive(Clone, Copy, Debug)]
pub struct Text;

impl ValueType for Text {
    fn name(&self) -> &str {
        "text"
    }

    fn validate(&self, _value: &str) -> bool {
        true
    }
}

/// The default [`Type`] — [`Text`], accepting anything.
pub fn text() -> Type {
    Type::new(Text)
}

impl Default for Type {
    fn default() -> Self {
        text()
    }
}
