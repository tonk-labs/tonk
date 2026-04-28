//! Typed syntax tree for asserted notation.
//!
//! `parse::parse` produces a [`Syntax`] tree from a YAML or JSON
//! document. The tree is purely structural — it captures the
//! three-level subject/context/fields shape and classifies each
//! level-1 / level-2 key, but it does *not* resolve bookmark
//! references, derive entity URIs, or know anything about the
//! dialog meta-schema. Those concerns live in
//! [`tonk-schema`'s interpreter][interpret].
//!
//! Every node carries an [`lsp_types::Range`] so downstream
//! consumers (the language server, the interpreter) can attach
//! diagnostics to the source token they came from.
//!
//! [interpret]: https://github.com/dialog-db/tonk-workers/tree/main/rust/tonk-schema/src/interpret.rs

use lsp_types::Range;

/// A whole asserted-notation document, as a list of statements.
///
/// Each statement is one top-level entry: a *subject* paired
/// with one or more *contexts* describing facts about that
/// subject. Statements appear in source order.
#[derive(Clone, Debug, PartialEq)]
pub struct Syntax {
    /// The statements in document order.
    pub statements: Vec<Statement>,
    /// The span covering the whole document. Exposed so
    /// document-level diagnostics ("root must be a mapping")
    /// have something to point at.
    pub range: Range,
}

/// One subject-headed entry in a document.
///
/// ```yaml
/// person-name:                  # subject
///   attribute:                  # context (with fields)
///     the: io.gozala.person/name
///     as:  Text
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Statement {
    /// What the level-1 key names — bookmark, URI, anonymous, or
    /// variable. The original textual form is in
    /// [`Subject::source`] for diagnostic round-tripping.
    pub subject: Subject,
    /// One or more contexts on this subject. The same subject can
    /// be described under multiple contexts in one document
    /// (e.g. attribute facts under one domain plus relations
    /// under another), in which case all the resulting facts land
    /// on the same entity.
    pub contexts: Vec<Context>,
    /// The span of the entire `<subject>: { <contexts> }` block.
    pub range: Range,
}

/// Classified level-1 key.
#[derive(Clone, Debug, PartialEq)]
pub struct Subject {
    /// The classification.
    pub kind: SubjectKind,
    /// The original key text (without quoting). Useful when the
    /// interpreter needs to feed the bookmark name into a
    /// `dialog.meta/name` claim, or when an error wants to quote
    /// the literal source token.
    pub source: String,
    /// The span of the level-1 key.
    pub range: Range,
}

/// What flavour of identifier the subject is.
#[derive(Clone, Debug, PartialEq)]
pub enum SubjectKind {
    /// A bare identifier — interpreted by the interpreter as a
    /// bookmark name. Resolves document-then-branch.
    Bookmark,
    /// A URI literal (the source contains `:`). Interpreter uses
    /// it as the entity directly.
    Uri,
    /// `_` — an anonymous fresh entity. Acceptance / rejection
    /// is the interpreter's call; the parser only tags it.
    Anonymous,
    /// `?<name>` — a logic variable. Reserved for future query /
    /// rule contexts; today the interpreter rejects it.
    Variable,
}

/// One level-2 entry on a subject: a context plus the level-3
/// fields under it.
#[derive(Clone, Debug, PartialEq)]
pub enum Context {
    /// A bare-domain context — level-2 key contains `.`. Each
    /// level-3 field expands to a raw `<domain>/<field>` claim
    /// against the subject's entity.
    Domain(DomainContext),
    /// A built-in `attribute` context — declares an attribute
    /// (the / as / cardinality / description).
    Attribute(AttributeNode),
    /// A built-in `concept` context — composes attributes into a
    /// shape via `with` / `maybe`.
    Concept(ConceptNode),
    /// A user-defined concept name (level-2 key with no `.` and
    /// not a built-in). The interpreter resolves these via
    /// branch lookup.
    UserConcept(UserConceptNode),
}

/// Raw EAV writes under a `<domain>:` context.
#[derive(Clone, Debug, PartialEq)]
pub struct DomainContext {
    /// The domain string (level-2 key).
    pub domain: String,
    /// Span of the level-2 key.
    pub key_range: Range,
    /// The fields inside the context. Order is preserved from
    /// the source so deterministic emit order is the same as
    /// authored order.
    pub fields: Vec<DomainField>,
    /// Span of the whole `domain: { … }` block.
    pub range: Range,
}

/// One field under a domain context.
#[derive(Clone, Debug, PartialEq)]
pub struct DomainField {
    /// The field name (level-3 key).
    pub name: String,
    /// Span of the field name.
    pub name_range: Range,
    /// The value at this field. Sequences are folded into a
    /// `Many` value so cardinality-many writes are representable
    /// without a separate node kind.
    pub value: DomainValue,
    /// Span covering the value (keeps zero-width sequences
    /// pointing at the right place for diagnostics).
    pub value_range: Range,
}

/// Domain-context field value. Maps and sequences are
/// represented structurally so the interpreter can decide what
/// to do with each shape (nested entity, multi-value claim,
/// rejection).
#[derive(Clone, Debug, PartialEq)]
pub enum DomainValue {
    /// A YAML scalar — primitive value as written.
    Scalar(Scalar),
    /// A YAML sequence — many values for the same field.
    Sequence(Vec<DomainValue>),
    /// A YAML mapping — nested entity, today unsupported by the
    /// interpreter but represented so the diagnostic can be
    /// raised at interpret time.
    Mapping(Vec<DomainField>),
}

/// A primitive value carried by a domain field. Mirrors the
/// shapes saphyr produces for scalar leaves.
#[derive(Clone, Debug, PartialEq)]
pub enum Scalar {
    /// A textual scalar.
    String(String),
    /// An integer literal (saphyr-classified or raw representation).
    Integer(i128),
    /// An unsigned-integer literal too large for `i128`'s
    /// negative half — preserved as a string so we don't lose
    /// precision before the interpreter coerces.
    UnsignedInteger(u128),
    /// A floating-point literal.
    Float(f64),
    /// A boolean literal.
    Boolean(bool),
    /// A `null` literal.
    Null,
}

/// `attribute:` context body.
///
/// Maps to `dialog_query::AttributeDescriptor` once the
/// interpreter resolves and validates each field. We mirror the
/// descriptor's optional fields verbatim so the JSON / YAML
/// shape round-trips through serde-deriving on the interpreter
/// side.
#[derive(Clone, Debug, PartialEq)]
pub struct AttributeNode {
    /// The required `the` field — `domain/name` form.
    pub the: Option<Spanned<String>>,
    /// Optional value-type descriptor (e.g. `Text`,
    /// `UnsignedInteger`). Stored as the source string and
    /// validated by the interpreter.
    pub as_type: Option<Spanned<String>>,
    /// Optional cardinality (`one` / `many`).
    pub cardinality: Option<Spanned<String>>,
    /// Optional description.
    pub description: Option<Spanned<String>>,
    /// Span of the level-2 key.
    pub key_range: Range,
    /// Span of the entire `attribute: { … }` block.
    pub range: Range,
}

/// `concept:` context body.
#[derive(Clone, Debug, PartialEq)]
pub struct ConceptNode {
    /// Optional description.
    pub description: Option<Spanned<String>>,
    /// Required-field references.
    pub with: Vec<ConceptField>,
    /// Optional-field references.
    pub maybe: Vec<ConceptField>,
    /// Span of the level-2 key.
    pub key_range: Range,
    /// Span of the entire `concept: { … }` block.
    pub range: Range,
}

/// One field of a concept's `with` or `maybe` block.
#[derive(Clone, Debug, PartialEq)]
pub struct ConceptField {
    /// The user-chosen field name (the key in `with`).
    pub name: String,
    /// Span of the field name.
    pub name_range: Range,
    /// What the field references — bookmark, URI, or inline
    /// attribute definition.
    pub value: Reference,
    /// Span of the value side of the entry.
    pub value_range: Range,
}

/// Reference to an attribute from inside a concept's `with` /
/// `maybe` block.
#[derive(Clone, Debug, PartialEq)]
pub enum Reference {
    /// A bookmark name. Resolves document-then-branch.
    Bookmark(Spanned<String>),
    /// A URI literal (`the:…` / `did:key:…`).
    Uri(Spanned<String>),
    /// An inline attribute descriptor — same shape as a
    /// top-level `attribute:` body. Asserted recursively.
    Inline(Box<AttributeNode>),
}

/// User-defined concept context. The interpreter resolves the
/// concept by name on the branch and validates the fields
/// against its descriptor's `with` / `maybe`.
#[derive(Clone, Debug, PartialEq)]
pub struct UserConceptNode {
    /// The concept name (level-2 key).
    pub name: String,
    /// Span of the level-2 key.
    pub key_range: Range,
    /// The fields written under this concept. Each one is a
    /// reference to an attribute value — same flavour as a
    /// `with` field — though the *interpretation* differs
    /// (these populate concept *instances*, not concept
    /// definitions).
    pub fields: Vec<DomainField>,
    /// Span of the entire `<concept>: { … }` block.
    pub range: Range,
}

/// A value plus the source range it came from.
///
/// Used for the optional fields of an [`AttributeNode`] /
/// [`ConceptNode`] where we want both the parsed value and the
/// span (for type-error diagnostics).
#[derive(Clone, Debug, PartialEq)]
pub struct Spanned<T> {
    /// The value.
    pub value: T,
    /// The source range of the value (or, where the value side
    /// was missing, the key it was attached to).
    pub range: Range,
}

impl<T> Spanned<T> {
    /// Construct a `Spanned` from a value and a range.
    pub fn new(value: T, range: Range) -> Self {
        Self { value, range }
    }
}
