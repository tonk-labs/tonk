//! Native template diagnostics — names the declarative-template
//! footguns explicitly instead of letting them render as silent
//! blanks. Pre-render checks compare the template's `{field}`
//! references against the model's real field set and the
//! projected values; the post-render check (see [`post_render`])
//! compares resolved values against the renderer's actual output.

use ipld_core::ipld::Ipld;
use serde::Serialize;
use tonk_display::template::{Segment, parse_segments};
use tonk_schema::conclusion::Conclusion;

/// One named footgun, machine-readable for `--json` and rendered
/// human-readable via [`std::fmt::Display`].
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Diagnostic {
    /// `{field}` is not in the model's projected field set.
    UnboundField {
        /// The referenced field name.
        field: String,
        /// Closest real field name, when one is plausibly meant.
        suggestion: Option<String>,
    },
    /// The field exists on the model but resolved empty/absent on
    /// every projected row.
    EmptyResolve {
        /// The field that resolved empty.
        field: String,
    },
    /// The entity projected zero rows — the renderer will show
    /// fallback chrome, not data.
    EmptyFrame,
    /// The field resolved to a non-empty value but that value does
    /// not appear in the rendered output — the symptom of the
    /// single-occurrence-text and iteration-root anchoring traps.
    ValueMissingFromOutput {
        /// The field whose value went missing.
        field: String,
        /// The resolved value that should have appeared.
        value: String,
    },
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnboundField {
                field,
                suggestion: Some(s),
            } => {
                write!(
                    f,
                    "unbound-field: {{{field}}} is not a model field — did you mean {{{s}}}?"
                )
            }
            Self::UnboundField {
                field,
                suggestion: None,
            } => {
                write!(f, "unbound-field: {{{field}}} is not a model field")
            }
            Self::EmptyResolve { field } => {
                write!(f, "empty-resolve: {{{field}}} resolved empty on every row")
            }
            Self::EmptyFrame => write!(
                f,
                "empty-frame: the entity projected zero rows; fallback chrome rendered, not data"
            ),
            Self::ValueMissingFromOutput { field, value } => write!(
                f,
                "value-missing-from-output: {{{field}}} resolved to \"{value}\" but the value \
                 does not appear in the rendered HTML (template anchoring trap — see \
                 `slide guide views`)"
            ),
        }
    }
}

/// Distinct `{field}` names referenced by `template`, in first-use
/// order, excluding `this` and `dom.host/*` (not subject fields).
pub fn referenced_fields(template: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for segment in parse_segments(template) {
        if let Segment::Field(name) = segment
            && name != "this"
            && !name.starts_with("dom.host/")
            && !out.contains(&name)
        {
            out.push(name);
        }
    }
    out
}

/// Pre-render checks: unbound fields (with did-you-mean), fields
/// empty on every row, and the empty-frame case.
pub fn pre_render(
    template: &str,
    descriptor_fields: &[String],
    conclusions: &[Conclusion],
) -> Vec<Diagnostic> {
    if conclusions.is_empty() {
        return vec![Diagnostic::EmptyFrame];
    }
    let mut out = Vec::new();
    for field in referenced_fields(template) {
        if !descriptor_fields.contains(&field) {
            let suggestion = closest_field(&field, descriptor_fields);
            out.push(Diagnostic::UnboundField { field, suggestion });
        } else if conclusions
            .iter()
            .all(|c| is_empty_value(c.fields.get(&field)))
        {
            out.push(Diagnostic::EmptyResolve { field });
        }
    }
    out
}

/// A field counts as empty when absent or an empty string.
fn is_empty_value(value: Option<&Ipld>) -> bool {
    match value {
        None | Some(Ipld::Null) => true,
        Some(Ipld::String(s)) => s.is_empty(),
        Some(_) => false,
    }
}

/// Closest real field by edit distance, when within 2 edits.
fn closest_field(field: &str, available: &[String]) -> Option<String> {
    available
        .iter()
        .map(|candidate| (levenshtein(field, candidate), candidate))
        .filter(|(distance, _)| *distance <= 2)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, candidate)| candidate.clone())
}

/// Classic two-row Levenshtein — small inputs, no dependency.
fn levenshtein(a: &str, b: &str) -> usize {
    let b_chars: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b_chars.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut current = vec![i + 1];
        for (j, cb) in b_chars.iter().enumerate() {
            let substitution = previous[j] + usize::from(ca != *cb);
            current.push(substitution.min(previous[j + 1] + 1).min(current[j] + 1));
        }
        previous = current;
    }
    previous[b_chars.len()]
}

/// Post-render check: every referenced field whose value resolved
/// non-empty must appear somewhere in the rendered HTML. A miss is
/// the black-box symptom of the structural anchoring traps
/// (single-occurrence bare text, surprising iteration root) without
/// needing renderer internals.
pub fn post_render(template: &str, conclusions: &[Conclusion], html: &str) -> Vec<Diagnostic> {
    if conclusions.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for field in referenced_fields(template) {
        for conclusion in conclusions {
            if let Some(value) = conclusion.fields.get(&field)
                && let Some(text) = display_value(value)
                && !text.is_empty()
                && !html.contains(&text)
            {
                out.push(Diagnostic::ValueMissingFromOutput {
                    field: field.clone(),
                    value: text,
                });
                break; // one report per field is enough
            }
        }
    }
    out
}

/// Stringify a scalar Ipld value the way the renderer would
/// interpolate it. Non-scalar values are skipped (no containment
/// check is meaningful for them).
fn display_value(value: &Ipld) -> Option<String> {
    match value {
        Ipld::String(s) => Some(s.clone()),
        Ipld::Integer(i) => Some(i.to_string()),
        Ipld::Float(x) => Some(x.to_string()),
        Ipld::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipld_core::ipld::Ipld;
    use std::collections::BTreeMap;
    use tonk_schema::conclusion::Conclusion;

    fn conclusion(fields: &[(&str, &str)]) -> Conclusion {
        let mut map = BTreeMap::new();
        for (k, v) in fields {
            map.insert((*k).to_string(), Ipld::String((*v).to_string()));
        }
        Conclusion {
            this: "did:key:zX".into(),
            fields: map,
        }
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    mod when_extracting_referenced_fields {
        use super::*;

        #[dialog_common::test]
        fn it_finds_fields_in_text_and_attributes() {
            let fields = referenced_fields("<a href=\"/p/{slug}\"><b>{name}</b></a>");
            assert_eq!(fields, strings(&["slug", "name"]));
        }

        #[dialog_common::test]
        fn it_skips_this_and_dom_host_references() {
            let fields = referenced_fields("<i data-e=\"{this}\">{dom.host/value} {name}</i>");
            assert_eq!(fields, strings(&["name"]));
        }

        #[dialog_common::test]
        fn it_dedupes_repeated_references() {
            let fields = referenced_fields("<b>{name}</b><i>{name}</i>");
            assert_eq!(fields, strings(&["name"]));
        }
    }

    mod when_analyzing_before_render {
        use super::*;

        #[dialog_common::test]
        fn it_flags_an_unbound_field_with_a_suggestion() {
            let diagnostics = pre_render(
                "<b>{nmae}</b>",
                &strings(&["name", "age"]),
                &[conclusion(&[("name", "Alice")])],
            );
            assert_eq!(
                diagnostics,
                vec![Diagnostic::UnboundField {
                    field: "nmae".into(),
                    suggestion: Some("name".into()),
                }],
            );
        }

        #[dialog_common::test]
        fn it_flags_a_field_that_resolved_empty_on_every_row() {
            let diagnostics = pre_render(
                "<b>{name}</b><i>{nickname}</i>",
                &strings(&["name", "nickname"]),
                &[conclusion(&[("name", "Alice")])],
            );
            assert_eq!(
                diagnostics,
                vec![Diagnostic::EmptyResolve {
                    field: "nickname".into()
                }],
            );
        }

        #[dialog_common::test]
        fn it_flags_an_empty_frame_instead_of_per_field_noise() {
            let diagnostics = pre_render("<b>{name}</b>", &strings(&["name"]), &[]);
            assert_eq!(diagnostics, vec![Diagnostic::EmptyFrame]);
        }

        #[dialog_common::test]
        fn it_reports_nothing_for_a_clean_template() {
            let diagnostics = pre_render(
                "<b>{name}</b>",
                &strings(&["name"]),
                &[conclusion(&[("name", "Alice")])],
            );
            assert!(diagnostics.is_empty(), "got {diagnostics:?}");
        }
    }

    mod when_analyzing_after_render {
        use super::*;

        #[dialog_common::test]
        fn it_flags_a_resolved_value_missing_from_the_output() {
            let diagnostics = post_render(
                "<article>{name}</article>",
                &[conclusion(&[("name", "Alice")])],
                "<article></article>",
            );
            assert_eq!(
                diagnostics,
                vec![Diagnostic::ValueMissingFromOutput {
                    field: "name".into(),
                    value: "Alice".into(),
                }],
            );
        }

        #[dialog_common::test]
        fn it_accepts_output_containing_every_resolved_value() {
            let diagnostics = post_render(
                "<article>{name}</article>",
                &[conclusion(&[("name", "Alice")])],
                "<article>Alice</article>",
            );
            assert!(diagnostics.is_empty(), "got {diagnostics:?}");
        }

        #[dialog_common::test]
        fn it_stays_quiet_on_an_empty_frame() {
            let diagnostics = post_render("<article>{name}</article>", &[], "<tonk-fallback>");
            assert!(
                diagnostics.is_empty(),
                "empty frame already flagged pre-render"
            );
        }
    }
}
