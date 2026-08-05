//! On-the-wire shape for `/transact` requests — typed claims that
//! carry concept-level transient/durable classification through to
//! the reactor's transaction builder.
//!
//! See `plan/transact-endpoint.md` for the design. The short
//! version: every assertion or retraction names a predicate
//! ([`ConceptDescriptor`]) along with its parameter bindings
//! ([`PredicateApplication`]); the predicate wrapper carries
//! whether the concept is durable (carries forward across
//! commits) or transient (one-timestep lifetime, retracted
//! before durable write). The reactor reads this classification
//! to bucket transients without re-querying the schema.

use crate::command::SourceInvocation;
use crate::meta::AnchorName;
use dialog_artifacts::{Attribute, Value, ValueDataType};
use dialog_query::ConceptDescriptor as DialogConceptDescriptor;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Parameter bindings carried on the wire. Each entry is a
/// concrete [`Value`] (entity URI, scalar, ref) — the wire format
/// has no representation for logic variables or blanks. The
/// dialog-query [`Term`](dialog_query::Term)-flavoured `Parameters`
/// is used downstream after [`crate::claim`]-time lift; on the
/// wire we keep the surface narrow so the worker never has to
/// defend against terms that don't make sense for an assertion.
pub type ValueMap = IndexMap<String, Value>;

/// Errors raised when projecting a wire-form [`SourceApplication`]
/// into the validated [`PredicateApplication`].
///
/// Each variant names the offending field so the worker can return
/// a message that points at the schema mistake, the same way the
/// notation analyzer surfaces `TypeMismatch` on the `eval` path.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum TransactError {
    /// A field's value can't be represented as the field's declared
    /// type and can't be losslessly coerced to it (e.g. `"hi"` into
    /// a numeric field, or `27.5` into an integer field).
    #[error("field {field:?} expects {expected} but got an incompatible {found} value")]
    TypeMismatch {
        /// The concept field name.
        field: String,
        /// The type the field's `as:` declares.
        expected: ValueDataType,
        /// The type the supplied value actually inhabits.
        found: ValueDataType,
    },
    /// A parameter was supplied that the predicate's `with:` map
    /// does not declare (excluding the reserved `this` slot).
    #[error("field {field:?} is not declared by this concept")]
    UnknownField {
        /// The undeclared parameter name.
        field: String,
    },
    /// Nominal invocations need the target branch's current command
    /// schema and consumer indexes, so they cannot be converted through
    /// the structural predicate path.
    #[error(
        "command {command} requires authoritative branch resolution before it can be transacted"
    )]
    InvocationRequiresResolution {
        /// The nominal command kind that still needs resolving.
        command: dialog_artifacts::Entity,
    },
}

/// Coerce `value` to the field's declared type `expected`, accepting
/// only lossless conversions and rejecting everything else.
///
/// The wire format (JSON from `tonk.transact`) has no integer type —
/// every JavaScript number deserializes to [`Value::Float`]. So an
/// integral float (`27.0`) bound to a `SignedInteger`/`UnsignedInteger`
/// field is the common, benign case and is narrowed to the integer
/// value. A fractional float, or a negative into an unsigned field,
/// would lose information and is a [`TransactError::TypeMismatch`]
/// instead.
///
/// This is deliberately more permissive on the int/float axis than
/// the notation analyzer's `scalar_to_value`, which keeps a bare
/// `27.0` literal a float: notation authors write `27` (a real
/// integer literal) when they mean an integer, so the analyzer has
/// no reason to bridge floats. The wire has no such literal, so the
/// transact path must.
///
/// `expected == None` means the field accepts any type (untyped
/// claim attributes, the `this` slot) and the value passes through
/// untouched.
pub fn coerce_value(
    field: &str,
    expected: Option<ValueDataType>,
    value: Value,
) -> Result<Value, TransactError> {
    let Some(expected) = expected else {
        return Ok(value);
    };
    let found = value.data_type();
    if found == expected {
        return Ok(value);
    }
    let coerced = match (expected, &value) {
        // An integral float fills an integer field. JS numbers are
        // always floats, so this is the path UI-created integers take.
        (ValueDataType::SignedInt, Value::Float(f)) if f.fract() == 0.0 && i128_in_range(*f) => {
            Some(Value::SignedInt(*f as i128))
        }
        (ValueDataType::UnsignedInt, Value::Float(f))
            if f.fract() == 0.0 && *f >= 0.0 && u128_in_range(*f) =>
        {
            Some(Value::UnsignedInt(*f as u128))
        }
        // Cross-coerce between integer widths when the value fits.
        (ValueDataType::UnsignedInt, Value::SignedInt(i)) if *i >= 0 => {
            Some(Value::UnsignedInt(*i as u128))
        }
        (ValueDataType::SignedInt, Value::UnsignedInt(u)) if *u <= i128::MAX as u128 => {
            Some(Value::SignedInt(*u as i128))
        }
        // An integer fills a float field exactly.
        (ValueDataType::Float, Value::SignedInt(i)) => Some(Value::Float(*i as f64)),
        (ValueDataType::Float, Value::UnsignedInt(u)) => Some(Value::Float(*u as f64)),
        // An entity or symbol widens to its canonical string form.
        (ValueDataType::String, Value::Entity(e)) => Some(Value::String(e.to_string())),
        (ValueDataType::String, Value::Symbol(a)) => Some(Value::String(a.into())),
        // JSON has no entity or symbol type, so a field declared
        // `as: entity`/`as: symbol` receives its value as a string.
        // Parse it into the real type, failing loudly on a malformed
        // URI or attribute name rather than writing a bogus fact that
        // no query would match.
        (ValueDataType::Entity, Value::String(s)) => {
            return s.clone().try_into().map(Value::Entity).map_err(|_| {
                TransactError::TypeMismatch {
                    field: field.to_string(),
                    expected,
                    found,
                }
            });
        }
        (ValueDataType::Symbol, Value::String(s)) => {
            return Attribute::try_from(s.clone())
                .map(Value::Symbol)
                .map_err(|_| TransactError::TypeMismatch {
                    field: field.to_string(),
                    expected,
                    found,
                });
        }
        _ => None,
    };
    coerced.ok_or(TransactError::TypeMismatch {
        field: field.to_string(),
        expected,
        found,
    })
}

/// `true` if `f` is exactly representable as an `i128`.
fn i128_in_range(f: f64) -> bool {
    f >= -(2f64.powi(127)) && f < 2f64.powi(127)
}

/// `true` if `f` is exactly representable as a `u128`.
fn u128_in_range(f: f64) -> bool {
    f >= 0.0 && f < 2f64.powi(128)
}

/// A concept predicate plus its durability classification.
///
/// `Durable` is the default — facts of the concept carry forward
/// across commits until retracted (the implicit-persistence
/// rule). `Transient` means the facts exist only at the
/// timestep they're submitted in; the reactor's commit pipeline
/// asserts them so effects can read them, then retracts them
/// inside the same transaction so they never reach durable
/// storage.
///
/// The wrapper lives here, on the wire side, rather than as a
/// `transient: bool` field on [`DialogConceptDescriptor`] upstream.
/// Validating the end-to-end mechanism this way means we can
/// keep dialog's descriptor untouched until we're sure of the
/// design.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "concept", rename_all = "lowercase")]
pub enum ConceptDescriptor {
    /// Facts of this concept persist across commits until
    /// retracted.
    Durable(DialogConceptDescriptor),
    /// Facts of this concept exist only at the current
    /// timestep. The reactor strips them before the durable
    /// commit.
    Transient(DialogConceptDescriptor),
}

impl ConceptDescriptor {
    /// Borrow the inner [`DialogConceptDescriptor`], discarding the
    /// durability wrapper.
    pub fn concept(&self) -> &DialogConceptDescriptor {
        match self {
            Self::Durable(c) | Self::Transient(c) => c,
        }
    }

    /// `true` if this descriptor names a transient concept.
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Transient(_))
    }
}

/// A predicate applied to parameter bindings as it arrives on the
/// wire — the **source** form, before type validation.
///
/// Each entry in `parameters` is a concrete [`Value`] decoded
/// straight from JSON, so its runtime type reflects the wire
/// encoding (every JS number is a [`Value::Float`]), not the
/// concept's declared `as:` types. Project it into the validated
/// [`PredicateApplication`] via [`TryFrom`] before emitting facts —
/// that conversion is the only place the declared types meet the
/// supplied values.
///
/// The `"this"` slot, when present, names the subject entity and
/// marks this as an **update**; when absent the worker derives the
/// subject from `(predicate, parameters)` and treats it as a
/// **construction** (which must supply every declared field).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceApplication {
    /// The predicate, with its durability classification.
    pub predicate: ConceptDescriptor,
    /// Value bindings for this application. Omitting `"this"`
    /// asks the worker to derive the subject from the predicate
    /// and the remaining payload.
    #[serde(default)]
    pub parameters: ValueMap,
    /// Published name (`&anchor` in notation), if any. See
    /// [`PredicateApplication::name`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<AnchorName>,
}

impl SourceApplication {
    /// `true` if the predicate names a transient concept.
    pub fn is_transient(&self) -> bool {
        self.predicate.is_transient()
    }
}

/// A predicate application whose parameter values have been
/// validated and coerced against the predicate's declared `as:`
/// types — the claim counterpart of `tonk_schema::query::Query`.
///
/// The fields are private and there is no public constructor other
/// than [`TryFrom<SourceApplication>`]: holding a
/// `PredicateApplication` is a proof that every parameter already
/// conforms to its field's declared type and (for a construction)
/// that no declared field is missing. Downstream fact emission can
/// therefore trust the values without re-checking.
#[derive(Debug, Clone)]
pub struct PredicateApplication {
    predicate: ConceptDescriptor,
    parameters: ValueMap,
    name: Option<AnchorName>,
}

impl PredicateApplication {
    /// `true` if the predicate names a transient concept.
    pub fn is_transient(&self) -> bool {
        self.predicate.is_transient()
    }

    /// The validated predicate and its durability classification.
    pub fn predicate(&self) -> &ConceptDescriptor {
        &self.predicate
    }

    /// The validated, type-coerced parameter bindings.
    pub fn parameters(&self) -> &ValueMap {
        &self.parameters
    }

    /// The published `&anchor` name, if any.
    pub fn name(&self) -> Option<&AnchorName> {
        self.name.as_ref()
    }

    /// Decompose into the validated parts, consuming `self`. Used by
    /// the planner, which needs to move the values out.
    pub fn into_parts(self) -> (ConceptDescriptor, ValueMap, Option<AnchorName>) {
        (self.predicate, self.parameters, self.name)
    }
}

impl TryFrom<SourceApplication> for PredicateApplication {
    type Error = TransactError;

    /// Validate and coerce a wire-form application against its
    /// predicate's declared types.
    ///
    /// Every supplied parameter (other than the reserved `this`)
    /// must name a field the predicate's `with:` map declares
    /// ([`TransactError::UnknownField`] otherwise) and must [`coerce_value`]
    /// to that field's declared type ([`TransactError::TypeMismatch`]
    /// otherwise).
    ///
    /// An assertion may provide a *subset* of the declared fields: a
    /// `with:` map describes the concept's shape, but a caller writes
    /// only the fields it has — an update touches a few, and a
    /// transient command supplies only the event fields that fired
    /// (the rest of its `with:` stay unbound). So a missing field is
    /// not an error; the per-field facts simply aren't emitted for
    /// the fields left out.
    fn try_from(source: SourceApplication) -> Result<Self, Self::Error> {
        let SourceApplication {
            predicate,
            mut parameters,
            name,
        } = source;
        let with = predicate.concept().with();

        // Coerce each supplied field against its declared type, and
        // reject any parameter the concept does not declare. `this`
        // is the subject entity, not a field — it passes through.
        let mut validated = ValueMap::new();
        for (key, value) in parameters.drain(..) {
            if key == "this" {
                validated.insert(key, value);
                continue;
            }
            let Some(attr) = with.iter().find(|(name, _)| *name == key).map(|(_, a)| a) else {
                return Err(TransactError::UnknownField { field: key });
            };
            let coerced = coerce_value(&key, attr.content_type(), value)?;
            validated.insert(key, coerced);
        }

        Ok(Self {
            predicate,
            parameters: validated,
            name,
        })
    }
}

/// One assertion or retraction in a [`TransactRequest`] — the
/// **source** write-unit decoded from the wire, before validation.
#[derive(Debug, Clone)]
pub enum SourceClaim {
    /// Assert the facts produced by this predicate application.
    Assert(SourceApplication),
    /// Retract the facts produced by this predicate
    /// application.
    Retract(SourceApplication),
    /// Invoke a nominal command. The worker resolves and validates it
    /// against authoritative branch data before building a transaction.
    Invoke(SourceInvocation),
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
enum SourceClaimWire {
    Assert {
        application: SourceApplication,
    },
    Retract {
        application: SourceApplication,
    },
    Invoke {
        command: dialog_artifacts::Entity,
        #[serde(default)]
        arguments: ValueMap,
    },
}

impl Serialize for SourceClaim {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Assert(application) => SourceClaimWire::Assert {
                application: application.clone(),
            },
            Self::Retract(application) => SourceClaimWire::Retract {
                application: application.clone(),
            },
            Self::Invoke(invocation) => SourceClaimWire::Invoke {
                command: invocation.command.clone(),
                arguments: invocation.arguments.clone(),
            },
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SourceClaim {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(match SourceClaimWire::deserialize(deserializer)? {
            SourceClaimWire::Assert { application } => Self::Assert(application),
            SourceClaimWire::Retract { application } => Self::Retract(application),
            SourceClaimWire::Invoke { command, arguments } => {
                Self::Invoke(SourceInvocation { command, arguments })
            }
        })
    }
}

impl SourceClaim {
    /// Borrow the structural application, or return `None` for a
    /// nominal invocation.
    pub fn application(&self) -> Option<&SourceApplication> {
        match self {
            Self::Assert(a) | Self::Retract(a) => Some(a),
            Self::Invoke(_) => None,
        }
    }

    /// Borrow the nominal invocation, if this is an `invoke` claim.
    pub fn invocation(&self) -> Option<&SourceInvocation> {
        match self {
            Self::Invoke(invocation) => Some(invocation),
            Self::Assert(_) | Self::Retract(_) => None,
        }
    }
}

/// A validated assertion or retraction — a [`SourceClaim`] whose
/// application has passed the [`PredicateApplication`] type gate.
#[derive(Debug, Clone)]
pub enum Claim {
    /// Assert the facts produced by this predicate application.
    Assert(PredicateApplication),
    /// Retract the facts produced by this predicate
    /// application.
    Retract(PredicateApplication),
}

impl Claim {
    /// Borrow the inner [`PredicateApplication`], regardless of
    /// variant.
    pub fn application(&self) -> &PredicateApplication {
        match self {
            Self::Assert(a) | Self::Retract(a) => a,
        }
    }
}

impl TryFrom<SourceClaim> for Claim {
    type Error = TransactError;

    fn try_from(source: SourceClaim) -> Result<Self, Self::Error> {
        Ok(match source {
            SourceClaim::Assert(a) => Claim::Assert(a.try_into()?),
            SourceClaim::Retract(a) => Claim::Retract(a.try_into()?),
            SourceClaim::Invoke(invocation) => {
                return Err(TransactError::InvocationRequiresResolution {
                    command: invocation.command,
                });
            }
        })
    }
}

/// Body of a `POST /api/repository/{repo}/branch/{branch}/transact`
/// (and profile counterpart) request — a list of [`SourceClaim`]s
/// applied in order under one dialog commit. Each claim is validated
/// into a [`Claim`] (via [`TryFrom`]) before its facts are emitted.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TransactRequest {
    /// In document order. Each claim contributes facts to the
    /// transaction; the reactor buckets transient applications
    /// separately so they can be retracted before the durable
    /// write.
    pub claims: Vec<SourceClaim>,
}

impl TransactRequest {
    /// Reconstruct a request from its canonical DAG-JSON encoding.
    /// Used by the `claim!` macro, which serializes the lowered
    /// request at compile time and embeds the bytes; the generated
    /// code calls this at runtime. The bytes are always produced by
    /// `serde_ipld_dagjson` from this same type, so a decode
    /// failure is a build-time bug, not a user error.
    pub fn from_dagjson_bytes(bytes: &[u8]) -> Self {
        serde_ipld_dagjson::from_slice(bytes)
            .expect("claim!: compiled bootstrap is not valid DAG-JSON")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::SourceInvocation;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    /// Build a durable single-field descriptor with the given `as:`
    /// type. The `as:` token is the serialized [`ValueDataType`] name
    /// (`Text`, `SignedInteger`, `UnsignedInteger`, `Float`, `Entity`,
    /// `Symbol`, …), the same vocabulary the wire and notation use.
    fn one_field(field: &str, as_type: &str) -> ConceptDescriptor {
        let json = format!(
            r#"{{ "kind": "durable", "concept": {{ "with": {{ "{field}": {{ "the": "xyz.tonk.thing/{field}", "as": "{as_type}", "cardinality": "one" }} }} }} }}"#,
        );
        serde_json::from_str(&json).unwrap()
    }

    fn source(predicate: ConceptDescriptor, params: &[(&str, Value)]) -> SourceApplication {
        let mut parameters = ValueMap::new();
        for (k, v) in params {
            parameters.insert((*k).into(), v.clone());
        }
        SourceApplication {
            predicate,
            parameters,
            name: None,
        }
    }

    #[dialog_common::test]
    fn source_claim_invoke_uses_command_specific_shape() {
        let invoke = TransactRequest {
            claims: vec![SourceClaim::Invoke(SourceInvocation {
                command: "id:todo/add".parse().unwrap(),
                arguments: [("title".into(), Value::String("Buy milk".into()))]
                    .into_iter()
                    .collect(),
            })],
        };
        let invoke_json = serde_json::to_string(&invoke).unwrap();
        assert_eq!(
            invoke_json,
            r#"{"claims":[{"op":"invoke","command":"id:todo/add","arguments":{"title":"Buy milk"}}]}"#
        );

        let application = source(
            one_field("title", "Text"),
            &[("title", Value::String("Buy milk".into()))],
        );
        let assert = SourceClaim::Assert(application.clone());
        let retract = SourceClaim::Retract(application);
        let assert_json = serde_json::to_string(&assert).unwrap();
        let retract_json = serde_json::to_string(&retract).unwrap();
        assert_eq!(
            assert_json,
            r#"{"op":"assert","application":{"predicate":{"kind":"durable","concept":{"with":{"title":{"the":"xyz.tonk.thing/title","description":"","cardinality":"one","as":"Text"}}}},"parameters":{"title":"Buy milk"}}}"#
        );
        assert_eq!(
            retract_json,
            r#"{"op":"retract","application":{"predicate":{"kind":"durable","concept":{"with":{"title":{"the":"xyz.tonk.thing/title","description":"","cardinality":"one","as":"Text"}}}},"parameters":{"title":"Buy milk"}}}"#
        );

        let decoded: TransactRequest = serde_json::from_str(&invoke_json).unwrap();
        let decoded = decoded.claims[0].invocation().unwrap();
        assert_eq!(decoded.command.to_string(), "id:todo/add");
        assert_eq!(
            decoded.arguments.get("title"),
            Some(&Value::String("Buy milk".into()))
        );
    }

    #[dialog_common::test]
    fn source_claim_invoke_requires_authoritative_resolution() {
        let source = SourceClaim::Invoke(SourceInvocation {
            command: "id:todo/add".parse().unwrap(),
            arguments: ValueMap::new(),
        });
        assert!(matches!(
            Claim::try_from(source),
            Err(TransactError::InvocationRequiresResolution { command })
                if command.to_string() == "id:todo/add"
        ));
    }

    // -------- coercion: numeric values --------

    #[dialog_common::test]
    fn it_coerces_integral_float_into_signed_integer_field() {
        let d = one_field("n", "SignedInteger");
        let app = PredicateApplication::try_from(source(d, &[("n", Value::Float(27.0))]))
            .expect("integral float coerces");
        assert_eq!(app.parameters().get("n"), Some(&Value::SignedInt(27)));
    }

    #[dialog_common::test]
    fn it_coerces_integral_float_into_unsigned_integer_field() {
        let d = one_field("n", "UnsignedInteger");
        let app = PredicateApplication::try_from(source(d, &[("n", Value::Float(27.0))]))
            .expect("integral non-negative float coerces");
        assert_eq!(app.parameters().get("n"), Some(&Value::UnsignedInt(27)));
    }

    #[dialog_common::test]
    fn it_rejects_fractional_float_into_integer_field() {
        let d = one_field("n", "SignedInteger");
        let err = PredicateApplication::try_from(source(d, &[("n", Value::Float(27.5))]))
            .expect_err("fractional float is lossy");
        assert!(matches!(err, TransactError::TypeMismatch { .. }));
    }

    #[dialog_common::test]
    fn it_rejects_negative_into_unsigned_field() {
        let d = one_field("n", "UnsignedInteger");
        let err = PredicateApplication::try_from(source(d, &[("n", Value::SignedInt(-1))]))
            .expect_err("negative does not fit unsigned");
        assert!(matches!(err, TransactError::TypeMismatch { .. }));
    }

    #[dialog_common::test]
    fn it_coerces_integer_into_float_field() {
        let d = one_field("n", "Float");
        let app = PredicateApplication::try_from(source(d, &[("n", Value::SignedInt(3))]))
            .expect("integer fills a float field exactly");
        assert_eq!(app.parameters().get("n"), Some(&Value::Float(3.0)));
    }

    #[dialog_common::test]
    fn it_rejects_string_into_integer_field() {
        let d = one_field("n", "SignedInteger");
        let err = PredicateApplication::try_from(source(d, &[("n", Value::String("3".into()))]))
            .expect_err("a numeric string is not parsed into a number");
        assert!(matches!(err, TransactError::TypeMismatch { .. }));
    }

    #[dialog_common::test]
    fn it_passes_matching_type_through() {
        let d = one_field("s", "Text");
        let app = PredicateApplication::try_from(source(d, &[("s", Value::String("hi".into()))]))
            .expect("matching type passes");
        assert_eq!(app.parameters().get("s"), Some(&Value::String("hi".into())));
    }

    // -------- coercion: entity / symbol widening and parsing --------

    #[dialog_common::test]
    fn it_parses_string_into_entity_field() {
        let d = one_field("ref", "Entity");
        let uri = "did:key:z6MkParseMe";
        let app = PredicateApplication::try_from(source(d, &[("ref", Value::String(uri.into()))]))
            .expect("a valid URI string parses into an entity");
        let expected: Value = Value::Entity(uri.parse().unwrap());
        assert_eq!(app.parameters().get("ref"), Some(&expected));
    }

    #[dialog_common::test]
    fn it_rejects_malformed_string_for_entity_field() {
        let d = one_field("ref", "Entity");
        let err = PredicateApplication::try_from(source(
            d,
            &[("ref", Value::String("not a uri".into()))],
        ))
        .expect_err("a malformed URI fails loudly rather than writing a bogus entity");
        assert!(matches!(err, TransactError::TypeMismatch { .. }));
    }

    #[dialog_common::test]
    fn it_parses_string_into_symbol_field() {
        let d = one_field("attr", "Symbol");
        let app = PredicateApplication::try_from(source(
            d,
            &[("attr", Value::String("xyz.tonk.thing/field".into()))],
        ))
        .expect("a namespaced name parses into a symbol");
        assert!(matches!(
            app.parameters().get("attr"),
            Some(Value::Symbol(_))
        ));
    }

    #[dialog_common::test]
    fn it_widens_entity_into_text_field() {
        let d = one_field("s", "Text");
        let e: Value = Value::Entity("did:key:z6MkWiden".parse().unwrap());
        let app = PredicateApplication::try_from(source(d, &[("s", e)]))
            .expect("an entity widens to its canonical string");
        assert_eq!(
            app.parameters().get("s"),
            Some(&Value::String("did:key:z6MkWiden".into()))
        );
    }

    // -------- subset of declared fields is allowed --------

    /// An assertion may bind only some of the concept's declared
    /// fields — a transient command supplies just the event fields
    /// that fired, and an update touches just what changed. Missing
    /// fields are not an error; their facts simply aren't emitted.
    #[dialog_common::test]
    fn it_allows_a_subset_of_declared_fields() {
        let json = r#"{ "kind": "durable", "concept": { "with": {
            "a": { "the": "xyz.tonk.thing/a", "as": "Text", "cardinality": "one" },
            "b": { "the": "xyz.tonk.thing/b", "as": "Text", "cardinality": "one" }
        } } }"#;
        let d: ConceptDescriptor = serde_json::from_str(json).unwrap();
        // Construction (no `this`) binding only `a` — `b` is left out.
        let app = PredicateApplication::try_from(source(d, &[("a", Value::String("x".into()))]))
            .expect("an assertion may bind a subset of the declared fields");
        assert!(app.parameters().get("a").is_some());
        assert!(app.parameters().get("b").is_none());
    }

    #[dialog_common::test]
    fn it_rejects_unknown_field() {
        let d = one_field("a", "Text");
        let this: Value = Value::Entity("did:key:z6MkUnknown".parse().unwrap());
        let err = PredicateApplication::try_from(source(
            d,
            &[("this", this), ("z", Value::String("x".into()))],
        ))
        .expect_err("a parameter the concept does not declare is rejected");
        assert!(
            matches!(err, TransactError::UnknownField { ref field } if field == "z"),
            "got {err:?}"
        );
    }

    /// End-to-end of the reported bug: a UI-created bug whose
    /// `ordering` arrives as a JSON number (a float) into a
    /// `SignedInteger` field is coerced to an integer, so the
    /// strictly-typed concept query can see it again.
    #[dialog_common::test]
    fn it_coerces_ui_float_ordering_to_integer() {
        let d = one_field("ordering", "SignedInteger");
        let this: Value = Value::Entity("did:key:z6Mk5kFgreYpnVphq2HcMQ65".parse().unwrap());
        let app = PredicateApplication::try_from(source(
            d,
            &[("this", this), ("ordering", Value::Float(27.0))],
        ))
        .expect("the UI float ordering coerces to an integer fact");
        assert_eq!(
            app.parameters().get("ordering"),
            Some(&Value::SignedInt(27))
        );
    }
}
