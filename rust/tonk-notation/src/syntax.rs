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

/// One top-level entry. Three flavours, distinguished by the head:
///
/// | Head     | Body shape         | Variant       |
/// |----------|--------------------|---------------|
/// | `name`   | fields or empty    | `Query`       |
/// | `name!`  | fields or empty    | `Assertion`   |
/// | `rule!`  | `{assert!:|retract!:, when:, unless:?, description:?}` | `Rule` |
///
/// Retraction of a *single fact* (per attribute) is not a separate
/// top-level variant — it happens inside an assertion body via
/// `field: _` or `..: _`. The `Rule` variant captures *inductive
/// rules* whose head is an `assert!:` or `retract!:` directive
/// against a concept, and whose body is a `when:` / `unless:`
/// premise list.
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
    /// `rule!:` — an inductive rule. The body carries
    /// `assert!:` or `retract!:` (the head concept), `when:` (a
    /// list of positive premises), and optionally `unless:` (a
    /// list of negative premises) and `description:`.
    Rule(Rule),
}

impl Expression {
    /// Source range of the whole expression.
    pub fn range(&self) -> Range {
        match self {
            Expression::Query(q) => q.range,
            Expression::Assertion(a) => a.range,
            Expression::Rule(r) => r.range,
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

/// `rule!:` with a structured body. The body's shape is fixed
/// by the rule grammar rather than free-form `Field`s:
///
/// ```yaml
/// rule!:
///   assert!: counter        # or retract!: counter
///   description: "..."      # optional
///   when:
///     - assert: counter
///       where: { this: ?c, count: ?prev }
///     - assert: increment
///       where: { subject: ?c }
///   unless:                 # optional
///     - assert: counter-paused
///       where: { this: ?c }
/// ```
///
/// `assert!:` / `retract!:` are mutually exclusive; exactly one
/// must be present and its value is the head concept name. The
/// `when` list must be non-empty; each premise binds variables
/// that the head reads.
#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    /// The `rule!:` head itself (always concept `rule` with
    /// `effect = true`).
    pub head: Head,
    /// `Assert` for `assert!:`, `Retract` for `retract!:`.
    pub polarity: RulePolarity,
    /// Head concept name — the value of the `assert!:` /
    /// `retract!:` field.
    pub conclusion: Spanned<String>,
    /// Positive premises (under `when:`).
    pub when: Vec<Premise>,
    /// Negative premises (under `unless:`), if any.
    pub unless: Vec<Premise>,
    /// Optional human-readable description.
    pub description: Option<Spanned<String>>,
    /// Span of the whole `rule!: …` block.
    pub range: Range,
}

/// Polarity of a [`Rule`]'s head — whether matches assert or
/// retract head facts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RulePolarity {
    /// `assert!:` head — body matches produce new head facts.
    Assert,
    /// `retract!:` head — body matches produce retractions of
    /// the head concept's facts at the bound entity.
    Retract,
}

/// One premise inside a rule's `when` or `unless` list. A
/// premise is a mapping with `assert: <concept>` plus `where:
/// { … }` field bindings:
///
/// ```yaml
/// - assert: counter
///   where:
///     this: ?c
///     count: ?prev
/// ```
///
/// Variable names in the bindings are how rules connect
/// premises and feed the head — sharing a `?name` joins two
/// premises, and `?name` reappearing in the head's
/// (implicit) operand position binds the head's field.
#[derive(Clone, Debug, PartialEq)]
pub struct Premise {
    /// Concept name (value of the `assert:` field). For
    /// negative premises (under `unless:`) the `assert:` key is
    /// reused — there's no separate `retract:` key inside a
    /// premise body, because the premise's polarity is
    /// determined by which list (`when` vs `unless`) it
    /// appears in.
    pub concept: Spanned<String>,
    /// `where:` field bindings. May be empty (matches every
    /// entity of the concept).
    pub bindings: Vec<Field>,
    /// Span of the whole premise mapping.
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
