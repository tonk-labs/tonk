//! Field-value translation — turns parsed [`FieldValue`]s into
//! the `Term<Any>` slots the engine consumes, plus a few small
//! utilities (scalar coercion, claim-attribute validation,
//! reserved meta-field detection, unbound-variable collection).

use std::collections::HashSet;

use dialog_artifacts::{Entity, Value};
use dialog_query::{Term, Type, attribute::The as AttributeThe};
use tonk_notation::{FieldValue, Scalar};

use super::error::{AnalyzeError, AnalyzeErrorKind};
use super::resolver::Resolver;
use super::scope::Scope;
use crate::analyzer::Working;
use tonk_core::transact::Application;

/// Reserved body field names that don't correspond to schema
/// fields: `this:` (entity selection), `..:` (rest-of-attributes
/// retraction marker).
pub(crate) fn is_meta_field(name: &str) -> bool {
    matches!(name, "this" | "..")
}

/// Translate a parsed [`FieldValue`] into the `Term<Any>` slot it
/// belongs in. Bare symbols resolve at analysis time (against
/// in-doc declarations first, then the branch's name table);
/// variables resolve against `analysis.variables` if known,
/// otherwise stay as `Term::Variable` so planning can substitute
/// them later; literals become `Term::Constant`; blanks become
/// `Term::blank()`.
///
/// `expected` carries the field's declared [`Type`] (from the
/// concept's attribute descriptor) when known. It disambiguates
/// integer literals: the notation parser always produces a signed
/// `Scalar::Integer` for a non-negative literal like `1`, so a
/// field declared `as: unsigned-integer` needs schema-directed
/// coercion. Pass `None` for slots with no declared type (`this`,
/// claim attributes, formula operands).
pub(crate) async fn field_value_to_term<R: Resolver>(
    field_name: &str,
    value: &FieldValue,
    range: lsp_types::Range,
    scope: &Scope<'_, R>,
    analysis: &Working,
    expected: Option<Type>,
) -> Result<Term<dialog_query::Any>, AnalyzeError> {
    Ok(match value {
        FieldValue::Literal(scalar) => {
            Term::Constant(scalar_to_value(scalar, expected).map_err(|e| e.with_range(range))?)
        }
        FieldValue::Variable(name) => {
            // If this variable was derived in Phase 1, substitute
            // the entity now; otherwise leave it as a variable
            // that planning will bind from query results.
            if let Some(entity) = analysis.variables.get(name) {
                Term::Constant(Value::Entity(entity.clone()))
            } else {
                Term::<dialog_query::Any>::var(name)
            }
        }
        FieldValue::Symbol(name) => {
            // Bare lowercase symbol — name-table lookup. Same
            // resolution order as the old `.bookmark` form:
            //   1. Doc-local declarations (head anchor from the
            //      same document — `concept!: &foo` or
            //      `attribute!: &foo`).
            //   2. Doc-local attribute by name.
            //   3. Branch entity with `dialog.meta/name = name`.
            if let Some(entity) = scope.lookup_entity(name) {
                Term::Constant(Value::Entity(entity))
            } else if let Some(resolved) = scope.resolve_attribute(name).await.map_err(|e| {
                AnalyzeError::at(
                    AnalyzeErrorKind::ResolverFailed {
                        context: format!("symbol {name}"),
                        reason: e.to_string(),
                    },
                    range,
                )
            })? {
                Term::Constant(Value::Entity(resolved.entity))
            } else if let Some(entity) = scope.resolve_named_entity(name).await.map_err(|e| {
                AnalyzeError::at(
                    AnalyzeErrorKind::ResolverFailed {
                        context: format!("symbol {name}"),
                        reason: e.to_string(),
                    },
                    range,
                )
            })? {
                Term::Constant(Value::Entity(entity))
            } else {
                return Err(AnalyzeError::at(
                    AnalyzeErrorKind::UnknownBookmark {
                        field: field_name.into(),
                        bookmark: name.clone(),
                    },
                    range,
                ));
            }
        }
        FieldValue::Uri(uri) => {
            let entity: Entity =
                uri.parse()
                    .map_err(|e: dialog_artifacts::DialogArtifactsError| {
                        AnalyzeError::at(
                            AnalyzeErrorKind::InvalidSubjectUri {
                                subject: uri.clone(),
                                reason: e.to_string(),
                            },
                            range,
                        )
                    })?;
            Term::Constant(Value::Entity(entity))
        }
        FieldValue::Blank => {
            // Mint an auto-named variable rather than a true
            // blank so the engine binds the matched value into
            // the frame. The renderer detects the `__`-prefixed
            // name [`Term::unique`] uses and projects the value
            // back under the user-facing field name. Without
            // this the result block silently omits `_`-marked
            // fields, which is confusing — the user wrote `_`
            // to opt out of *binding* (joining), not to opt out
            // of *seeing* the value.
            Term::<dialog_query::Any>::unique()
        }
        FieldValue::Nested(_) => {
            return Err(AnalyzeError::at(
                AnalyzeErrorKind::UnsupportedFieldValue {
                    field: field_name.into(),
                    form: "nested mapping (only `concept!`'s `with:` accepts a nested map)",
                },
                range,
            ));
        }
        FieldValue::Premises(_) => {
            // Premises only make sense as the value of `when:` /
            // `unless:` inside a `rule!:` claim body — the rule
            // lift consumes them there before this generic
            // field-to-term path runs. Reaching this arm means
            // the user put `when:` / `unless:` somewhere it does
            // not belong.
            return Err(AnalyzeError::at(
                AnalyzeErrorKind::UnsupportedFieldValue {
                    field: field_name.into(),
                    form: "premise list (only valid under `when:` / `unless:` inside a `rule!:` body)",
                },
                range,
            ));
        }
    })
}

/// Translate a parsed [`Scalar`] into a [`Value`].
///
/// `expected` carries the field's declared [`Type`] when known.
/// The notation parser always parses a non-negative integer
/// literal as a signed `Scalar::Integer` (it falls back to
/// unsigned only for values too large for `i128`). When the
/// declared type is `Type::UnsignedInt` and the literal is
/// non-negative, coerce to `Value::UnsignedInt`. Every other case
/// keeps the parsed scalar's natural mapping: a negative
/// `Integer` stays signed, an explicit `UnsignedInteger` stays
/// unsigned, and a `None` or non-integer `expected` changes
/// nothing.
pub(crate) fn scalar_to_value(
    scalar: &Scalar,
    expected: Option<Type>,
) -> Result<Value, AnalyzeError> {
    Ok(match scalar {
        Scalar::String(s) => Value::String(s.clone()),
        Scalar::Boolean(b) => Value::Boolean(*b),
        Scalar::Integer(i) if *i >= 0 && expected == Some(Type::UnsignedInt) => {
            Value::UnsignedInt(*i as u128)
        }
        Scalar::Integer(i) => Value::SignedInt(*i),
        Scalar::UnsignedInteger(u) => Value::UnsignedInt(*u),
        Scalar::Float(f) => Value::Float(*f),
        Scalar::Null => {
            return Err(AnalyzeErrorKind::UnsupportedFieldValue {
                field: "<scalar>".into(),
                form: "null literal",
            }
            .into());
        }
    })
}

pub(crate) fn scalar_to_string(scalar: &Scalar) -> Result<String, AnalyzeError> {
    Ok(match scalar {
        Scalar::String(s) => s.clone(),
        Scalar::Boolean(b) => b.to_string(),
        Scalar::Integer(i) => i.to_string(),
        Scalar::UnsignedInteger(u) => u.to_string(),
        Scalar::Float(f) => f.to_string(),
        Scalar::Null => {
            return Err(AnalyzeErrorKind::UnsupportedFieldValue {
                field: "<scalar>".into(),
                form: "null literal",
            }
            .into());
        }
    })
}

pub(crate) fn validate_claim_attribute(
    domain: &str,
    field: &str,
    range: lsp_types::Range,
) -> Result<(), AnalyzeError> {
    let uri = format!("{domain}/{field}");
    uri.parse::<AttributeThe>().map(|_| ()).map_err(|e| {
        AnalyzeError::at(
            AnalyzeErrorKind::InvalidClaimAttribute {
                domain: domain.to_owned(),
                field: field.to_owned(),
                reason: format!("{e}"),
            },
            range,
        )
    })
}

pub(crate) fn collect_unbound_variables(
    application: &Application,
    analysis: &Working,
    out: &mut HashSet<String>,
) {
    for name in application.bindings() {
        if !analysis.variables.contains_key(&name) {
            out.insert(name);
        }
    }
}
