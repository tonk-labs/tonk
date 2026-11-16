//! Named query steps: a query whose predicate is a name rather than a
//! concept.
//!
//! A `/query` body whose `predicate` is a bare string names one of
//! dialog's own steps, and its `terms` are that step's `where`:
//!
//! - a **resolver** (`tree/node`, `tree/span`, `tree/entry`,
//!   `tree/block`, `tree/value`), which reads a search-tree node by its
//!   `node:` handle, or a value by its `asset:` one, through the branch;
//! - a **formula** (`dialog/key-part`, `dialog/separator-part`,
//!   `text/length`, ...), computed from the constants the terms give.
//!
//! Nothing here knows what a step means: the name and the terms are
//! handed to dialog as the premise `{ assert: <name>, where: <terms> }`,
//! the same notation a rule's body uses, and each row comes back as a
//! [`Conclusion`] whose fields are the terms that were variables, under
//! the variables' names. A page can so ask one step directly; a join of
//! several is a rule's work.

use std::collections::BTreeMap;

use dialog_corpus::{DialogCorpusError, Value};
use dialog_query::selection::Match;
use dialog_query::{FormulaQuery, ResolverQuery};
use dialog_repository::Branch;
use futures_util::TryStreamExt as _;
use ipld_core::ipld::Ipld;
use thiserror::Error;

use crate::{Conclusion, Query, SelectProvider};

/// Failure modes for [`resolve_formula`].
#[derive(Debug, Error)]
pub enum FormulaError {
    /// The query named no formula (a concept query reached here — a
    /// router bug).
    #[error("not a formula query")]
    NotFormula,

    /// No formula is registered under this name.
    #[error("unknown formula: {0}")]
    Unknown(String),

    /// A required input term was missing or not a node-hash string.
    #[error("bad input for {formula}: {reason}")]
    BadInput {
        /// The formula that was being resolved.
        formula: String,
        /// Why the input was rejected.
        reason: String,
    },

    /// Reading a node block from the archive failed.
    #[error("archive read failed: {0}")]
    Read(String),

    /// Decoding a node block failed.
    #[error("node decode failed: {0}")]
    Decode(String),
}

impl From<DialogCorpusError> for FormulaError {
    fn from(error: DialogCorpusError) -> Self {
        Self::Decode(error.to_string())
    }
}

/// Answer the named step `query` asks, against `branch`.
pub async fn resolve_formula<Env: SelectProvider>(
    branch: &Branch,
    env: &Env,
    query: &Query,
) -> Result<Vec<Conclusion>, FormulaError> {
    let name = query.formula().ok_or(FormulaError::NotFormula)?;
    let bad = |reason: String| FormulaError::BadInput {
        formula: name.into(),
        reason,
    };
    let terms = serde_json::to_value(&query.terms).map_err(|error| bad(error.to_string()))?;
    let wanted = variables(&terms);
    let this = subject(&terms);
    let premise = serde_json::json!({ "assert": name, "where": terms });

    // The name decides the kind. A name both refuse is unknown; a name
    // one knows with terms it refuses is that step's bad input.
    let resolver = serde_json::from_value::<ResolverQuery>(premise.clone());
    let formula = serde_json::from_value::<FormulaQuery>(premise);
    let rows: Vec<BTreeMap<String, Value>> = match (resolver, formula) {
        (Ok(resolver), _) => branch
            .query()
            .select(resolver)
            .perform(env)
            .try_collect()
            .await
            .map_err(|error| FormulaError::Read(error.to_string()))?,
        (_, Ok(formula)) => formula
            .expand(Match::new())
            .map_err(|error| bad(error.to_string()))?
            .iter()
            .map(|row| {
                wanted
                    .iter()
                    .filter_map(|name| Some((name.clone(), row.value_of(name)?.clone())))
                    .collect()
            })
            .collect(),
        (Err(resolver), Err(formula)) => {
            return Err(if known(name) {
                bad(format!("{resolver}; {formula}"))
            } else {
                FormulaError::Unknown(name.into())
            });
        }
    };

    Ok(rows
        .into_iter()
        .map(|row| Conclusion {
            this: this.clone(),
            fields: row
                .iter()
                .filter(|(name, _)| wanted.contains(*name))
                .filter_map(|(name, value)| Some((name.clone(), value_to_ipld(value)?)))
                .collect(),
        })
        .collect())
}

/// Whether dialog has a resolver or a formula called `name`: whether
/// either, given the name and no terms, objects to something other than
/// the name.
fn known(name: &str) -> bool {
    let bare = serde_json::json!({ "assert": name, "where": {} });
    let named = |error: Option<serde_json::Error>| {
        error.is_none_or(|error| !error.to_string().contains("unknown variant"))
    };
    named(serde_json::from_value::<ResolverQuery>(bare.clone()).err())
        || named(serde_json::from_value::<FormulaQuery>(bare).err())
}

/// The names of the variables among `terms`: what a row is asked for.
fn variables(terms: &serde_json::Value) -> Vec<String> {
    let Some(terms) = terms.as_object() else {
        return Vec::new();
    };
    terms
        .values()
        .filter_map(|term| term.get("?")?.get("name")?.as_str())
        .map(str::to_owned)
        .collect()
}

/// What the step was asked `of`, when that is a constant text or
/// entity: the row's `this`, so a row says what it is about.
fn subject(terms: &serde_json::Value) -> String {
    terms
        .get("of")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// Convert a [`Value`] to [`Ipld`] for the wire. Mirrors
/// `tonk_core::conclusion`'s handling: `u128` is special-cased since
/// `ipld_core`'s serde path rejects it.
fn value_to_ipld(value: &Value) -> Option<Ipld> {
    Some(match value {
        Value::Bytes(b) => Ipld::Bytes(b.clone()),
        Value::Entity(e) => Ipld::String(e.to_string()),
        Value::Boolean(b) => Ipld::Bool(*b),
        Value::String(s) => Ipld::String(s.clone()),
        Value::Symbol(s) => Ipld::String(s.to_string()),
        Value::UnsignedInt(u) => match i128::try_from(*u) {
            Ok(i) => Ipld::Integer(i),
            Err(_) => Ipld::String(u.to_string()),
        },
        Value::SignedInt(i) => Ipld::Integer(*i),
        Value::Float(f) => Ipld::Float(*f),
        Value::Record(b) => Ipld::Bytes(b.clone()),
    })
}
