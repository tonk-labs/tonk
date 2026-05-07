//! Assertion-side analysis — `build_assertion_application`,
//! `derive_head_intent`, and the entity-derivation helpers
//! (`this_term_for_assertion`, `body_digest`).

use std::collections::BTreeMap;

use dialog_artifacts::{Entity, Value};
use dialog_query::{Parameters, Term, concept::query::ConceptQuery};
use tonk_notation::{Anchor, Assertion, Field, FieldValue, HeadName, Scalar};

use super::error::AnalyzeError;
use super::field::{field_value_to_term, is_meta_field, validate_claim_attribute};
use super::resolver::Resolver;
use super::scope::Scope;
use crate::prelude::EntityExt;
use crate::transact::{Analysis, Application, DomainApplication, ThisIntent};

/// Output of analyzing a single `head!:` expression. An
/// expression can produce up to two statements:
///
/// - `assert` — the explicit non-blank fields the user wrote.
///   Carries the published `&name` if any.
/// - `retract` — fields whose value was `_` (per-field
///   retraction) plus, if `..: _` is present, every other
///   `with:` attribute on the concept that wasn't mentioned
///   explicitly. No `name` here — naming is an additive
///   operation, only the assert side carries it.
///
/// `None` for either side means "no work on this side." A
/// simple `person!: &alice\n  name: "Alice"` produces
/// `(Some(assert), None)`. A `person!:\n  this: ?p\n  ..: _`
/// produces `(None, Some(retract))`. The mixed form
/// `person!:\n  this: ?p\n  name: ?name\n  ..: _` produces
/// `(Some(assert), Some(retract))`.
#[derive(Debug, Clone)]
pub(crate) struct AssertionPlan {
    pub assert: Option<Application>,
    pub retract: Option<Application>,
}

pub(crate) async fn build_assertion_application<R: Resolver>(
    assertion: &Assertion,
    scope: &Scope<'_, R>,
    analysis: &mut Analysis,
) -> Result<AssertionPlan, AnalyzeError> {
    let head_label = match &assertion.head.name {
        HeadName::Concept(name) => name.clone(),
        HeadName::Claim(domain) => domain.clone(),
        HeadName::Uri(uri) => uri.clone(),
    };

    if assertion.fields.is_empty() {
        return Err(AnalyzeError::AssertionWithoutFields { head: head_label });
    }

    let (this, name) =
        derive_head_intent(&assertion.fields, assertion.anchor.as_ref(), scope).await?;
    if let ThisIntent::Uri(entity) = &this {
        check_writable(entity)?;
    }
    let this_term = this_term_for_assertion(&this, &name, &assertion.fields, analysis)?;

    // Detect the rest-marker `..: _` once. Per-field `_`
    // blanks are handled in the per-field walk below.
    let has_rest_retraction = assertion
        .fields
        .iter()
        .any(|f| f.name == ".." && matches!(f.value, FieldValue::Blank));

    match &assertion.head.name {
        HeadName::Concept(concept_name) => {
            let resolved = scope
                .resolve_concept(concept_name)
                .await
                .map_err(|e| AnalyzeError::ResolverFailed {
                    context: format!("concept {concept_name:?}"),
                    reason: e.message,
                })?
                .ok_or_else(|| AnalyzeError::UnknownConcept {
                    name: concept_name.clone(),
                })?;

            // Walk user-supplied fields, separating asserts from
            // retracts. `..: _` and per-field `_` blanks go
            // to the retract pile; everything else asserts.
            let mut user_fields: BTreeMap<&str, &FieldValue> = BTreeMap::new();
            for field in &assertion.fields {
                if is_meta_field(&field.name) {
                    continue;
                }
                user_fields.insert(field.name.as_str(), &field.value);
            }

            let mut assert_terms = Parameters::new();
            assert_terms.insert("this".into(), this_term.clone());
            let mut retract_terms = Parameters::new();
            retract_terms.insert("this".into(), this_term.clone());
            let mut any_assert = false;
            let mut any_retract = false;

            for (field_name, _attr) in resolved.descriptor.with().iter() {
                match user_fields.remove(field_name) {
                    Some(FieldValue::Blank) => {
                        // Per-field retraction: planner walks the
                        // branch to find the current value(s) and
                        // dissociates each.
                        retract_terms.insert(field_name.into(), Term::<dialog_query::Any>::blank());
                        any_retract = true;
                        // Assert side gets a blank so the
                        // emitter skips it.
                        assert_terms.insert(field_name.into(), Term::<dialog_query::Any>::blank());
                    }
                    Some(value) => {
                        // Explicit non-blank field — asserts.
                        let term = field_value_to_term(field_name, value, scope, analysis).await?;
                        assert_terms.insert(field_name.into(), term);
                        any_assert = true;
                        // Retract side blanks this field — the
                        // emitter skips blanks on retract, so
                        // an asserted field never gets
                        // accidentally dropped on the same pass.
                        retract_terms.insert(field_name.into(), Term::<dialog_query::Any>::blank());
                    }
                    None => {
                        // Field not mentioned. Behavior depends
                        // on the rest-marker:
                        //   - With `..: _`: retract (alongside
                        //     other unnamed fields)
                        //   - Without: leave blank on assert
                        //     side, blank on retract side
                        let blank = Term::<dialog_query::Any>::blank();
                        assert_terms.insert(field_name.into(), blank.clone());
                        retract_terms.insert(field_name.into(), blank);
                        if has_rest_retraction {
                            any_retract = true;
                        }
                    }
                }
            }

            if let Some((unknown, _)) = user_fields.into_iter().next() {
                return Err(AnalyzeError::UnknownField {
                    concept: concept_name.clone(),
                    field: unknown.to_owned(),
                });
            }

            // Build the assert/retract `Application`s. Naming
            // (`name: Some(...)`) only attaches to the assert
            // side — the planner emits the desugared `name!`
            // assertion on `id:<name>` which is itself an
            // additive write. Putting `name` on the retract
            // side would erase the published name.
            let assert_app = if any_assert || (name.is_some() && !has_rest_retraction) {
                Some(Application::Concept {
                    query: ConceptQuery {
                        terms: assert_terms,
                        predicate: resolved.descriptor.clone(),
                    },
                    this: this.clone(),
                    name: name.clone(),
                })
            } else if name.is_some() {
                // `&name` published, but every field is being
                // retracted (`..: _` with no other set fields).
                // Still need an assert pass to publish the name.
                Some(Application::Concept {
                    query: ConceptQuery {
                        terms: {
                            let mut t = Parameters::new();
                            t.insert("this".into(), this_term.clone());
                            for (field_name, _) in resolved.descriptor.with().iter() {
                                t.insert(field_name.into(), Term::<dialog_query::Any>::blank());
                            }
                            t
                        },
                        predicate: resolved.descriptor.clone(),
                    },
                    this: this.clone(),
                    name: name.clone(),
                })
            } else {
                None
            };
            let retract_app = if any_retract {
                Some(Application::Concept {
                    query: ConceptQuery {
                        terms: retract_terms,
                        predicate: resolved.descriptor,
                    },
                    this,
                    name: None,
                })
            } else {
                None
            };
            Ok(AssertionPlan {
                assert: assert_app,
                retract: retract_app,
            })
        }
        HeadName::Claim(domain) => {
            // Claim heads don't yet support retraction
            // semantics — they have no schema to enumerate, so
            // `..: _` doesn't have a closed set of attributes
            // to expand into. Field-level `_` is also not
            // wired here (Stage 2.7+ extension).
            let mut parameters = Parameters::new();
            parameters.insert("this".into(), this_term);
            for field in &assertion.fields {
                if is_meta_field(&field.name) {
                    continue;
                }
                validate_claim_attribute(domain, &field.name)?;
                let term = field_value_to_term(&field.name, &field.value, scope, analysis).await?;
                parameters.insert(field.name.clone(), term);
            }
            Ok(AssertionPlan {
                assert: Some(Application::Domain {
                    application: DomainApplication {
                        domain: domain.clone(),
                        parameters,
                    },
                    this,
                    name,
                }),
                retract: None,
            })
        }
        HeadName::Uri(uri) => Err(AnalyzeError::UnsupportedFieldValue {
            field: uri.clone(),
            form: "URI head in assertion (not yet implemented in Stage 2.1)",
        }),
    }
}

/// Derive the head's source-form intent — `(ThisIntent, name)`
/// — from an expression's body and optional value-side anchor.
///
/// Under the new grammar the head carries no binding token; the
/// two intent axes live in the body and value side:
///
/// - **Entity selection** — the body's `this:` field. Mapping:
///   - omitted → [`ThisIntent::Derived`]
///   - `?var` → [`ThisIntent::Variable(var)`]
///   - `did:key:…` / `id:…` / `db:…` → [`ThisIntent::Uri(entity)`]
///   - bare symbol → resolved through the name table to
///     [`ThisIntent::Uri(entity)`]. Resolution order matches
///     [`super::field::field_value_to_term`]: doc-local
///     declarations first, then the branch's `dialog.meta/name`
///     index. Unresolvable symbols surface as `UnknownBookmark`.
///
/// - **Naming** — the `&name` on the value side, captured by the
///   parser as `Anchor`. Returned as `Some(name)` when present.
///
/// The two are independent: every combination is meaningful
/// (e.g. `person!: &alice\n  this: did:key:zX` → publish `id:alice`
/// pointing at zX without producing a new entity).
pub(crate) async fn derive_head_intent<R: Resolver>(
    fields: &[Field],
    anchor: Option<&Anchor>,
    scope: &Scope<'_, R>,
) -> Result<(ThisIntent, Option<String>), AnalyzeError> {
    let name = anchor.map(|a| a.name.clone());
    let this = match fields.iter().find(|f| f.name == "this") {
        None => ThisIntent::Derived,
        Some(field) => match &field.value {
            FieldValue::Variable(v) => ThisIntent::Variable(v.clone()),
            FieldValue::Uri(uri) => {
                let entity: Entity =
                    uri.parse()
                        .map_err(|e: dialog_artifacts::DialogArtifactsError| {
                            AnalyzeError::InvalidSubjectUri {
                                subject: uri.clone(),
                                reason: e.to_string(),
                            }
                        })?;
                ThisIntent::Uri(entity)
            }
            FieldValue::Symbol(name) => {
                // Bare symbol in `this:` — resolve through the
                // name table. Same order as field-value
                // `Symbol`: doc-local declarations, then
                // doc-local attributes, then branch lookup.
                let entity = if let Some(entity) = scope.lookup_entity(name) {
                    entity
                } else if let Some(resolved) = scope.resolve_attribute(name).await.map_err(|e| {
                    AnalyzeError::ResolverFailed {
                        context: format!("symbol {name}"),
                        reason: e.message,
                    }
                })? {
                    resolved.entity
                } else if let Some(entity) =
                    scope.resolve_named_entity(name).await.map_err(|e| {
                        AnalyzeError::ResolverFailed {
                            context: format!("symbol {name}"),
                            reason: e.message,
                        }
                    })?
                {
                    entity
                } else {
                    return Err(AnalyzeError::UnknownBookmark {
                        field: "this".into(),
                        bookmark: name.clone(),
                    });
                };
                ThisIntent::Uri(entity)
            }
            FieldValue::Literal(_) | FieldValue::Blank | FieldValue::Nested(_) => {
                return Err(AnalyzeError::UnsupportedFieldValue {
                    field: "this".into(),
                    form: "expected `?var`, a bare symbol, or a URI \
                           (`did:key:…`, `id:…`, `db:…`)",
                });
            }
        },
    };
    Ok((this, name))
}

/// What to put in the `this` slot of a mutation [`Application`].
///
/// Driven by the entity-selection axis (`this`) and the optional
/// published name (`name`). The two axes are orthogonal — both
/// can be present, both can be absent.
///
/// - `Derived` + no `name`: mint a body-content-derived entity.
/// - `Derived` + `name`: body-derived entity. The
///   `dialog.meta/name` claim on `id:<name>` is emitted by the
///   planner from `ApplicationPlan::name`, not as a parameter
///   on the predicate. Also registers the name → entity binding
///   in `analysis.declarations` so duplicate-name checks across
///   heads catch overlaps.
/// - `Variable(name)` already in `analysis.variables`: substitute
///   the registered entity.
/// - `Variable(name)` not yet known: if there's no query
///   binding for it, mint a body-derived entity and register it
///   in `analysis.variables` so subsequent uses share the
///   entity. If a query binding exists, leave as
///   `Term::Variable` — planning will substitute from the
///   query frame.
/// - `Uri(entity)`: substitute directly. With `name`, this is
///   the "publish a name pointing at an existing entity" form.
fn this_term_for_assertion(
    this: &ThisIntent,
    name: &Option<String>,
    fields: &[Field],
    analysis: &mut Analysis,
) -> Result<Term<dialog_query::Any>, AnalyzeError> {
    Ok(match this {
        ThisIntent::Derived => {
            let entity = Entity::of(&body_digest(fields));
            if let Some(name) = name {
                if let Some(prior) = analysis.declarations.get(name)
                    && prior != &entity
                {
                    return Err(AnalyzeError::DuplicateName { name: name.clone() });
                }
                analysis.declarations.insert(name.clone(), entity.clone());
            }
            Term::Constant(Value::Entity(entity))
        }
        ThisIntent::Variable(var) => {
            if let Some(entity) = analysis.variables.get(var) {
                Term::Constant(Value::Entity(entity.clone()))
            } else if query_binds(analysis, var) {
                // Bound at planning time from query results.
                Term::<dialog_query::Any>::var(var)
            } else {
                // First introduction — mint a body-derived
                // entity and register it for later expressions
                // that share `?name`.
                let entity = Entity::of(&body_digest(fields));
                analysis.variables.insert(var.clone(), entity.clone());
                Term::Constant(Value::Entity(entity))
            }
        }
        ThisIntent::Uri(entity) => Term::Constant(Value::Entity(entity.clone())),
    })
}

/// Hash-stable summary of an assertion body — pairs of
/// `(field_name, FieldDigest)` sorted by name. Used by
/// `Entity::of` to derive a content-addressed entity for
/// `Derived` and unbound `Variable` heads.
///
/// Only literal scalars contribute. Variables, references, and
/// blanks are skipped — they're not part of the entity's
/// identity (they'd reference *other* entities, and including
/// them in the hash would defeat the deterministic-rerun
/// property).
///
/// Pure function of `fields` (no scope, no resolver), so it's
/// safe to call from Phase 1 to pre-compute the entity for an
/// anchor declaration.
pub(super) fn body_digest(fields: &[Field]) -> Vec<(String, FieldDigest)> {
    let mut out: Vec<(String, FieldDigest)> = Vec::new();
    for field in fields {
        let digest = match &field.value {
            FieldValue::Literal(scalar) => FieldDigest::from_scalar(scalar),
            // Skip variables, references, blanks, nested.
            _ => continue,
        };
        out.push((field.name.clone(), digest));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Serializable shadow of [`Scalar`] used only by
/// [`body_digest`]. Round-trips the scalar's primitive value so
/// `Entity::of` can hash it deterministically. Distinct from
/// `Scalar` because we want a stable serde representation
/// independent of any future surface-syntax changes.
#[derive(serde::Serialize)]
#[serde(untagged)]
pub(super) enum FieldDigest {
    String(String),
    Integer(i128),
    UnsignedInteger(u128),
    Float(f64),
    Boolean(bool),
    Null,
}

impl FieldDigest {
    fn from_scalar(scalar: &Scalar) -> Self {
        match scalar {
            Scalar::String(s) => Self::String(s.clone()),
            Scalar::Integer(i) => Self::Integer(*i),
            Scalar::UnsignedInteger(u) => Self::UnsignedInteger(*u),
            Scalar::Float(f) => Self::Float(*f),
            Scalar::Boolean(b) => Self::Boolean(*b),
            Scalar::Null => Self::Null,
        }
    }
}

/// Does `analysis.query` bind `?name`? Used by
/// [`this_term_for_assertion`] to decide between minting a
/// body-derived entity and leaving the variable for planning.
fn query_binds(analysis: &Analysis, name: &str) -> bool {
    analysis
        .query
        .as_ref()
        .map(|q| q.bindings().contains(name))
        .unwrap_or(false)
}

/// Reject assertions targeting a system-reserved URI scheme.
///
/// Per the guide, the `db:` scheme is reserved for built-in
/// entities (`db:attribute`, `db:concept`, `db:name`); user
/// assertions cannot modify what lives at these URIs.
///
/// The check fires on any resolved [`ThisIntent::Uri`], whether
/// the user wrote `this: db:concept` directly or named a symbol
/// that happens to resolve to a `db:`-prefixed entity.
fn check_writable(entity: &Entity) -> Result<(), AnalyzeError> {
    const RESERVED_SCHEMES: &[&str] = &["db"];
    let s = entity.to_string();
    for scheme in RESERVED_SCHEMES {
        let prefix = format!("{scheme}:");
        if s.starts_with(&prefix) {
            return Err(AnalyzeError::ProtectedUri {
                entity: s,
                scheme: (*scheme).to_owned(),
            });
        }
    }
    Ok(())
}
