//! Tokenizer for `<tonk-notation>`. Walks a parsed
//! `tonk_notation::Syntax` tree and emits a sorted,
//! non-overlapping list of decoration marks tagged with class
//! names that match the dialog-yaml editor pack
//! (`tonk-cm-effect`, `tonk-cm-name-sigil`, `tonk-cm-name`,
//! `tonk-cm-entity`, `tonk-cm-variable`). Kept out of the
//! wasm-only `notation.rs` so native tests can exercise it
//! without spinning up the DOM glue.

use lsp_types::{Position, Range};
use tonk_notation::syntax::{
    Anchor, Application, Effectful, Expression, Field, FieldValue, HeadName, Predicate, Syntax,
};

/// Editor decoration class names. The `class()` method returns
/// `None` for the `Plain` variant so the renderer can fall through
/// to an unstyled span for the gaps between marks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decoration {
    /// No decoration — plain text between marks.
    Plain,
    /// `head!` — the `!` plus the head name on assertions.
    Effect,
    /// The `&` introducing an anchor name.
    NameSigil,
    /// The anchor name itself (after the `&`) or a bare-symbol
    /// field value.
    Name,
    /// An entity URI used as a head or field value.
    Entity,
    /// A `?variable` or `_` blank field value.
    Variable,
    /// A field name (the level-3 mapping key — `this`, `message`,
    /// `model`, etc.). In the editor these are painted by
    /// CodeMirror's YAML grammar via the `--tonk-code-key`
    /// variable; we don't have Lezer here so the tokenizer emits
    /// the same class explicitly.
    Key,
}

impl Decoration {
    /// The CSS class name to apply to a `<span>` for this
    /// decoration, or `None` for plain text.
    pub fn class(self) -> Option<&'static str> {
        match self {
            Decoration::Plain => None,
            Decoration::Effect => Some("tonk-cm-effect"),
            Decoration::NameSigil => Some("tonk-cm-name-sigil"),
            Decoration::Name => Some("tonk-cm-name"),
            Decoration::Entity => Some("tonk-cm-entity"),
            Decoration::Variable => Some("tonk-cm-variable"),
            Decoration::Key => Some("tonk-cm-key"),
        }
    }
}

/// One byte-range decoration applied to the rendered source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    /// Inclusive UTF-8 byte offset where the mark begins.
    pub from: usize,
    /// Exclusive UTF-8 byte offset where the mark ends.
    pub to: usize,
    /// What to render.
    pub decoration: Decoration,
}

/// Parse `text` and collect a sorted, non-overlapping list of
/// decoration marks. Returns an empty list when the parse
/// produces no usable syntax tree (e.g. for an empty input).
pub fn collect_marks(text: &str) -> Vec<Mark> {
    let parsed = tonk_notation::parse(text);
    let Some(syntax) = parsed.syntax else {
        return Vec::new();
    };
    let line_starts = compute_line_starts(text);
    let mut marks: Vec<Mark> = Vec::new();
    walk_syntax(&syntax, text, &line_starts, &mut marks);
    marks.sort_by_key(|m| (m.from, m.to));
    dedupe_overlaps(marks)
}

fn walk_syntax(syntax: &Syntax, text: &str, line_starts: &[usize], out: &mut Vec<Mark>) {
    for expr in &syntax.expressions {
        match expr {
            Expression::Query(q) => {
                walk_application(q, /*effectful=*/ false, text, line_starts, out)
            }
            Expression::Claim(Effectful { anchor, inner }) => {
                walk_application(inner, /*effectful=*/ true, text, line_starts, out);
                if let Some(anchor) = anchor {
                    mark_anchor(anchor, line_starts, out);
                }
            }
        }
    }
}

fn walk_application(
    app: &Application,
    effectful: bool,
    text: &str,
    line_starts: &[usize],
    out: &mut Vec<Mark>,
) {
    if effectful {
        mark_head(&app.predicate, text, line_starts, out);
    } else {
        // Queries get only the URI-entity mark, not the Effect mark.
        if let HeadName::Uri(_) = app.predicate.name
            && let Some((from, to)) = range_to_bytes(&app.predicate.range, line_starts)
        {
            out.push(Mark {
                from,
                to,
                decoration: Decoration::Entity,
            });
        }
    }
    for field in &app.fields {
        walk_field(field, text, line_starts, out);
    }
}

fn mark_head(head: &Predicate, text: &str, line_starts: &[usize], out: &mut Vec<Mark>) {
    let Some((from, to)) = range_to_bytes(&head.range, line_starts) else {
        return;
    };
    // The parser's head range covers the bare name without the
    // trailing `!`. Extend by one byte when the next character is
    // `!` so the Effect decoration paints both — matching how
    // dialog-yaml.ts highlights it.
    let extended_to = match text.as_bytes().get(to) {
        Some(b'!') => to + 1,
        _ => to,
    };
    out.push(Mark {
        from,
        to: extended_to,
        decoration: Decoration::Effect,
    });
    if let HeadName::Uri(_) = head.name {
        out.push(Mark {
            from,
            to,
            decoration: Decoration::Entity,
        });
    }
}

fn mark_anchor(anchor: &Anchor, line_starts: &[usize], out: &mut Vec<Mark>) {
    let Some((from, to)) = range_to_bytes(&anchor.range, line_starts) else {
        return;
    };
    out.push(Mark {
        from,
        to: from + 1,
        decoration: Decoration::NameSigil,
    });
    if to > from + 1 {
        out.push(Mark {
            from: from + 1,
            to,
            decoration: Decoration::Name,
        });
    }
}

fn walk_field(field: &Field, text: &str, line_starts: &[usize], out: &mut Vec<Mark>) {
    if let Some((from, to)) = range_to_bytes(&field.name_range, line_starts) {
        out.push(Mark {
            from,
            to,
            decoration: Decoration::Key,
        });
    }
    walk_value(&field.value, &field.value_range, text, line_starts, out);
}

fn walk_value(
    value: &FieldValue,
    value_range: &Range,
    text: &str,
    line_starts: &[usize],
    out: &mut Vec<Mark>,
) {
    let Some((from, to)) = range_to_bytes(value_range, line_starts) else {
        return;
    };
    match value {
        FieldValue::Variable(_) | FieldValue::Blank => out.push(Mark {
            from,
            to,
            decoration: Decoration::Variable,
        }),
        FieldValue::Symbol(_) => out.push(Mark {
            from,
            to,
            decoration: Decoration::Name,
        }),
        FieldValue::Uri(_) => out.push(Mark {
            from,
            to,
            decoration: Decoration::Entity,
        }),
        FieldValue::Nested(fields) => {
            for field in fields {
                walk_field(field, text, line_starts, out);
            }
        }
        FieldValue::Premises(premises) => {
            // A premise's `where:` bindings paint as ordinary
            // field values; the `assert: <concept>` key paints as
            // a key. The premise mapping's own range is its
            // structure, no extra mark for the list itself.
            for premise in premises {
                for binding in &premise.bindings {
                    walk_field(binding, text, line_starts, out);
                }
            }
        }
        FieldValue::Literal(_) => {}
    }
}

fn dedupe_overlaps(marks: Vec<Mark>) -> Vec<Mark> {
    let mut out: Vec<Mark> = Vec::with_capacity(marks.len());
    for mark in marks {
        let drop = matches!(out.last(), Some(prev) if mark.from < prev.to);
        if drop {
            continue;
        }
        out.push(mark);
    }
    out
}

fn compute_line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    for (idx, b) in text.bytes().enumerate() {
        if b == b'\n' {
            starts.push(idx + 1);
        }
    }
    starts
}

fn position_to_byte(pos: &Position, line_starts: &[usize]) -> Option<usize> {
    let line = pos.line as usize;
    if line >= line_starts.len() {
        return None;
    }
    // LSP characters are UTF-16 code units; our notation is
    // ASCII-leaning. Approximate `character` as a byte offset
    // within the line. Documented limitation rather than pulling
    // in a UTF-16 lookup table.
    Some(line_starts[line] + pos.character as usize)
}

fn range_to_bytes(range: &Range, line_starts: &[usize]) -> Option<(usize, usize)> {
    let from = position_to_byte(&range.start, line_starts)?;
    let to = position_to_byte(&range.end, line_starts)?;
    if from > to {
        return None;
    }
    Some((from, to))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_decorates_an_assertion_head() {
        let text = "greeting!:\n  this: did:key:zX\n";
        let marks = collect_marks(text);
        assert!(
            marks
                .iter()
                .any(|m| m.decoration == Decoration::Effect && &text[m.from..m.to] == "greeting!")
        );
        assert!(
            marks
                .iter()
                .any(|m| m.decoration == Decoration::Entity && &text[m.from..m.to] == "did:key:zX")
        );
    }

    #[test]
    fn it_decorates_an_anchor() {
        let text = "greeting!: &demo\n  this: did:key:zX\n";
        let marks = collect_marks(text);
        assert!(
            marks
                .iter()
                .any(|m| m.decoration == Decoration::NameSigil && &text[m.from..m.to] == "&")
        );
        assert!(
            marks
                .iter()
                .any(|m| m.decoration == Decoration::Name && &text[m.from..m.to] == "demo")
        );
    }

    #[test]
    fn it_decorates_a_variable() {
        let text = "greeting!:\n  this: ?alice\n";
        let marks = collect_marks(text);
        assert!(
            marks
                .iter()
                .any(|m| m.decoration == Decoration::Variable && &text[m.from..m.to] == "?alice")
        );
    }

    #[test]
    fn it_decorates_a_bare_symbol_value() {
        let text = "view!:\n  this: greeting\n";
        let marks = collect_marks(text);
        assert!(
            marks
                .iter()
                .any(|m| m.decoration == Decoration::Name && &text[m.from..m.to] == "greeting")
        );
    }

    #[test]
    fn it_returns_no_marks_for_empty_input() {
        assert!(collect_marks("").is_empty());
    }

    #[test]
    fn it_maps_decorations_to_dialog_yaml_class_names() {
        // The whole point of this tokenizer is to share class names
        // with the dialog-yaml editor pack — pin the mapping so a
        // rename on either side surfaces here.
        assert_eq!(Decoration::Plain.class(), None);
        assert_eq!(Decoration::Effect.class(), Some("tonk-cm-effect"));
        assert_eq!(Decoration::NameSigil.class(), Some("tonk-cm-name-sigil"));
        assert_eq!(Decoration::Name.class(), Some("tonk-cm-name"));
        assert_eq!(Decoration::Entity.class(), Some("tonk-cm-entity"));
        assert_eq!(Decoration::Variable.class(), Some("tonk-cm-variable"));
        assert_eq!(Decoration::Key.class(), Some("tonk-cm-key"));
    }

    #[test]
    fn it_leaves_a_query_head_undecorated() {
        // Queries (`head:` without `!`) are reads, not effects —
        // the editor doesn't paint them with the alarm-red effect
        // decoration, and neither do we.
        let text = "greeting:\n  message: ?msg\n";
        let marks = collect_marks(text);
        assert!(
            !marks.iter().any(|m| m.decoration == Decoration::Effect),
            "no Effect decoration expected for a query head, got {marks:?}",
        );
    }

    #[test]
    fn it_decorates_a_retraction_blank_as_variable() {
        // `_` in field-value position is a retraction in an
        // assertion. The editor paints it with the variable
        // decoration (italic blue); we mirror that.
        let text = "greeting!:\n  message: _\n";
        let marks = collect_marks(text);
        assert!(
            marks
                .iter()
                .any(|m| m.decoration == Decoration::Variable && &text[m.from..m.to] == "_")
        );
    }

    #[test]
    fn it_recurses_into_nested_fields() {
        // Nested maps appear as `concept!.with` definitions and
        // similar — the tokenizer needs to descend so inner keys
        // and values get their own decorations.
        let text = "concept!:\n  with:\n    message: ?inner\n";
        let marks = collect_marks(text);
        assert!(
            marks
                .iter()
                .any(|m| m.decoration == Decoration::Variable && &text[m.from..m.to] == "?inner"),
            "expected Variable on nested `?inner`, got {marks:?}",
        );
        // Outer + inner keys both surface.
        let keys: Vec<&str> = marks
            .iter()
            .filter(|m| m.decoration == Decoration::Key)
            .map(|m| &text[m.from..m.to])
            .collect();
        assert!(keys.contains(&"with"), "expected `with` key, got {keys:?}");
        assert!(
            keys.contains(&"message"),
            "expected nested `message` key, got {keys:?}",
        );
    }

    #[test]
    fn it_yields_sorted_non_overlapping_marks() {
        // Renderer relies on `from <= cursor` invariant to slice
        // text without overlap; pin the contract.
        let text = "greeting!: &demo\n  this: did:key:zX\n  message: \"Hi\"\n";
        let marks = collect_marks(text);
        let mut last_to = 0usize;
        for m in &marks {
            assert!(
                m.from >= last_to,
                "overlap: mark {m:?} starts before previous end {last_to}",
            );
            assert!(m.from <= m.to, "inverted mark: {m:?}");
            last_to = m.to;
        }
    }

    #[test]
    fn it_decorates_field_names_as_keys() {
        let text = "greeting!:\n  this: did:key:zX\n  message: \"Hi\"\n";
        let marks = collect_marks(text);
        let keys: Vec<&str> = marks
            .iter()
            .filter(|m| m.decoration == Decoration::Key)
            .map(|m| &text[m.from..m.to])
            .collect();
        assert!(keys.contains(&"this"), "expected `this` key, got {keys:?}");
        assert!(
            keys.contains(&"message"),
            "expected `message` key, got {keys:?}"
        );
    }
}
