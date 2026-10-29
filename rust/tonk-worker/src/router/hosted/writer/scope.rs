//! Which reads a space writer serves, judged before they run.
//!
//! The writer's memory is the high water of everything it has run, and
//! wasm memory never shrinks, not even across a `ctx.abort()` that keeps
//! the isolate. The answer caps bound what a read returns, not what it
//! scans: a filter matching 400 of 20k records held 57.8 MiB in the
//! spike for a small answer. So a read that may scan is sent to a
//! stateless isolate before it runs, and the writer keeps the reads whose
//! scan is bounded by the document itself:
//!
//! - every expression pinned to one entity (`this:` an exact URI or a
//!   name), or
//! - a schema read (`concept:`, `attribute:` and the like), bounded by
//!   the space's schema rather than its records; on a fresh space the
//!   largest, `concept:`, answers 67 rows in about 100 KB.
//!
//! A document that does not parse stays in the writer: it fails there
//! before it reads anything.

use tonk_notation::{Expression, FieldValue, HeadName, parse};

/// Built-in heads whose instances are schema, not records.
const SCHEMA: &[&str] = &["concept", "attribute", "rule", "command", "event", "name"];

/// Whether the read `document` is bounded by the document alone, so the
/// space writer may run it.
pub fn is_narrow_read(document: &str) -> bool {
    let Some(syntax) = parse(document).syntax else {
        return true;
    };
    syntax.expressions.iter().all(|expression| {
        let Expression::Query(query) = expression else {
            // A read with a claim in it is refused as a read anyway.
            return true;
        };
        let schema = matches!(&query.predicate.name, HeadName::Concept(name) if SCHEMA.contains(&name.as_str()));
        let pinned = query.fields.iter().any(|field| {
            field.name == "this" && matches!(field.value, FieldValue::Uri(_) | FieldValue::Symbol(_))
        });
        schema || pinned
    })
}
