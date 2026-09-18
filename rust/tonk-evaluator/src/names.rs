//! Published names for the entities a result set mentions.
//!
//! An entity URI is unambiguous and unreadable: `did:key:z6MkfpAV…`
//! says nothing about what it is. A branch usually knows better — an
//! `&anchor` in notation publishes a [`Name`] claim, an `id:<n>` entity
//! whose `db.name/referent` points at the anchored entity — so most of
//! the entities a query returns already have a human name sitting one
//! hop away.
//!
//! This module reads that mapping in the *reverse* direction (entity →
//! name, rather than [`resolution::NamedReference`]'s name → entity) for
//! exactly the entities a set of [`QueryMatchBlock`]s mentions, so a
//! renderer can substitute the name and keep the URI as the reveal.
//!
//! The map ships on the `/evaluate` response rather than being resolved
//! per-renderer: the inspector deliberately links no query engine, so
//! the only place that *can* run the lookup is the side that already ran
//! the queries.

use std::collections::{BTreeMap, BTreeSet};

use tonk_schema::query_source::Source;
use tonk_schema::resolution::NamedReference;

use crate::evaluate::{EvaluateEnv, EvaluateError, QueryMatchBlock};

/// Entity URI → the published name that currently points at it.
///
/// Only entities a caller asked about appear; an entity nothing names
/// is simply absent, and a renderer falls back to the URI.
pub type Names = BTreeMap<String, String>;

/// The longest string worth testing as an entity URI. Entity URIs are
/// short; anything longer is prose that happens to carry a colon, and
/// collecting it would only grow the candidate set.
const MAX_URI_LEN: usize = 256;

/// How deep to descend into a field value looking for entity URIs. A
/// concept descriptor nests a few levels; beyond that a value is data,
/// not references.
const MAX_DEPTH: usize = 8;

/// Resolve the published name of every entity `blocks` mentions.
///
/// One query reads the branch's whole name table (names are schema-
/// scale, and one scan beats one round trip per distinct entity in a
/// result set that may hold hundreds); the result is then filtered to
/// the entities actually mentioned, so the map that ships is bounded by
/// the response rather than by the branch.
///
/// A resolution failure is not fatal to the caller's response — names
/// are an affordance, not data — but it is returned rather than
/// swallowed so the caller decides.
pub async fn resolve<'a, Env: EvaluateEnv>(
    source: impl Into<Source<'a>>,
    blocks: &[&[QueryMatchBlock]],
    env: &Env,
) -> Result<Names, EvaluateError> {
    let mut mentioned = BTreeSet::new();
    for group in blocks {
        collect_blocks(group, &mut mentioned);
    }
    if mentioned.is_empty() {
        return Ok(Names::new());
    }

    let published = NamedReference::list(source)
        .perform(env)
        .await
        .map_err(|error| EvaluateError::Query(format!("name lookup failed: {error}")))?;

    let mut names = Names::new();
    for claim in published {
        let target = claim.entity.0.to_string();
        if !mentioned.contains(&target) {
            continue;
        }
        let Some(name) = published_name(&claim.this.to_string()) else {
            continue;
        };
        // Several names may point at one entity. Pick deterministically:
        // the shortest, ties broken lexicographically — so a rendering
        // does not flicker between names across evaluates.
        match names.get(&target) {
            Some(existing) if better(existing, &name) => {}
            _ => {
                names.insert(target, name);
            }
        }
    }
    Ok(names)
}

/// The bare name a name entity publishes — `id:alice` → `alice`,
/// `db:attribute` → `attribute`.
///
/// `None` for anything else: substituting one opaque URI for another
/// buys the reader nothing, so such a binding is left unrendered.
fn published_name(name_entity: &str) -> Option<String> {
    let bare = name_entity
        .strip_prefix("id:")
        .or_else(|| name_entity.strip_prefix("db:"))?;
    (!bare.is_empty()).then(|| bare.to_owned())
}

/// Whether `existing` wins over `candidate` — shorter first, then
/// lexicographic.
fn better(existing: &str, candidate: &str) -> bool {
    (existing.chars().count(), existing) <= (candidate.chars().count(), candidate)
}

/// Every entity URI the blocks mention: each result's `this`, plus any
/// field value that reads as a URI.
fn collect_blocks(blocks: &[QueryMatchBlock], into: &mut BTreeSet<String>) {
    for block in blocks {
        for result in &block.results {
            if looks_like_uri(&result.this) {
                into.insert(result.this.clone());
            }
            for value in result.fields.values() {
                collect_value(value, 0, into);
            }
        }
    }
}

/// Descend a field value collecting URI-shaped strings.
///
/// A string that is itself serialized JSON (a concept's `source`, which
/// the renderer expands into the `concept!:` body) is parsed and
/// descended too — that body is exactly where attribute and concept
/// entities show up unreadably.
fn collect_value(value: &serde_json::Value, depth: usize, into: &mut BTreeSet<String>) {
    if depth > MAX_DEPTH {
        return;
    }
    match value {
        serde_json::Value::String(text) => {
            // Serialized JSON first: a compact descriptor carries no
            // whitespace and plenty of colons, so it reads as a URI to
            // the heuristic below and would swallow its own contents.
            let trimmed = text.trim();
            let nested = (trimmed.starts_with('{') && trimmed.ends_with('}'))
                || (trimmed.starts_with('[') && trimmed.ends_with(']'));
            if nested {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed) {
                    collect_value(&parsed, depth + 1, into);
                }
                return;
            }
            if looks_like_uri(text) {
                into.insert(text.clone());
            }
        }
        serde_json::Value::Object(map) => {
            for child in map.values() {
                collect_value(child, depth + 1, into);
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                collect_value(child, depth + 1, into);
            }
        }
        _ => {}
    }
}

/// True if `text` reads as an entity URI — scheme-prefixed, no
/// whitespace, short enough to be an identifier.
///
/// Deliberately loose: a false positive costs one lookup against a map
/// that will not contain it, while a false negative loses a name. The
/// same shape decides the `tonk-cm-entity` tint in the inspector's
/// renderer, so producer and consumer agree on what counts.
fn looks_like_uri(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_URI_LEN
        && !text.chars().any(char::is_whitespace)
        && text.contains(':')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluate::QueryResult;

    fn block(label: &str, results: Vec<QueryResult>) -> QueryMatchBlock {
        QueryMatchBlock {
            label: label.to_owned(),
            results,
        }
    }

    fn result(this: &str, fields: &[(&str, serde_json::Value)]) -> QueryResult {
        QueryResult {
            this: this.to_owned(),
            fields: fields
                .iter()
                .map(|(name, value)| ((*name).to_owned(), value.clone()))
                .collect(),
        }
    }

    #[test]
    fn it_collects_this_and_entity_valued_fields() {
        let blocks = vec![block(
            "person",
            vec![result(
                "did:key:z6Mkalice",
                &[
                    ("name", serde_json::json!("Alice")),
                    ("employer", serde_json::json!("did:key:z6Mkacme")),
                ],
            )],
        )];
        let mut found = BTreeSet::new();
        collect_blocks(&blocks, &mut found);
        assert!(found.contains("did:key:z6Mkalice"));
        assert!(found.contains("did:key:z6Mkacme"));
        assert!(!found.contains("Alice"), "plain text is not a URI");
    }

    #[test]
    fn it_descends_a_stringified_descriptor() {
        let source =
            serde_json::json!({"with": {"name": {"the": "concept:z6Mkthing"}}}).to_string();
        let blocks = vec![block(
            "concept",
            vec![result(
                "concept:z6Mkouter",
                &[("source", serde_json::Value::String(source))],
            )],
        )];
        let mut found = BTreeSet::new();
        collect_blocks(&blocks, &mut found);
        assert!(
            found.contains("concept:z6Mkthing"),
            "a stringified descriptor's entities are reachable: {found:?}"
        );
    }

    #[test]
    fn it_reads_the_bare_name_out_of_a_name_entity() {
        assert_eq!(published_name("id:alice").as_deref(), Some("alice"));
        assert_eq!(published_name("db:attribute").as_deref(), Some("attribute"));
        assert_eq!(published_name("did:key:z6Mk").as_deref(), None);
        assert_eq!(published_name("id:").as_deref(), None);
    }

    #[test]
    fn it_prefers_the_shortest_name() {
        assert!(better("bob", "roberto"), "shorter wins");
        assert!(
            better("alice", "bobbie"),
            "shorter wins regardless of order"
        );
        assert!(!better("roberto", "bob"), "longer loses");
        assert!(
            better("aaa", "bbb"),
            "equal length breaks lexicographically"
        );
    }
}
