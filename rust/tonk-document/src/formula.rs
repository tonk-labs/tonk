//! History reads, as query formulas.
//!
//! Reading a document's past takes parameters — which version, which
//! two versions — so it cannot be a mirror fact. It is a query formula
//! instead: a query whose `predicate` is a string names one, and the
//! rows come back in the same [`Conclusion`] shape every query returns.
//! The worker resolves formulas from its `/query` route and the CLI from
//! its query path, so both read history the same way.
//!
//! | formula | terms | rows |
//! |---|---|---|
//! | `document/versions` | `document`, `limit?` | `{ heads, author, issuer, revision }`, newest first |
//! | `document/changes` | `document`, `since?`, `limit?` | `{ change, author, time, parents }` |
//! | `document/content` | `document`, `heads?` | text: `{ text, heads }`; table: `{ sheet, name, at, content, style }` |
//! | `document/diff` | `document`, `from`, `to` | `{ op, at, text, length }` or `{ op, path, value }` |
//!
//! Heads travel as hex change hashes separated by single spaces.
//! Positions are UTF-16 code units at the `from` version. Nothing here
//! is an automerge shape, so a page that reads history keeps working
//! when the app moves to a new automerge.

use std::collections::BTreeMap;

use dialog_artifacts::{Entity, Value};
use dialog_query::Term;
use dialog_reactor::{Conclusion, Query};
use dialog_repository::Branch;
use futures_util::StreamExt as _;
use ipld_core::ipld::Ipld;
use thiserror::Error;

use crate::engine::{Content, DiffOp};
use crate::session::{self, DocumentEnv, HEADS, SessionError};

/// Default and ceiling for `limit`.
const DEFAULT_LIMIT: usize = 100;

/// Failures resolving a document formula.
#[derive(Debug, Error)]
pub enum FormulaError {
    /// A required term is missing or has the wrong type.
    #[error("bad input for {formula}: {reason}")]
    BadInput {
        /// The formula being resolved.
        formula: String,
        /// Why the input was rejected.
        reason: String,
    },
    /// The document could not be read.
    #[error(transparent)]
    Session(#[from] SessionError),
}

/// Whether `name` is a formula this module resolves.
pub fn handles(name: &str) -> bool {
    matches!(
        name,
        "document/versions" | "document/changes" | "document/content" | "document/diff"
    )
}

fn bad(formula: &str, reason: impl Into<String>) -> FormulaError {
    FormulaError::BadInput {
        formula: formula.to_string(),
        reason: reason.into(),
    }
}

fn text_term(query: &Query, name: &str) -> Option<String> {
    match query.terms.get(name) {
        Some(Term::Constant(Value::String(text))) => Some(text.clone()),
        Some(Term::Constant(Value::Entity(entity))) => Some(entity.to_string()),
        _ => None,
    }
}

fn heads_term(query: &Query, name: &str) -> Option<Vec<String>> {
    text_term(query, name).map(|text| parse_heads(&text))
}

/// Heads as they travel in terms and command fields.
pub fn parse_heads(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_string).collect()
}

/// The inverse of [`parse_heads`].
pub fn format_heads(heads: &[String]) -> String {
    heads.join(" ")
}

fn limit_term(query: &Query) -> usize {
    match query.terms.get("limit") {
        Some(Term::Constant(Value::UnsignedInt(n))) => (*n as usize).clamp(1, 10 * DEFAULT_LIMIT),
        Some(Term::Constant(Value::String(text))) => text
            .parse::<usize>()
            .map_or(DEFAULT_LIMIT, |n| n.clamp(1, 10 * DEFAULT_LIMIT)),
        _ => DEFAULT_LIMIT,
    }
}

fn row(this: impl Into<String>, fields: impl IntoIterator<Item = (&'static str, Ipld)>) -> Conclusion {
    Conclusion {
        this: this.into(),
        fields: fields
            .into_iter()
            .map(|(name, value)| (name.to_string(), value))
            .collect::<BTreeMap<_, _>>(),
    }
}

fn text(value: impl Into<String>) -> Ipld {
    Ipld::String(value.into())
}

/// Resolve a `document/*` formula. Call only when [`handles`] is true.
pub async fn resolve<Env: DocumentEnv>(
    branch: &Branch,
    env: &Env,
    query: &Query,
) -> Result<Vec<Conclusion>, FormulaError> {
    let name = query.formula().unwrap_or_default().to_string();
    let document = text_term(query, "document")
        .ok_or_else(|| bad(&name, "`document` must name the document entity"))?;
    let entity: Entity = document
        .parse()
        .map_err(|_| bad(&name, format!("`{document}` is not an entity")))?;

    match name.as_str() {
        "document/content" => {
            let (mut opened, format) = session::open(branch, &entity, None, env).await?;
            let heads = match heads_term(query, "heads") {
                Some(heads) => heads,
                None => session::read(branch, &entity, None, env).await?.heads,
            };
            let _ = format;
            let joined = format_heads(&heads);
            Ok(match opened.content(&heads).map_err(SessionError::from)? {
                Content::Text(body) => vec![row(
                    document,
                    [("text", text(body)), ("heads", text(joined))],
                )],
                Content::Table(table) => {
                    let mut rows = Vec::new();
                    for sheet in table.sheets {
                        for (at, content) in &sheet.cells {
                            rows.push(row(
                                format!("{document}#{}/{at}", sheet.id),
                                [
                                    ("sheet", text(sheet.id.clone())),
                                    ("name", text(sheet.name.clone())),
                                    ("at", text(at.clone())),
                                    ("content", text(content.clone())),
                                    ("style", text(sheet.styles.get(at).cloned().unwrap_or_default())),
                                    ("heads", text(joined.clone())),
                                ],
                            ));
                        }
                    }
                    rows
                }
            })
        }

        "document/changes" => {
            let (mut opened, _) = session::open(branch, &entity, None, env).await?;
            let since = heads_term(query, "since").unwrap_or_default();
            let limit = limit_term(query);
            let mut changes = opened.changes(&since).map_err(SessionError::from)?;
            // Newest first, like `versions`.
            changes.reverse();
            Ok(changes
                .into_iter()
                .take(limit)
                .map(|change| {
                    row(
                        format!("change:{}", change.change),
                        [
                            ("change", text(change.change.clone())),
                            ("author", text(change.author.unwrap_or_default())),
                            ("time", Ipld::Integer(change.time.into())),
                            ("parents", text(format_heads(&change.parents))),
                        ],
                    )
                })
                .collect())
        }

        "document/diff" => {
            let from = heads_term(query, "from").ok_or_else(|| bad(&name, "`from` must be heads"))?;
            let to = heads_term(query, "to").ok_or_else(|| bad(&name, "`to` must be heads"))?;
            let (mut opened, _) = session::open(branch, &entity, None, env).await?;
            let ops = opened.diff(&from, &to).map_err(SessionError::from)?;
            Ok(ops
                .into_iter()
                .enumerate()
                .map(|(index, op)| {
                    let this = format!("{document}#diff/{index}");
                    match op {
                        DiffOp::Insert { at, text: inserted } => row(
                            this,
                            [("op", text("insert")), ("at", Ipld::Integer(at as i128)), ("text", text(inserted))],
                        ),
                        DiffOp::Delete { at, length } => row(
                            this,
                            [
                                ("op", text("delete")),
                                ("at", Ipld::Integer(at as i128)),
                                ("length", Ipld::Integer(length as i128)),
                            ],
                        ),
                        DiffOp::Put { path, value } => row(
                            this,
                            [("op", text("put")), ("path", text(path)), ("value", text(plain(&value)))],
                        ),
                        DiffOp::Remove { path } => row(this, [("op", text("remove")), ("path", text(path))]),
                    }
                })
                .collect())
        }

        "document/versions" => versions(branch, &entity, &document, limit_term(query), env).await,

        other => Err(bad(other, "not a document formula")),
    }
}

fn plain(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// One entry per branch revision that asserted a `document/heads`
/// value for `entity`, newest first. The author is the revision's own
/// signed attribution, which automerge changes cannot give.
///
/// History may not be local: a replica that downloaded only a branch's
/// operational regions reads it from the branch's remote, so this list
/// can need the network and can be incomplete offline.
async fn versions<Env: DocumentEnv>(
    branch: &Branch,
    entity: &Entity,
    document: &str,
    limit: usize,
    env: &Env,
) -> Result<Vec<Conclusion>, FormulaError> {
    let failed = |error: &dyn std::fmt::Display| FormulaError::Session(SessionError::Branch(error.to_string()));
    let heads_attribute = HEADS;
    // Revisions that did not touch the document are skipped, so scan a
    // generous window of the log for `limit` hits.
    let log = branch
        .log(env, limit.saturating_mul(20))
        .await
        .map_err(|error| failed(&error))?;
    let history = branch.history(env).await;
    let mut rows = Vec::new();
    for (version, record) in log {
        if rows.len() >= limit {
            break;
        }
        let mut asserted = Vec::new();
        {
            let stream = history.select(version);
            futures_util::pin_mut!(stream);
            while let Some(next) = stream.next().await {
                let (_, entry) = next.map_err(|error| failed(&error))?;
                if let dialog_artifacts::history::Record::Assert(claim) = entry {
                    if &claim.of == entity && claim.the.to_string() == heads_attribute {
                        if let Ok(head) = String::try_from(claim.is) {
                            asserted.push(head);
                        }
                    }
                }
            }
        }
        if asserted.is_empty() {
            continue;
        }
        asserted.sort();
        rows.push(row(
            format!("{document}#version/{}", rows.len()),
            [
                ("heads", text(format_heads(&asserted))),
                ("author", text(record.authority.clone())),
                ("issuer", text(record.issuer.clone())),
                ("revision", text(format!("{version:?}"))),
            ],
        ));
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Edit, Format, Stamp};
    use dialog_operator::helpers::{test_operator_with_profile, test_repo};
    use tonk_schema_query::formula_query;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    /// Build a formula [`Query`] the way the wire does.
    mod tonk_schema_query {
        use dialog_reactor::Query;

        pub(super) fn formula_query(name: &str, terms: serde_json::Value) -> Query {
            serde_json::from_value(serde_json::json!({ "predicate": name, "terms": terms }))
                .expect("a formula query")
        }
    }

    fn field(conclusion: &Conclusion, name: &str) -> String {
        match conclusion.fields.get(name) {
            Some(Ipld::String(text)) => text.clone(),
            Some(Ipld::Integer(n)) => n.to_string(),
            other => panic!("no text field {name}: {other:?}"),
        }
    }

    #[dialog_common::test]
    async fn it_reads_versions_content_diff_and_changes() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;
        let doc: Entity = "id:prose/doc".parse().unwrap();
        let stamp = Stamp { author: Some("did:key:zAuthor".into()), time: 7 };

        let v1 = session::write(&branch, &doc, Some(Format::Text), None, &[Edit::SetText { text: "one".into() }], &stamp, &operator).await?;
        let v2 = session::write(&branch, &doc, None, None, &[Edit::SetText { text: "one two".into() }], &stamp, &operator).await?;

        let versions = resolve(&branch, &operator, &formula_query("document/versions", serde_json::json!({ "document": "id:prose/doc" }))).await?;
        assert_eq!(versions.len(), 2, "one version per save");
        assert_eq!(field(&versions[0], "heads"), format_heads(&v2.snapshot.heads), "newest first");
        assert_eq!(field(&versions[1], "heads"), format_heads(&v1.snapshot.heads));
        assert!(field(&versions[0], "author").starts_with("did:"), "the revision's signed attribution");

        let old = resolve(&branch, &operator, &formula_query("document/content", serde_json::json!({ "document": "id:prose/doc", "heads": format_heads(&v1.snapshot.heads) }))).await?;
        assert_eq!(field(&old[0], "text"), "one", "content at old heads is the text saved then");
        let now = resolve(&branch, &operator, &formula_query("document/content", serde_json::json!({ "document": "id:prose/doc" }))).await?;
        assert_eq!(field(&now[0], "text"), "one two");

        let diff = resolve(&branch, &operator, &formula_query("document/diff", serde_json::json!({ "document": "id:prose/doc", "from": format_heads(&v1.snapshot.heads), "to": format_heads(&v2.snapshot.heads) }))).await?;
        assert_eq!(diff.len(), 1);
        assert_eq!(field(&diff[0], "op"), "insert");
        assert_eq!(field(&diff[0], "at"), "3");
        assert_eq!(field(&diff[0], "text"), " two");

        let changes = resolve(&branch, &operator, &formula_query("document/changes", serde_json::json!({ "document": "id:prose/doc", "since": format_heads(&v1.snapshot.heads) }))).await?;
        assert_eq!(changes.len(), 1);
        assert_eq!(field(&changes[0], "author"), "did:key:zAuthor");
        assert_eq!(field(&changes[0], "time"), "7");
        Ok(())
    }

    #[dialog_common::test]
    async fn it_refuses_a_formula_without_its_document() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;
        let result = resolve(&branch, &operator, &formula_query("document/content", serde_json::json!({}))).await;
        assert!(matches!(result, Err(FormulaError::BadInput { .. })));
        assert!(handles("document/diff") && !handles("tree/node"));
        Ok(())
    }
}
