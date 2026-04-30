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
///
/// Each expression is one top-level entry: a [`Head`] (concept or
/// claim name, optional `!` effect marker, optional binding) paired
/// with a [`Body`] (fields, or a single `_` discard).
#[derive(Clone, Debug, PartialEq)]
pub struct Syntax {
    /// Expressions in document order.
    pub expressions: Vec<Expression>,
    /// Span covering the whole document. Exposed so document-level
    /// diagnostics ("root must be a mapping") have something to
    /// point at.
    pub range: Range,
}

/// One top-level entry. Three flavours, distinguished by the head's
/// effect marker (`!`) and body shape:
///
/// | Head     | Body         | Variant       |
/// |----------|--------------|---------------|
/// | `name`   | fields       | `Query`       |
/// | `name!`  | fields       | `Assertion`   |
/// | `name!`  | `_`          | `Retraction`  |
///
/// `name` (no `!`) with `_` body is rejected by the parser — a
/// query body always introduces or constrains; an empty query has
/// no useful meaning.
#[derive(Clone, Debug, PartialEq)]
pub enum Expression {
    /// `head:` (no `!`) — read facts matching the body's pattern.
    Query(Query),
    /// `head!:` — assert each field as a fact about the head's
    /// entity.
    Assertion(Assertion),
    /// `head! …: _` — retract every fact for this head's
    /// entity-and-concept (or, for claims, every fact under the
    /// claim's domain on this entity).
    Retraction(Retraction),
}

impl Expression {
    /// Source range of the whole expression.
    pub fn range(&self) -> Range {
        match self {
            Expression::Query(q) => q.range,
            Expression::Assertion(a) => a.range,
            Expression::Retraction(r) => r.range,
        }
    }
}

/// `head:` — a query expression.
#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    /// What the entity is named by — concept or claim, plus
    /// binding.
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
    /// What the entity is named by — concept or claim, plus
    /// binding.
    pub head: Head,
    /// Fields to assert. Each field's value is substituted in by
    /// the analyzer (literals stay literal, `?var` resolves to its
    /// binding from a query expression).
    pub fields: Vec<Field>,
    /// Span of the whole `head!: …` block.
    pub range: Range,
}

/// `head! …: _` — retract this head's facts.
///
/// Body shape rules:
///
/// - **Concept retraction** (`person! ?nick: _`) drops the entire
///   concept-projection for the entity. Equivalent to retracting
///   each `with`/`maybe` field of the concept.
/// - **Claim retraction** (`xyz.tonk! did:key:zJack: _`) lists
///   every fact on the entity whose attribute URI starts with the
///   claim's domain, then retracts each.
/// - **Field-level retraction** (`person! ?nick: { name: _ }`) is
///   represented as an [`Assertion`] with a [`FieldValue::Blank`]
///   value — the analyzer recognises blanks in `!` mode as
///   per-field retractions.
#[derive(Clone, Debug, PartialEq)]
pub struct Retraction {
    /// What the entity is named by.
    pub head: Head,
    /// Span of the whole `head!: _` block.
    pub range: Range,
}

/// Classified head: name + effect + binding.
///
/// Parsed by splitting the YAML key on whitespace: the first token
/// (with optional trailing `!`) is the [`HeadName`], any remaining
/// tokens form the [`Binding`].
#[derive(Clone, Debug, PartialEq)]
pub struct Head {
    /// What kind of entity this head names — concept (bare
    /// identifier) or claim (reverse-dotted domain).
    pub name: HeadName,
    /// Span of the head name (without binding).
    pub name_range: Range,
    /// Original text of the name, without trailing `!`. Useful for
    /// diagnostic round-tripping and for builtin-concept dispatch
    /// in the analyzer (matching `"attribute"` / `"concept"`).
    pub name_source: String,
    /// `true` if the head ended in `!`, marking the expression as
    /// having an effect (assertion or retraction).
    pub effect: bool,
    /// What identifies the entity this head refers to.
    pub binding: Binding,
    /// Span of the binding token. For [`Binding::Anonymous`] this
    /// collapses to the end of the name range (zero-width widened
    /// per the parser's span policy).
    pub binding_range: Range,
}

/// Concept vs claim, distinguished lexically by the presence of
/// `.` in the name (claim) versus a bare identifier (concept).
#[derive(Clone, Debug, PartialEq)]
pub enum HeadName {
    /// A concept name (bare identifier). The analyzer resolves it
    /// through its `Resolver` — built-in concepts like `attribute`
    /// and `concept` have hard-coded descriptors; user-defined
    /// names hit the branch.
    Concept(String),
    /// A reverse-dotted domain (`xyz.tonk`, `io.gozala.person`).
    /// Each field name combines with the domain to form an
    /// attribute URI (`xyz.tonk/role`).
    Claim(String),
}

/// What the entity is identified by.
#[derive(Clone, Debug, PartialEq)]
pub enum Binding {
    /// `head:` — no binding. For queries this matches any entity;
    /// for assertions a fresh entity is derived (concept-content
    /// for built-ins; nameless for claims).
    Anonymous,
    /// `head ?var:` — binds the entity as a variable named `var`.
    /// Shared variables across expressions in the same document
    /// join.
    Variable(String),
    /// `head bookmark:` — assertion only typically. Derives an
    /// entity from the bookmark name and asserts a name-binding
    /// claim. Already-defined bookmarks resolve to their existing
    /// entity.
    Bookmark(String),
    /// `head did:key:zX:` — explicit entity URI.
    Uri(String),
}

/// One field under a head's body.
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    /// Field name (the level-3 key).
    pub name: String,
    /// Span of the field name.
    pub name_range: Range,
    /// Field value — literal, variable, blank, reference, or a
    /// nested map (e.g. `concept!`'s `with:` map, or an inline
    /// `attribute!` definition).
    pub value: FieldValue,
    /// Span of the value side of the entry.
    pub value_range: Range,
}

/// What sits on the right of a `field:` entry.
#[derive(Clone, Debug, PartialEq)]
pub enum FieldValue {
    /// A primitive literal — string, number, bool, null.
    Literal(Scalar),
    /// `?var` — a logic variable. In a query, binds whatever
    /// matches; in an assertion, must be bound by some earlier
    /// query expression.
    Variable(String),
    /// `_` — anonymous. In a query, matches any value (not
    /// surfaced as a join key). In an assertion, retracts that
    /// field for the head's entity.
    Blank,
    /// A reference to another entity by name — bookmark or URI.
    /// This is what concept-field references look like
    /// (`with: { name: person-name }` resolves `person-name` as a
    /// bookmark).
    Reference(Reference),
    /// A nested mapping. The analyzer interprets it based on
    /// context — `concept!.with`, an inline `attribute!`
    /// definition, etc.
    Nested(Vec<Field>),
}

/// Where a referenced entity comes from.
#[derive(Clone, Debug, PartialEq)]
pub enum Reference {
    /// A bookmark name.
    Bookmark(String),
    /// A URI literal.
    Uri(String),
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

/// A value plus the source range it came from. Used wherever a
/// downstream pass needs both the parsed value and the span (e.g.
/// to attach a type-error diagnostic to the right token).
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
