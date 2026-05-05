//! Snapshot the author-supplied row template, extract a binding
//! plan, and apply that plan to a clone for each rendered row.
//!
//! Two pieces:
//! 1. The pure segment parser ([`parse_segments`]) that splits a
//!    string like `"Hello {name}!"` into an alternating sequence
//!    of literal text and field references.
//! 2. (Browser-only) DOM walking that builds a [`BindingPlan`]
//!    over a `DocumentFragment` and re-applies it to a clone.

/// One chunk of an interpolated string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// A literal text fragment.
    Text(String),
    /// A `{field}` reference; the inner string is the field name.
    Field(String),
}

/// Parse a `{field}`-interpolated string into a sequence of
/// [`Segment`]s. Single-identifier interpolation only — `{name}`
/// works, `{name + "x"}` does not (the inner expression is treated
/// as the field name verbatim, leading to a guaranteed lookup
/// miss).
///
/// A literal `{` cannot appear in input today; document this
/// limitation upstream.
pub fn parse_segments(input: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '{' {
            buf.push(ch);
            continue;
        }
        // Find the matching '}'.
        let mut name = String::new();
        let mut closed = false;
        for nch in chars.by_ref() {
            if nch == '}' {
                closed = true;
                break;
            }
            name.push(nch);
        }
        if !closed {
            // Unterminated — emit as literal.
            buf.push('{');
            buf.push_str(&name);
            continue;
        }
        if !buf.is_empty() {
            out.push(Segment::Text(std::mem::take(&mut buf)));
        }
        out.push(Segment::Field(name));
    }
    if !buf.is_empty() {
        out.push(Segment::Text(buf));
    }
    out
}

/// True if any segment is a [`Segment::Field`].
pub fn has_field(segments: &[Segment]) -> bool {
    segments.iter().any(|s| matches!(s, Segment::Field(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_parses_plain_text_as_one_segment() {
        assert_eq!(parse_segments("hello"), vec![Segment::Text("hello".into())]);
    }

    #[test]
    fn it_parses_a_single_field_reference() {
        assert_eq!(
            parse_segments("{name}"),
            vec![Segment::Field("name".into())]
        );
    }

    #[test]
    fn it_parses_text_field_text() {
        assert_eq!(
            parse_segments("Hello {name}!"),
            vec![
                Segment::Text("Hello ".into()),
                Segment::Field("name".into()),
                Segment::Text("!".into()),
            ],
        );
    }

    #[test]
    fn it_parses_two_adjacent_fields() {
        assert_eq!(
            parse_segments("{first}{last}"),
            vec![
                Segment::Field("first".into()),
                Segment::Field("last".into()),
            ],
        );
    }

    #[test]
    fn it_parses_multiple_fields_with_separators() {
        assert_eq!(
            parse_segments("{name} is {age}"),
            vec![
                Segment::Field("name".into()),
                Segment::Text(" is ".into()),
                Segment::Field("age".into()),
            ],
        );
    }

    #[test]
    fn it_treats_unterminated_brace_as_literal() {
        assert_eq!(
            parse_segments("oops {name"),
            vec![Segment::Text("oops {name".into())],
        );
    }

    #[test]
    fn it_returns_empty_for_empty_input() {
        assert!(parse_segments("").is_empty());
    }

    #[test]
    fn it_detects_field_segments() {
        assert!(!has_field(&parse_segments("plain text")));
        assert!(has_field(&parse_segments("hello {name}")));
    }
}
