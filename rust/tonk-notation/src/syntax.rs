//! Typed syntax tree for asserted notation.
//!
//! [`parse::parse`][crate::parse::parse] produces a [`Syntax`] tree
//! from a YAML document. The tree captures the surface shape — what
//! the user typed — without resolving names against a branch or
//! deriving entity URIs. Resolution happens in
//! [`tonk-schema`'s analyzer][analyze].
//!
//! Every node carries an [`lsp_types::Range`] so consumers can
//! attach diagnostics to the source token they came from.
//!
//! [analyze]: https://github.com/dialog-db/tonk-workers/tree/main/rust/tonk-schema/src/interpret.rs

use lsp_types::Range;

/// A whole asserted-notation document: a list of expressions.
#[derive(Clone, Debug, PartialEq)]
pub struct Syntax {
    /// Expressions in document order.
    pub expressions: Vec<Expression>,
    /// Span covering the whole document. Exposed so document-level
    /// diagnostics ("root must be a mapping") have something to
    /// point at.
    pub range: Range,
}

/// One top-level entry. Two flavours, distinguished by the head's
/// effect marker (`!`):
///
/// | Head     | Body              | Variant       |
/// |----------|-------------------|---------------|
/// | `name`   | fields or empty   | `Query`       |
/// | `name!`  | fields or empty   | `Assertion`   |
///
/// Retraction is not a separate top-level variant — it happens
/// *inside* an assertion body via `field: _` (retract one
/// attribute) or `..: _` (retract every attribute in the concept's
/// `with:` map not named elsewhere in the body).
///
/// A bare `_` body (`head!: _`) is a parse error: with no `this:`
/// field there's no entity selection mechanism for the operation
/// to act on.
#[derive(Clone, Debug, PartialEq)]
pub enum Expression {
    /// `head:` (no `!`) — read facts matching the body's pattern.
    Query(Query),
    /// `head!:` — assert each field as a fact about the entity
    /// selected by the body's `this:` field (or the body-derived
    /// entity if `this:` is omitted). Per-field retractions live
    /// inside the body as `field: _` or `..: _`.
    Assertion(Assertion),
}

impl Expression {
    /// Source range of the whole expression.
    pub fn range(&self) -> Range {
        match self {
            Expression::Query(q) => q.range,
            Expression::Assertion(a) => a.range,
        }
    }
}

/// `head:` — a query expression.
#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    /// Concept or claim name with no effect marker.
    pub head: Head,
    /// Field constraints under the head. Empty body (`head:`) is
    /// allowed and means "any entity matching the head's concept".
    pub fields: Vec<Field>,
    /// Span of the whole `head: …` block.
    pub range: Range,
}

/// `head!:` with a fields body — an assertion expression.
#[derive(Clone, Debug, PartialEq)]
pub struct Assertion {
    /// Concept or claim name with the effect marker (`!`).
    pub head: Head,
    /// Optional `&anchor` written between the head's `:` and the
    /// body. Desugars to a `name!: this: id:<anchor>, entity:
    /// ?<anchor>` expression so future references to the anchor
    /// resolve through the name table.
    pub anchor: Option<Anchor>,
    /// Fields to assert. The reserved field `this:` selects which
    /// entity the assertion operates on; `..: _` retracts every
    /// other attribute in the concept's `with:` map; `field: _`
    /// retracts that one attribute. Other fields are asserted.
    pub fields: Vec<Field>,
    /// Span of the whole `head!: …` block.
    pub range: Range,
}

/// A YAML anchor written on the head's value. Captured from the
/// source text between the head's `:` and the start of the body's
/// span (saphyr exposes anchors only as numeric IDs in events, so
/// the parser scans the source slice to recover the literal name).
#[derive(Clone, Debug, PartialEq)]
pub struct Anchor {
    /// Anchor name (without the leading `&`).
    pub name: String,
    /// Span of the `&name` token in the source.
    pub range: Range,
}

/// Classified head: name + effect marker.
///
/// Heads under the new grammar carry no inline binding — every
/// reference to *which* entity the expression operates on lives in
/// the body (`this:` meta-key) or, for assertions, in a `&anchor`.
#[derive(Clone, Debug, PartialEq)]
pub struct Head {
    /// What kind of entity this head names — concept (bare
    /// identifier) or claim (reverse-dotted domain) or a direct
    /// entity URI (`db:concept`, `id:person`, `did:key:zX`).
    pub name: HeadName,
    /// Span of the head text (without the trailing `:`).
    pub range: Range,
    /// Original text of the name, without trailing `!`. Useful for
    /// diagnostic round-tripping and for builtin-concept dispatch
    /// in the analyzer.
    pub source: String,
    /// `true` if the head ended in `!`, marking the expression as
    /// having an effect (assertion or retraction).
    pub effect: bool,
}

/// Concept vs claim, distinguished lexically. Concept names are
/// bare lowercase identifiers; claim domains are reverse-dotted;
/// URIs carry an explicit scheme.
#[derive(Clone, Debug, PartialEq)]
pub enum HeadName {
    /// A concept name (bare identifier). The analyzer resolves it
    /// through the branch's name table — built-in mappings cover
    /// `attribute`/`concept`/`name` out of the box.
    Concept(String),
    /// A reverse-dotted domain (`xyz.tonk`, `io.gozala.person`).
    /// Each field name combines with the domain to form an
    /// attribute URI (`xyz.tonk/role`).
    Claim(String),
    /// A scheme-prefixed URI used as the head with no resolution
    /// (`db:concept`, `id:person`, `did:key:…`). Lets users reach
    /// a built-in even when its bare name has been shadowed.
    Uri(String),
}

/// One field under a head's body.
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    /// Field name (the level-3 key). Reserved names: `this`, `..`.
    pub name: String,
    /// Span of the field name.
    pub name_range: Range,
    /// Field value — literal, variable, blank, symbol reference,
    /// URI, or a nested map.
    pub value: FieldValue,
    /// Span of the value side of the entry.
    pub value_range: Range,
}

/// What sits on the right of a `field:` entry.
#[derive(Clone, Debug, PartialEq)]
pub enum FieldValue {
    /// A primitive literal — quoted string, number, bool, null.
    /// Note: bare lowercase identifiers are parsed as
    /// [`FieldValue::Symbol`], not as string literals; quotes are
    /// load-bearing for string-shaped values that match the symbol
    /// charset.
    Literal(Scalar),
    /// `?var` — a logic variable. In a query, binds whatever
    /// matches; in an assertion, must be bound by some earlier
    /// query expression.
    Variable(String),
    /// `_` — blank. In a query, matches any value (not surfaced as
    /// a join key). In an assertion, retracts that field for the
    /// head's entity.
    Blank,
    /// A bare lowercase identifier (`person-name`). Resolves
    /// through the name table to the entity the symbol currently
    /// names.
    Symbol(String),
    /// A scheme-prefixed URI (`id:foo`, `db:foo`, `did:key:…`,
    /// `xyz.tonk/foo`). Direct entity reference, no resolution.
    Uri(String),
    /// A nested mapping. The analyzer interprets it based on
    /// context — `concept!.with`, an inline `attribute!`
    /// definition, or an explicit content-derivation salt as
    /// `this: { … }`.
    Nested(Vec<Field>),
}

/// A primitive value. Mirrors the shapes saphyr produces for
/// scalar leaves.
#[derive(Clone, Debug, PartialEq)]
pub enum Scalar {
    /// A textual scalar.
    String(String),
    /// An integer literal.
    Integer(i128),
    /// An unsigned-integer literal too large for `i128`'s
    /// negative half — preserved so we don't lose precision before
    /// the analyzer coerces.
    UnsignedInteger(u128),
    /// A floating-point literal.
    Float(f64),
    /// A boolean literal.
    Boolean(bool),
    /// A `null` literal.
    Null,
}

/// A value plus the source range it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct Spanned<T> {
    /// The value.
    pub value: T,
    /// Source range.
    pub range: Range,
}

impl<T> Spanned<T> {
    /// Construct a `Spanned` from a value and a range.
    pub fn new(value: T, range: Range) -> Self {
        Self { value, range }
    }
}
