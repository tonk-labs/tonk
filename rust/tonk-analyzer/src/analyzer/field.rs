//! Field-value translation — turns parsed [`FieldValue`]s into
//! the `Term<Any>` slots the engine consumes, plus a few small
//! utilities (scalar coercion, claim-attribute validation,
//! reserved meta-field detection, unbound-variable collection).

use std::collections::HashSet;

use dialog_artifacts::{Entity, Value};
use dialog_query::{Term, Type, attribute::The as AttributeThe};
use tonk_notation::{FieldValue, Scalar};

use super::error::{AnalyzeError, AnalyzeErrorKind};
use super::scope::Scope;
use crate::analyzer::Working;
use tonk_schema::transact::Application;

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
/// Lower a keyed-collection field's value to its `(key, value)`
/// terms. The entry form `{?key: ?value}` binds both halves: a
/// `?var` key is a variable, `_` a blank, anything else a literal
/// key. A bare value binds every entry with the key left blank.
pub(crate) fn collection_entry_terms(
    field_name: &str,
    value: &FieldValue,
    range: lsp_types::Range,
    scope: &Scope,
    analysis: &Working,
    expected: Option<Type>,
) -> Result<(Term<dialog_query::Any>, Term<dialog_query::Any>), AnalyzeError> {
    let FieldValue::Nested(inner) = value else {
        let value = field_value_to_term(field_name, value, range, scope, analysis, expected)?;
        return Ok((Term::<dialog_query::Any>::blank(), value));
    };
    let [entry] = inner.as_slice() else {
        return Err(AnalyzeError::at(
            AnalyzeErrorKind::UnsupportedFieldValue {
                field: field_name.into(),
                form: "a collection entry is one `{key: value}` pair",
            },
            range,
        ));
    };
    let key = match entry.name.as_str() {
        "_" => Term::<dialog_query::Any>::blank(),
        name => match name.strip_prefix('?') {
            Some(variable) => Term::<dialog_query::Any>::var(variable),
            None => Term::Constant(Value::String(name.to_owned())),
        },
    };
    // `{key: _}` RETRACTS the entry, so the value must stay a true
    // blank. `field_value_to_term` mints an auto-named variable for a
    // blank instead — right for a query, where `_` means "match
    // anything and project it back", but here it makes the caller's
    // `is_blank()` check false, so the entry compiles as an assertion
    // of an unbound variable and the mutation is rejected outright.
    let value = if matches!(entry.value, FieldValue::Blank) {
        Term::<dialog_query::Any>::blank()
    } else {
        field_value_to_term(
            field_name,
            &entry.value,
            entry.value_range,
            scope,
            analysis,
            expected,
        )?
    };
    Ok((key, value))
}

pub(crate) fn field_value_to_term(
    field_name: &str,
    value: &FieldValue,
    range: lsp_types::Range,
    scope: &Scope,
    analysis: &Working,
    expected: Option<Type>,
) -> Result<Term<dialog_query::Any>, AnalyzeError> {
    Ok(match value {
        FieldValue::Literal(scalar) => Term::Constant(
            scalar_to_value(scalar, expected)
                .map_err(|e| e.with_field(field_name).with_range(range))?,
        ),
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
            // Bare lowercase symbol — sync name-table lookup. The
            // table was populated during the resolve pass (in-doc
            // declarations, plus any branch entities prefetched up
            // front), so resolution here never touches the branch.
            if let Some(entity) = scope.symbol(name) {
                Term::Constant(Value::Entity(entity))
            } else {
                return Err(AnalyzeError::at(
                    AnalyzeErrorKind::UnknownNameReference {
                        field: field_name.into(),
                        name: name.clone(),
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
/// `expected` carries the field's declared [`Type`] when known. The
/// notation parser types a literal only by its lexical form, not by
/// the field it lands in, so this is where a literal is checked
/// against the field's declared type:
///
/// - A non-negative `Scalar::Integer` (the parser's default for
///   `1`, which only falls back to unsigned for values too large
///   for `i128`) coerces to `Value::UnsignedInt` when the field is
///   declared `as: unsigned-integer`.
/// - Otherwise the scalar must already match the declared type. A
///   mismatch (e.g. a bare integer `3` written into an `as: text`
///   field) is a [`TypeMismatch`](AnalyzeErrorKind::TypeMismatch)
///   error rather than a silently-coerced or
///   wrong-typed fact: the strict concept query would never match
///   such a fact, so the entity would vanish from its own concept
///   with no signal. Surfacing it points at the schema mistake
///   directly.
///
/// A `None` `expected` (untyped slots — `this`, claim attributes,
/// formula operands) keeps the parsed scalar's natural mapping.
/// The declared attribute with its value type stripped: a domain head
/// takes a declaration's identity and cardinality, but never its type
/// — raw domains are open-ended and mixed-type attributes are legal.
pub(crate) fn untyped_descriptor(
    descriptor: &dialog_query::AttributeDescriptor,
) -> dialog_query::AttributeDescriptor {
    let mut json = serde_json::to_value(descriptor).expect("a descriptor serializes");
    if let Some(map) = json.as_object_mut() {
        map.remove("as");
    }
    serde_json::from_value(json).expect("a descriptor without `as` deserializes")
}

pub(crate) fn scalar_to_value(
    scalar: &Scalar,
    expected: Option<Type>,
) -> Result<Value, AnalyzeError> {
    // Schema-directed integer coercion: a non-negative signed
    // literal fills an unsigned field.
    if let (Scalar::Integer(i), Some(Type::UnsignedInt)) = (scalar, expected)
        && *i >= 0
    {
        return Ok(Value::UnsignedInt(*i as u128));
    }
    // ...and a bare (unsigned-spelled) literal fills a signed field.
    if let (Scalar::UnsignedInteger(u), Some(Type::SignedInt)) = (scalar, expected)
        && *u <= i128::MAX as u128
    {
        return Ok(Value::SignedInt(*u as i128));
    }

    let value = match scalar {
        Scalar::String(s) => Value::String(s.clone()),
        Scalar::Boolean(b) => Value::Boolean(*b),
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
    };

    // Reject a literal whose type contradicts the field's declared
    // type. `data_type()` is the concrete [`Type`] the value
    // inhabits; an `expected` that doesn't admit it is a schema
    // mismatch the user should see at assert time. A `None`
    // `expected` (untyped slot) admits any type.
    if let Some(expected) = expected
        && value.data_type() != expected
    {
        return Err(AnalyzeErrorKind::TypeMismatch {
            field: "<scalar>".into(),
            expected,
            found: value.data_type(),
        }
        .into());
    }

    Ok(value)
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
