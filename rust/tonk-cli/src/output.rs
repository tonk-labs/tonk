//! Render an [`EvaluateResponse`] in either notation (default)
//! or JSON form.
//!
//! Notation output is the default surface — every successful
//! call produces a re-submittable asserted-notation document
//! with the envelope (revisions, claim count, entity bindings)
//! emitted as a YAML mapping prefix and the matches as
//! per-expression query expressions, separated by `---` YAML
//! document markers.
//!
//! [`EvaluateResponse`] is tonk's own copy of the JSON wire
//! shape the worker's `/evaluate` route returns. The two are
//! byte-compatible: tonk's `--json` output is the same JSON a
//! browser client would see from the HTTP route. Defined here
//! rather than imported so tonk doesn't depend on the worker
//! crate.

use std::fmt::Write as _;

use anyhow::{Context, Result};
use dialog_repository::Revision;
use serde::{Deserialize, Serialize};
use tonk_evaluator::evaluate::{CommitSummary, QueryMatchBlock, QueryResult};

/// JSON wire shape returned by both tonk and the worker's
/// `/evaluate` route. Tonk owns its own copy so the JSON
/// contract stays where it's consumed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvaluateResponse {
    /// Revision of the branch before the commit, if any.
    pub revision_before: Option<Revision>,
    /// Revision of the branch after the commit. Equal to
    /// `revision_before` when nothing committed.
    pub revision_after: Option<Revision>,
    /// Per-source-expression query matches as they looked
    /// *before* the commit.
    pub matches_before: Vec<QueryMatchBlock>,
    /// Per-source-expression query matches as they look *after*
    /// the commit.
    pub matches_after: Vec<QueryMatchBlock>,
    /// Commit summary — number of EAV claims plus entities the
    /// document touched.
    pub commits: CommitSummary,
}

/// Output format selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Re-submittable asserted-notation. Envelope as a YAML
    /// mapping document, then `---`, then per-expression query
    /// expressions.
    Notation,
    /// `EvaluateResponse` serialized as pretty JSON.
    Json,
}

/// Render `response` into stdout-ready bytes. `quiet` suppresses
/// the matches section, leaving only the envelope (notation) or
/// the structured commits-only response (JSON).
pub fn render(response: &EvaluateResponse, format: Format, quiet: bool) -> Result<String> {
    match format {
        Format::Notation => render_notation(response, quiet),
        Format::Json => render_json(response, quiet),
    }
}

// ---------------------------------------------------------------- //
// Notation renderer                                                //
// ---------------------------------------------------------------- //

fn render_notation(response: &EvaluateResponse, quiet: bool) -> Result<String> {
    let envelope = render_envelope(response).context("failed to render envelope")?;

    if quiet {
        return Ok(envelope);
    }

    let matches = render_matches(&response.matches_after);
    if matches.is_empty() {
        return Ok(envelope);
    }

    let mut out = envelope;
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("---\n");
    out.push_str(&matches);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

/// YAML envelope: `revision-before`, `revision-after`, `claims`,
/// `entities`. Built directly as a [`serde_yaml::Mapping`] so
/// keys land in declaration order rather than serde_yaml's
/// alphabetical default.
fn render_envelope(response: &EvaluateResponse) -> Result<String> {
    use serde_yaml::Value;

    let mut envelope = serde_yaml::Mapping::new();

    envelope.insert(
        "revision-before".into(),
        revision_to_yaml(response.revision_before.as_ref()),
    );
    envelope.insert(
        "revision-after".into(),
        revision_to_yaml(response.revision_after.as_ref()),
    );
    envelope.insert("claims".into(), Value::from(response.commits.claims));

    if !response.commits.entities.is_empty() {
        let mut entities = serde_yaml::Mapping::new();
        for (key, did) in &response.commits.entities {
            entities.insert(Value::from(key.clone()), Value::from(did.clone()));
        }
        envelope.insert("entities".into(), Value::Mapping(entities));
    }

    serde_yaml::to_string(&Value::Mapping(envelope)).context("envelope YAML serialization failed")
}

/// `Some(rev)` → its display form (a `#<base58>` tree-hash).
/// `None` → YAML `null`, which serializes as `~` and is the
/// signal for a freshly-initialized branch.
fn revision_to_yaml(revision: Option<&dialog_repository::Revision>) -> serde_yaml::Value {
    match revision {
        Some(rev) => serde_yaml::Value::from(rev.tree.to_string()),
        None => serde_yaml::Value::Null,
    }
}

/// Render every non-empty [`QueryMatchBlock`] as a stack of
/// notation expressions, joined with `---` document markers.
/// Empty blocks (zero results for a source expression) are
/// skipped — emitting an empty section would land an unparseable
/// document in front of the agent.
fn render_matches(blocks: &[QueryMatchBlock]) -> String {
    let mut sections: Vec<String> = Vec::new();
    for block in blocks {
        if block.results.is_empty() {
            continue;
        }
        let mut section = String::new();
        for result in &block.results {
            render_one(&mut section, &block.label, result);
        }
        sections.push(section.trim_end_matches('\n').to_string());
    }
    sections.join("\n---\n")
}

/// One match — `<label>:` head followed by a body whose first
/// entry is `this: <entity>` and the rest is `<field>: <value>`
/// pairs. Matches the head/body grammar so the output is a
/// re-submittable notation document.
fn render_one(out: &mut String, label: &str, result: &QueryResult) {
    let _ = writeln!(out, "{label}:");
    let _ = writeln!(out, "  this: {this}", this = result.this);
    for (field, value) in &result.fields {
        let _ = writeln!(out, "  {field}: {rendered}", rendered = render_value(value));
    }
}

/// Render a JSON-shaped attribute value as an asserted-notation
/// scalar literal.
///
/// Strings are always double-quoted so the output round-trips
/// unambiguously through saphyr — bare strings work too in most
/// cases, but a value like `"true"` would otherwise reparse as a
/// boolean.
fn render_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "~".into(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        // A signed integer rides the wire as an explicitly signed
        // string (JSON numbers can't tell +41 from 41); it prints
        // bare, as the author spells it.
        serde_json::Value::String(s) if is_signed_literal(s) => s.clone(),
        serde_json::Value::String(s) => quote_string(s),
        // Arrays / objects shouldn't normally appear here —
        // dialog attribute values are scalars — but fall back to
        // JSON if they do, so the output is at least lossless.
        other => other.to_string(),
    }
}

/// `+41` / `-7`: an explicitly signed integer literal — the wire
/// spelling of a SignedInteger value.
fn is_signed_literal(s: &str) -> bool {
    match s.strip_prefix(['+', '-']) {
        Some(rest) => !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()),
        None => false,
    }
}

fn quote_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// Render a pure data read without transaction-envelope noise.
///
/// Notation returns re-submittable instances plus an explicit count. JSON is
/// a bare array with `this` alongside the concept fields.
pub fn render_results(response: &EvaluateResponse, format: Format, label: &str) -> Result<String> {
    let results: Vec<&QueryResult> = response
        .matches_after
        .iter()
        .filter(|block| block.label == label)
        .flat_map(|block| block.results.iter())
        .collect();

    match format {
        Format::Notation => {
            let mut out = String::new();
            for result in &results {
                render_one(&mut out, label, result);
            }
            let distinct: std::collections::HashSet<&str> =
                results.iter().map(|result| result.this.as_str()).collect();
            let (instances, rows) = (distinct.len(), results.len());
            if instances == rows {
                let _ = writeln!(
                    out,
                    "# {instances} {label} instance{s}",
                    s = plural(instances)
                );
            } else {
                let _ = writeln!(
                    out,
                    "# {instances} {label} instance{s} ({rows} rows; many-valued fields repeat rows)",
                    s = plural(instances)
                );
            }
            Ok(out)
        }
        Format::Json => {
            let instances: Vec<serde_json::Value> = results
                .iter()
                .map(|result| {
                    let mut object = serde_json::Map::new();
                    object.insert(
                        "this".into(),
                        serde_json::Value::String(result.this.clone()),
                    );
                    for (field, value) in &result.fields {
                        object.insert(field.clone(), value.clone());
                    }
                    serde_json::Value::Object(object)
                })
                .collect();
            let mut rendered =
                serde_json::to_string_pretty(&instances).context("JSON serialization failed")?;
            rendered.push('\n');
            Ok(rendered)
        }
    }
}

// ---------------------------------------------------------------- //
// JSON renderer                                                    //
// ---------------------------------------------------------------- //

#[derive(Serialize)]
struct QuietJson<'a> {
    revision_before: Option<&'a dialog_repository::Revision>,
    revision_after: Option<&'a dialog_repository::Revision>,
    commits: &'a CommitSummary,
}

fn render_json(response: &EvaluateResponse, quiet: bool) -> Result<String> {
    if quiet {
        let projected = QuietJson {
            revision_before: response.revision_before.as_ref(),
            revision_after: response.revision_after.as_ref(),
            commits: &response.commits,
        };
        return serde_json::to_string_pretty(&projected).context("JSON serialization failed");
    }
    serde_json::to_string_pretty(response).context("JSON serialization failed")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use tonk_evaluator::evaluate::{CommitSummary, QueryMatchBlock, QueryResult};

    use super::*;

    fn sample_response() -> EvaluateResponse {
        let mut entities = BTreeMap::new();
        entities.insert("alice".into(), "did:key:zHj".into());
        entities.insert("?p".into(), "did:key:zHj".into());
        EvaluateResponse {
            revision_before: None,
            revision_after: None,
            matches_before: Vec::new(),
            matches_after: vec![QueryMatchBlock {
                label: "person".into(),
                results: vec![QueryResult {
                    this: "did:key:zHj".into(),
                    fields: {
                        let mut m = BTreeMap::new();
                        m.insert("name".into(), serde_json::json!("Alice"));
                        m.insert("age".into(), serde_json::json!(28));
                        m
                    },
                }],
            }],
            commits: CommitSummary {
                claims: 2,
                entities,
            },
        }
    }

    mod when_rendering_notation {
        use super::*;

        #[dialog_common::test]
        fn it_writes_an_envelope_followed_by_a_matches_section() {
            let out = render(&sample_response(), Format::Notation, false).unwrap();
            assert!(out.contains("revision-before: null"));
            assert!(out.contains("claims: 2"));
            assert!(out.contains("alice: did:key:zHj"));
            assert!(out.contains("---\n"));
            assert!(out.contains("person:\n  this: did:key:zHj"));
            assert!(out.contains("name: \"Alice\""));
            assert!(out.contains("age: 28"));
        }

        #[dialog_common::test]
        fn it_omits_the_matches_section_when_quiet() {
            let out = render(&sample_response(), Format::Notation, true).unwrap();
            assert!(!out.contains("---"));
            assert!(!out.contains("this: did:key:"));
        }

        #[dialog_common::test]
        fn it_skips_blocks_with_zero_results() {
            let mut response = sample_response();
            response.matches_after = vec![QueryMatchBlock {
                label: "person".into(),
                results: vec![],
            }];
            let out = render(&response, Format::Notation, false).unwrap();
            assert!(!out.contains("---"));
            assert!(!out.contains("person"));
        }
    }

    mod when_rendering_json {
        use super::*;

        #[dialog_common::test]
        fn it_serializes_the_full_response() {
            let out = render(&sample_response(), Format::Json, false).unwrap();
            assert!(out.contains("\"matches_after\""));
            assert!(out.contains("\"claims\": 2"));
        }

        #[dialog_common::test]
        fn it_omits_matches_when_quiet() {
            let out = render(&sample_response(), Format::Json, true).unwrap();
            assert!(!out.contains("\"matches_after\""));
            assert!(out.contains("\"commits\""));
        }
    }
}

#[cfg(test)]
mod value_spelling_tests {
    use super::*;

    #[test]
    fn it_renders_number_spellings() {
        // Bare digits are unsigned; the wire spells signed integers
        // as explicitly signed strings, printed bare; floats keep
        // their decimal point.
        assert_eq!(render_value(&serde_json::json!(41u64)), "41");
        assert_eq!(render_value(&serde_json::json!("+41")), "+41");
        assert_eq!(render_value(&serde_json::json!("-7")), "-7");
        assert_eq!(render_value(&serde_json::json!(41.5)), "41.5");
        assert_eq!(render_value(&serde_json::json!(41.0)), "41.0");
        // Ordinary strings still quote — including number-adjacent text.
        assert_eq!(render_value(&serde_json::json!("41a")), "\"41a\"");
        assert_eq!(render_value(&serde_json::json!("+")), "\"+\"");
    }
}
