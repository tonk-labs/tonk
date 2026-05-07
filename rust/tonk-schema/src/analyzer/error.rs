//! Errors raised by [`crate::analyzer::analyze`].

use thiserror::Error;

/// Errors raised while analyzing a [`tonk_notation::Syntax`] tree.
#[derive(Debug, Error)]
pub enum AnalyzeError {
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
    /// A bare-symbol reference in field-value position couldn't
    /// be resolved through the in-doc declarations or branch
    /// name table.
    #[error(
        "field {field:?} references unknown name {bookmark:?} \
         — define it earlier in the document or as an attribute on the branch"
    )]
    UnknownBookmark {
        /// Field where the reference appeared.
        field: String,
        /// The unresolved name.
        bookmark: String,
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
}
