//! Errors raised by [`crate::analyzer::analyze`].
//!
//! [`AnalyzeError`] is a struct pairing a [`AnalyzeErrorKind`]
//! payload with an optional [`lsp_types::Range`] pointing at the
//! source text that triggered it. The range is `Option` because
//! some failures (resolver I/O, empty document) have no obvious
//! source location to attach to.
//!
//! Each kind also carries a stable `code()` like
//! `"E_UNKNOWN_CONCEPT"` so editor integrations can key off a
//! machine-readable identifier rather than scraping the user-facing
//! message.

use lsp_types::Range;
use thiserror::Error;

/// Render a [`dialog_query::Type`] in the notation's `as:`
/// vocabulary (e.g. `Text`, `SignedInteger`, `UnsignedInteger`)
/// rather than its Rust `Debug` spelling. Falls back to the Debug
/// form if serde can't produce a string label.
fn type_label(ty: dialog_query::Type) -> String {
    serde_json::to_value(ty)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{ty:?}"))
}

/// Analyzer error: a [`AnalyzeErrorKind`] payload plus an
/// optional source range. Construct from a kind via
/// `kind.into()` (no range) or [`AnalyzeError::new`] /
/// [`AnalyzeError::with_range`] when a range is available.
#[derive(Debug, Error)]
#[error("{kind}")]
pub struct AnalyzeError {
    /// What went wrong.
    pub kind: AnalyzeErrorKind,
    /// Source range of the offending construct, when known.
    /// `None` for failures with no clear location (resolver I/O,
    /// empty document).
    pub range: Option<Range>,
}

impl AnalyzeError {
    /// Construct from a kind with no range.
    pub fn new(kind: AnalyzeErrorKind) -> Self {
        Self { kind, range: None }
    }

    /// Construct from a kind with a range.
    pub fn at(kind: AnalyzeErrorKind, range: Range) -> Self {
        Self {
            kind,
            range: Some(range),
        }
    }

    /// Builder-style: attach a range to a no-range error.
    /// Idempotent on already-ranged errors (keeps the existing
    /// range).
    pub fn with_range(mut self, range: Range) -> Self {
        if self.range.is_none() {
            self.range = Some(range);
        }
        self
    }

    /// Builder-style: fill in the `field` name on a kind that was
    /// raised without one (the value-coercion helpers don't know
    /// the field they're translating). A no-op for any other kind.
    pub fn with_field(mut self, field: &str) -> Self {
        if let AnalyzeErrorKind::TypeMismatch { field: f, .. } = &mut self.kind {
            *f = field.to_owned();
        }
        self
    }

    /// Stable, machine-readable code for the error category.
    /// Editors and other consumers can match on this without
    /// parsing the human-readable message.
    pub fn code(&self) -> &'static str {
        self.kind.code()
    }
}

impl From<AnalyzeErrorKind> for AnalyzeError {
    fn from(kind: AnalyzeErrorKind) -> Self {
        Self::new(kind)
    }
}

/// Severity of an [`AnalyzeDiagnostic`]. Mirrors LSP's three
/// useful severity levels; `Hint` and `Information` aren't used
/// by the analyzer today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    /// User intent likely doesn't match what was written; the
    /// analyzer can still produce a plan.
    Warning,
    /// Pattern is malformed in a way the analyzer chose not to
    /// hard-fail on, but execution will misbehave or produce
    /// nothing useful. Surfaced alongside the `Result::Ok`
    /// analysis.
    Error,
}

/// Non-fatal finding from the analyzer. Carries the same shape
/// (kind + range + code) as [`AnalyzeError`] so the LSP can
/// surface them through the same diagnostic pipeline, plus a
/// severity so the editor can color them differently.
///
/// Distinct from [`AnalyzeError`] because diagnostics are
/// *accumulated* into the analysis tree's
/// [`DocumentAnalysis`][crate::analysis::DocumentAnalysis]
/// alongside a successful analysis, whereas [`AnalyzeError`]
/// short-circuits the whole pass.
#[derive(Debug, Clone)]
pub struct AnalyzeDiagnostic {
    /// What the diagnostic is about.
    pub kind: AnalyzeDiagnosticKind,
    /// Severity for editor styling.
    pub severity: DiagnosticSeverity,
    /// Source range of the offending construct, when known.
    pub range: Option<lsp_types::Range>,
}

impl AnalyzeDiagnostic {
    /// Construct a warning-level diagnostic at a known range.
    pub fn warning(kind: AnalyzeDiagnosticKind, range: lsp_types::Range) -> Self {
        Self {
            kind,
            severity: DiagnosticSeverity::Warning,
            range: Some(range),
        }
    }

    /// Construct an error-level diagnostic at a known range.
    pub fn error(kind: AnalyzeDiagnosticKind, range: lsp_types::Range) -> Self {
        Self {
            kind,
            severity: DiagnosticSeverity::Error,
            range: Some(range),
        }
    }

    /// Stable, machine-readable code for the diagnostic.
    pub fn code(&self) -> &'static str {
        self.kind.code()
    }

    /// Human-readable message.
    pub fn message(&self) -> String {
        self.kind.to_string()
    }
}

/// Categories of non-fatal analyzer findings.
///
/// Each variant has a stable `code()` like
/// `"W_SINGLE_OCCURRENCE_VARIABLE_QUERY_FIELD"` so the editor
/// can route quickfixes by code.
#[derive(Debug, Clone, Error)]
pub enum AnalyzeDiagnosticKind {
    /// A `?var` appears exactly once in a query body's non-`this:`
    /// field. Variables exist to create joins; a single use binds
    /// nothing useful. The user almost certainly meant `_`.
    #[error(
        "variable ?{name} is used only once in field {field:?} — \
         use `_` if you don't need to bind the value, or \
         reference ?{name} elsewhere to create a join"
    )]
    SingleOccurrenceVariableQueryField {
        /// The variable name (without `?`).
        name: String,
        /// The field where the variable appears.
        field: String,
    },
    /// A `?var` appears exactly once as the `this:` value of a
    /// query body. Same logic as above but for the entity slot —
    /// `_` means "any entity" and is the right form when the user
    /// just wants to enumerate.
    #[error(
        "variable ?{name} is used only once in `this:` — \
         use `_` if you don't need to bind the entity, or \
         reference ?{name} elsewhere to create a join"
    )]
    SingleOccurrenceVariableQueryThis {
        /// The variable name (without `?`).
        name: String,
    },
    /// A `?var` appears exactly once as the `this:` value of an
    /// assertion body. The variable provides no entity selection
    /// (nothing else binds it), so the assertion would create a
    /// fresh entity — but the user wrote a variable name as if
    /// they meant something specific. Likely they meant to omit
    /// `this:` and let the body derive the entity.
    #[error(
        "variable ?{name} in `this:` isn't bound by anything — \
         omit `this:` if you mean to create a fresh body-derived \
         entity, or query for the existing entity first"
    )]
    SingleOccurrenceVariableAssertionThis {
        /// The variable name (without `?`).
        name: String,
    },
    /// A `?var` appears exactly once in an assertion body's
    /// non-`this:` field. The variable has no value to write —
    /// the assertion would commit a logic variable as a fact,
    /// which is meaningless. This is an error, not a warning,
    /// because no execution path produces useful behavior.
    #[error(
        "variable ?{name} in field {field:?} of an assertion has no value — \
         use a literal, a bare symbol (name lookup), or bind ?{name} \
         in a preceding query"
    )]
    SingleOccurrenceVariableAssertionField {
        /// The variable name (without `?`).
        name: String,
        /// The field where the variable appears.
        field: String,
    },
    /// A raw domain write's literal carries a different value type
    /// than a branch-declared attribute advertises. The write still
    /// commits — raw domains are open-ended — but typed readers (a
    /// concept declaring this attribute) will not see the fact, so
    /// the author gets a heads-up with the spelling that would.
    #[error("{attribute} is declared {declared}, this literal stores {found} — {hint}")]
    DeclaredTypeDivergence {
        /// The declared attribute URI (`io.gozala.person/age`).
        attribute: String,
        /// The declared value type, in `as:` spelling.
        declared: String,
        /// The literal's value type, in `as:` spelling.
        found: String,
        /// How to spell the literal for the declared type.
        hint: String,
    },
}

impl AnalyzeDiagnosticKind {
    /// Stable code for this diagnostic category.
    pub fn code(&self) -> &'static str {
        match self {
            Self::SingleOccurrenceVariableQueryField { .. } => {
                "W_SINGLE_OCCURRENCE_VARIABLE_QUERY_FIELD"
            }
            Self::SingleOccurrenceVariableQueryThis { .. } => {
                "W_SINGLE_OCCURRENCE_VARIABLE_QUERY_THIS"
            }
            Self::SingleOccurrenceVariableAssertionThis { .. } => {
                "W_SINGLE_OCCURRENCE_VARIABLE_ASSERTION_THIS"
            }
            Self::SingleOccurrenceVariableAssertionField { .. } => {
                "E_SINGLE_OCCURRENCE_VARIABLE_ASSERTION_FIELD"
            }
            Self::DeclaredTypeDivergence { .. } => "W_DECLARED_TYPE_DIVERGENCE",
        }
    }
}

/// What went wrong, independent of source location. The
/// [`AnalyzeError`] wrapper adds the `range` and is what
/// callers actually receive.
#[derive(Debug, Error)]
pub enum AnalyzeErrorKind {
    /// Document has zero expressions.
    #[error("document is empty — nothing to analyze")]
    EmptyDocument,
    /// Two heads in the document tried to declare the same
    /// anchor or `?variable` name.
    #[error(
        "name {name:?} declared twice — anchors and variables must be unique within a document"
    )]
    DuplicateName {
        /// The name that was duplicated.
        name: String,
    },
    /// A concept was declared under a name a built-in premise
    /// already claims. Premise heads resolve built-ins before
    /// concepts, so the declaration would be unreachable.
    #[error(
        "{name:?} is a built-in {kind} — a concept declared under that name could never be referenced; pick another name"
    )]
    ReservedName {
        /// The name that collides with a built-in.
        name: String,
        /// What kind of built-in claims it: `formula`, `constraint`,
        /// or `resolver`.
        kind: &'static str,
    },
    /// An anchor and a `?variable` are used with the same name —
    /// they share the same namespace.
    #[error("name {name:?} is used as both an anchor declaration and a variable — pick one")]
    NameShadowing {
        /// The conflicting name.
        name: String,
    },
    /// A mutation references `?var` that no source binds (neither
    /// analysis-time variables nor query bindings).
    #[error(
        "mutation references unbound variable ?{name} — define it earlier with `?{name}:` or bind it via a query"
    )]
    UnboundMutationVariable {
        /// The variable name.
        name: String,
    },
    /// Assertion subject URI didn't parse as an entity.
    #[error("assertion subject {subject:?} is not a valid entity URI: {reason}")]
    InvalidSubjectUri {
        /// The subject text the user wrote.
        subject: String,
        /// Underlying parse error.
        reason: String,
    },
    /// An `&anchor` name doesn't form a valid `id:<name>` entity
    /// URI. The anchor desugars to a `db.meta/name` claim on
    /// `id:<name>`; if that URI can't be built the name could never
    /// be published or resolved. Caught here so it's a clear error
    /// rather than a silently-dropped name at write time.
    #[error(
        "anchor name {name:?} can't be published — `id:{name}` is not a valid entity URI: {reason}"
    )]
    InvalidAnchorName {
        /// The anchor name written after `&`.
        name: String,
        /// Underlying `id:<name>` parse error.
        reason: String,
    },
    /// Assertion body had no fields — nothing to write.
    #[error("assertion `{head}!` has no fields — at least one is required")]
    AssertionWithoutFields {
        /// The head name (without `!`).
        head: String,
    },
    /// `attribute!` body was malformed (missing `the`, invalid
    /// `as`/`cardinality` value, etc.).
    #[error("invalid `attribute!` body: {reason}")]
    InvalidAttributeBody {
        /// Underlying validation message.
        reason: String,
    },
    /// `concept!` body was malformed.
    #[error("invalid `concept!` body: {reason}")]
    InvalidConceptBody {
        /// Underlying validation message.
        reason: String,
    },
    /// Head's concept name didn't resolve to anything known.
    #[error("unknown concept {name:?}: not a built-in and not found on the branch")]
    UnknownConcept {
        /// The concept name that failed to resolve.
        name: String,
    },
    /// A field in the body doesn't appear in the head concept's
    /// `with` map.
    #[error("field {field:?} is not part of concept {concept:?}")]
    UnknownField {
        /// The concept whose schema we were checking against.
        concept: String,
        /// The field name the user wrote.
        field: String,
    },
    /// A `concept!` declared the same field name more than once —
    /// either twice in one block or in both `with:` and `maybe:`.
    /// A field is required *or* optional, never both.
    #[error(
        "field {field:?} is declared more than once in concept {concept:?} \
         (a field cannot be both required and optional)"
    )]
    DuplicateConceptField {
        /// The concept being declared.
        concept: String,
        /// The duplicated field name.
        field: String,
    },
    /// A premise's `where:` names an operand the formula doesn't
    /// have. Unlike concepts (whose schema lives on the branch),
    /// formulas have a fixed operand set, so the analyzer can
    /// list exactly what the formula accepts.
    #[error(
        "operand {operand:?} is not part of formula {formula:?} \
         — valid operands: {valid}"
    )]
    UnknownFormulaOperand {
        /// The formula whose operand schema we checked against.
        formula: String,
        /// The operand name the user wrote.
        operand: String,
        /// Comma-separated list of the formula's valid operands.
        valid: String,
    },
    /// A formula premise omitted a required input operand. The
    /// formula can't compute without it, so this is a hard error
    /// rather than the auto-`?var` fill concept premises get.
    #[error(
        "formula {formula:?} is missing required operand {operand:?} \
         — every input operand must be bound"
    )]
    MissingFormulaOperand {
        /// The formula whose operand was missing.
        formula: String,
        /// The required operand name that wasn't bound.
        operand: String,
    },
    /// A bare-symbol reference in field-value position couldn't
    /// be resolved through the in-doc declarations or branch
    /// name table.
    #[error(
        "field {field:?} references unknown name {name:?} \
         — define it earlier in the document or as an attribute on the branch"
    )]
    UnknownNameReference {
        /// Field where the reference appeared.
        field: String,
        /// The unresolved name.
        name: String,
    },
    /// Claim head with no body fields — claims have no schema to
    /// fall back on.
    #[error(
        "claim head `{domain}:` needs at least one field. \
         Claims have no schema, so the parser cannot infer which \
         attributes to look up. Add the field names you want, e.g. \
         `{domain}:\\n  name: ?name`"
    )]
    ClaimWithoutFields {
        /// The claim domain.
        domain: String,
    },
    /// Claim attribute URI failed dialog's `the:…` validation.
    #[error("invalid attribute {domain:?}/{field:?}: {reason}")]
    InvalidClaimAttribute {
        /// The claim domain.
        domain: String,
        /// The field name.
        field: String,
        /// Underlying validation message.
        reason: String,
    },
    /// Field value used a form the analyzer doesn't accept here.
    #[error("field {field:?} value {form} isn't supported here")]
    UnsupportedFieldValue {
        /// Field where the offending value appeared.
        field: String,
        /// What kind of value it was.
        form: &'static str,
    },
    /// A literal's type contradicts the field's declared `as:`
    /// type. Caught at assert time because the strictly-typed
    /// concept query would never match a fact stored under the
    /// wrong value type, so the entity would silently vanish from
    /// its own concept. A field with no declared type accepts any
    /// literal and never raises this. `expected`/`found` render in
    /// the notation's `as:` vocabulary (e.g. `Text`,
    /// `SignedInteger`) via [`type_label`].
    #[error(
        "field {field:?} expects {} but got a {} literal — \
         quote it for text or fix the attribute's `as:` type",
        type_label(*expected), type_label(*found)
    )]
    TypeMismatch {
        /// Field whose declared type the literal violated.
        field: String,
        /// The field's declared content type.
        expected: dialog_query::Type,
        /// The literal's actual content type.
        found: dialog_query::Type,
    },
    /// Resolver I/O failed.
    #[error("resolver error for {context}: {reason}")]
    ResolverFailed {
        /// What was being resolved.
        context: String,
        /// Underlying message.
        reason: String,
    },
    /// Assertion targets an entity in a reserved URI scheme.
    /// `db:` is reserved for system-published built-ins
    /// (`db:attribute`, `db:concept`, `db:name`); user
    /// assertions cannot modify what lives at these URIs.
    #[error(
        "assertion targets reserved URI {entity} — the `{scheme}:` scheme is system-owned and cannot be written from user notation"
    )]
    ProtectedUri {
        /// The entity URI the assertion tried to target.
        entity: String,
        /// The reserved scheme prefix (e.g. `db`).
        scheme: String,
    },
    /// An assertion creates or names a fresh entity but the
    /// body sets only some of the concept's `with:` fields —
    /// almost always a bug. Either the user meant to update an
    /// existing entity (and forgot to query for it first) or
    /// they meant to set every field. Pinning the partial
    /// shape prevents accidentally creating "ghost" entities
    /// with one or two fields set.
    ///
    /// The error is suppressed in two cases:
    /// - A preceding query expression binds the `?var` in
    ///   `this:` (the user is intentionally updating an
    ///   existing entity, partial updates are fine).
    /// - The body contains `..: _` (the rest-marker explicitly
    ///   declares "I know what I'm doing about every other
    ///   field" — the unmentioned fields get retracted).
    #[error(
        "`{concept}!` body sets only some of the concept's fields ({set:?}; missing: {missing:?}) but {selector_form}. \
         Either query for an existing entity first (`{concept}:\n  this: ?var\n  …`), set every field, or add `..: _` to acknowledge the partial."
    )]
    IncompleteAssertion {
        /// The concept whose schema was being asserted against.
        concept: String,
        /// Fields the user set in the body (with non-blank
        /// values).
        set: Vec<String>,
        /// `with:` fields the user didn't set and didn't
        /// retract.
        missing: Vec<String>,
        /// Human-readable form of how `this:` selected the
        /// entity (e.g. "`this:` is omitted (body-derived
        /// entity)" or "`?alice` is not bound by any query").
        selector_form: String,
    },
    /// Dialog's [`InductiveRule::compile`](dialog_query::InductiveRule)
    /// rejected the lifted rule — body planner failure, unbound
    /// head variable, etc. The message preserves the dialog-level
    /// detail so users see what's wrong.
    #[error("rule compilation failed: {reason}")]
    RuleCompileFailed {
        /// Underlying message from dialog's compiler.
        reason: String,
    },
    /// Two differently named transient commands used as positive rule
    /// triggers have equal or subset required-attribute shapes, so one event
    /// can satisfy both rules.
    #[error(
        "an event intended for transient command {event_command:?} also satisfies {also_matches:?} because their required attributes overlap ({shared_attributes}); give each command verb-specific `the:` paths such as `dataset/toggle` and `dataset/remove`"
    )]
    OverlappingTransientCommands {
        /// The narrower command (or the later command when shapes are equal).
        event_command: String,
        /// The broader command the same event also satisfies.
        also_matches: String,
        /// Stable, sorted list of the attributes shared by both shapes.
        shared_attributes: String,
    },
}

impl AnalyzeErrorKind {
    /// Stable code for this error category. Used as
    /// `Diagnostic.code` at the LSP boundary so editors can
    /// match without scraping the message text.
    pub fn code(&self) -> &'static str {
        match self {
            Self::EmptyDocument => "E_EMPTY_DOCUMENT",
            Self::DuplicateName { .. } => "E_DUPLICATE_NAME",
            Self::NameShadowing { .. } => "E_NAME_SHADOWING",
            Self::UnboundMutationVariable { .. } => "E_UNBOUND_MUTATION_VARIABLE",
            Self::InvalidSubjectUri { .. } => "E_INVALID_SUBJECT_URI",
            Self::InvalidAnchorName { .. } => "E_INVALID_ANCHOR_NAME",
            Self::AssertionWithoutFields { .. } => "E_ASSERTION_WITHOUT_FIELDS",
            Self::InvalidAttributeBody { .. } => "E_INVALID_ATTRIBUTE_BODY",
            Self::InvalidConceptBody { .. } => "E_INVALID_CONCEPT_BODY",
            Self::ReservedName { .. } => "E_RESERVED_NAME",
            Self::UnknownConcept { .. } => "E_UNKNOWN_CONCEPT",
            Self::UnknownField { .. } => "E_UNKNOWN_FIELD",
            Self::DuplicateConceptField { .. } => "E_DUPLICATE_CONCEPT_FIELD",
            Self::UnknownFormulaOperand { .. } => "E_UNKNOWN_FORMULA_OPERAND",
            Self::MissingFormulaOperand { .. } => "E_MISSING_FORMULA_OPERAND",
            Self::UnknownNameReference { .. } => "E_UNKNOWN_NAME_REFERENCE",
            Self::ClaimWithoutFields { .. } => "E_CLAIM_WITHOUT_FIELDS",
            Self::InvalidClaimAttribute { .. } => "E_INVALID_CLAIM_ATTRIBUTE",
            Self::UnsupportedFieldValue { .. } => "E_UNSUPPORTED_FIELD_VALUE",
            Self::TypeMismatch { .. } => "E_TYPE_MISMATCH",
            Self::ResolverFailed { .. } => "E_RESOLVER_FAILED",
            Self::ProtectedUri { .. } => "E_PROTECTED_URI",
            Self::IncompleteAssertion { .. } => "E_INCOMPLETE_ASSERTION",
            Self::RuleCompileFailed { .. } => "E_RULE_COMPILE_FAILED",
            Self::OverlappingTransientCommands { .. } => "E_OVERLAPPING_TRANSIENT_COMMANDS",
        }
    }
}
