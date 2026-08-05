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
//! # Two expression shapes
//!
//! Every top-level entry is either a [`Query`] or a [`Claim`].
//! `Query` is `head:` (no `!`) and reads facts; `Claim` is `head!:`
//! and writes them. The `rule!:` form is *not* a separate variant —
//! it's a [`Claim`] whose head's predicate is the built-in `rule`
//! concept and whose body fields name the rule's parts
//! (`assert!:` / `retract!:` / `when:` / `unless:` / `description:`).
//! The analyzer recognises the `rule` predicate and lifts the body
//! into a [`tonk_schema::rule::Rule`] mutation.
//!
//! Retraction sits inside a claim body. `field: _` retracts that one
//! attribute; `..: _` retracts every attribute the concept declares
//! (used for whole-entity deletes — including rule deletes via
//! `rule!: this: <effect entity> ..: _`).
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
/// trailing `!`:
///
/// | Head     | Variant       |
/// |----------|---------------|
/// | `name`   | `Query`       |
/// | `name!`  | `Claim`       |
///
/// `rule!:` is **not** a third variant — it's a [`Claim`] over the
/// built-in `rule` concept (see the module docs). The analyzer
/// recognises the predicate and dispatches to its rule-install path.
///
/// Per-field retraction lives inside the claim body (`field: _` or
/// `..: _`). A bare `_` body (`head!: _`) is a parse error: with no
/// `this:` field there's no entity selection.
#[derive(Clone, Debug, PartialEq)]
pub enum Expression {
    /// `head:` (no `!`) — read facts matching the body's pattern.
    Query(Application),
    /// `head!:` — write facts of the head's concept. Wraps the
    /// application in an [`Effectful`] envelope that carries the
    /// optional `&anchor` and is the structural marker that the head
    /// had `!`.
    Claim(Effectful<Application>),
}

impl Expression {
    /// Source range of the whole expression.
    pub fn range(&self) -> Range {
        match self {
            Expression::Query(q) => q.range,
            Expression::Claim(c) => c.inner.range,
        }
    }

    /// The application underlying this expression — the predicate
    /// plus its field bindings. Same shape whether the expression
    /// is a query or a claim; only the wrapper differs.
    pub fn application(&self) -> &Application {
        match self {
            Expression::Query(q) => q,
            Expression::Claim(c) => &c.inner,
        }
    }
}

/// A predicate applied to a body of field bindings.
///
/// The shared shape between queries and claims. Whether this counts
/// as a read or a write is decided by the wrapping [`Expression`]
/// variant ([`Expression::Query`] vs [`Expression::Claim`]): inside
/// a [`Claim`](Expression::Claim) the same `Application` is the
/// thing being asserted; inside [`Expression::Query`] it is the
/// thing being matched against.
#[derive(Clone, Debug, PartialEq)]
pub struct Application {
    /// Concept / claim / URI on the head — *without* the `!` marker
    /// (which lives on the outer [`Expression`]).
    pub predicate: Predicate,
    /// Body field constraints (queries) or field assignments
    /// (claims). Empty (`head:` with no body) is allowed for queries
    /// and means "any entity matching the head's concept". Claims
    /// require at least one field (`this:` minimum) so they have an
    /// entity to operate on; the parser enforces this.
    pub fields: Vec<Field>,
    /// Span of the whole `head … : body` block.
    pub range: Range,
}

/// The `!` marker, wrapping whatever the marker decorates.
///
/// In tonk-notation grammar a head's trailing `!` is the "this is a
/// mutation" tag. We give it a dedicated wrapper rather than a
/// `effect: bool` field on the predicate because the structural
/// presence/absence of [`Effectful`] is more honest about what `!`
/// means: a claim *is* an effect; a query is not.
///
/// The wrapper also holds the optional `&anchor` written between the
/// head's `:` and the body — the anchor is the other piece that
/// only makes sense for effectful expressions (it names the entity
/// the assertion writes to so later expressions can refer back).
#[derive(Clone, Debug, PartialEq)]
pub struct Effectful<T> {
    /// Optional `&anchor` written between the head's `:` and the
    /// body. Desugars to `this: id:<anchor>` plus a name-table
    /// assertion so subsequent references resolve through it.
    pub anchor: Option<Anchor>,
    /// What the `!` decorates — typically an [`Application`].
    pub inner: T,
}

/// One premise inside a rule's `when:` or `unless:` list. A
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
///
/// Premises live in the [`syntax`](self) module rather than as
/// nested fields under the `when:` key because their shape is
/// structurally distinct (one concept + a where-map) and we want
/// the diagnostic surface to point at specific premise sub-ranges.
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

/// Classified head — name plus lexical class.
///
/// The `!` marker is *not* on `Predicate`; it lives on the outer
/// [`Expression`] variant (presence of [`Effectful`] = head ended in
/// `!`). Heads carry no inline binding — every reference to *which*
/// entity the expression operates on lives in the body (`this:`
/// meta-key) or, for claims, in a `&anchor` on the [`Effectful`]
/// wrapper.
#[derive(Clone, Debug, PartialEq)]
pub struct Predicate {
    /// What kind of entity this head names — concept (bare
    /// identifier) or claim (reverse-dotted domain) or a direct
    /// entity URI (`db:concept`, `id:person`, `did:key:zX`).
    pub name: HeadName,
    /// Span of the head text (without the trailing `:` or `!`).
    pub range: Range,
    /// Original text of the name, without trailing `!`. Useful for
    /// diagnostic round-tripping and for builtin-concept dispatch
    /// in the analyzer.
    pub source: String,
}

/// Concept vs claim, distinguished lexically. Concept names are
/// bare lowercase identifiers; claim domains are reverse-dotted;
/// URIs carry an explicit scheme.
#[derive(Clone, Debug, PartialEq)]
pub enum HeadName {
    /// A concept name (bare identifier). The analyzer resolves it
    /// through the branch's name table — built-in mappings cover
    /// `attribute`/`concept`/`name`/`rule` out of the box.
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
    /// URI, a nested map, or a premise list (for rule bodies'
    /// `when:` / `unless:` keys).
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
    /// A list of premises — the parsed shape of a `when:` or
    /// `unless:` value inside a `rule!:` claim body. Each premise
    /// is the structured `{assert: <concept>, where: {…}}` mapping.
    /// Carried as a typed list (rather than as `Nested(Vec<Field>)`
    /// with field-named nesting) so the analyzer can rely on
    /// per-premise ranges for diagnostics.
    Premises(Vec<Premise>),
    /// A sequence of scalar names. Used by finite enumerations such
    /// as `projection!.actions`; analyzers for other heads reject it.
    List(Vec<Spanned<String>>),
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
