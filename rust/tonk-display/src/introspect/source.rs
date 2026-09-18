//! The view template, sliced into what a reader needs to see in it.
//!
//! The concept panel answers what the data is. The view panel answers
//! what the template asked for — which is the other half of any
//! mismatch, and the half you cannot get at from a rendered page at
//! all, because interpolation is over by then.
//!
//! [`pieces`] cuts the template text into a flat run of literal spans,
//! `{field}` references and command-bound attribute values. It walks
//! with [`tonk_template::scan::walk`], the same lexer the analyzer's
//! checks use, so what the panel marks and what the build reports
//! cannot disagree about what is in a template. That matters for the
//! two exclusions the walk encodes: a `{field}` written inside an HTML
//! comment is prose, and one inside `<style>` or `<script>` is a real
//! CSS or JS brace. Marking either would make every stylesheet in the
//! library read as a wall of interpolations.
//!
//! Unlike [`tonk_template::fields::scan`], which keeps one earliest
//! offset per name for diagnostics, this keeps *every* occurrence: the
//! panel highlights all the places a field is written, not the first.

use serde::{Deserialize, Serialize};
use tonk_template::scan::{Found, walk};

/// One run of template text, classified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "piece", rename_all = "kebab-case")]
pub enum Piece {
    /// Ordinary template text, marked as nothing.
    Literal {
        /// The text, verbatim.
        text: String,
    },
    /// A `{field}` reference, braces included in `text`.
    Field {
        /// The name between the braces.
        name: String,
        /// The reference as written.
        text: String,
    },
    /// The value of an attribute that binds an interaction — the
    /// command it posts.
    Command {
        /// The command name or URI.
        name: String,
        /// The value as written.
        text: String,
    },
}

impl Piece {
    /// The template text this piece covers.
    pub fn text(&self) -> &str {
        match self {
            Piece::Literal { text } | Piece::Field { text, .. } | Piece::Command { text, .. } => {
                text
            }
        }
    }
}

/// A byte range the walk found worth marking.
struct Mark {
    start: usize,
    end: usize,
    /// `Some` for a command, `None` for a field.
    command: bool,
    name: String,
}

/// Cut `template` into literal, field and command runs, in order,
/// covering every byte exactly once.
pub fn pieces(template: &str) -> Vec<Piece> {
    let mut marks: Vec<Mark> = Vec::new();
    walk(template, &mut |found| match found {
        Found::Text { text, offset } => collect_fields(text, offset, &mut marks),
        Found::Attribute {
            name,
            value,
            value_offset,
            ..
        } => {
            if let Some(command) = command_name(name, value) {
                marks.push(Mark {
                    start: value_offset,
                    end: value_offset + value.len(),
                    command: true,
                    name: command,
                });
                // A command's value is the command, whatever braces it
                // happens to contain; do not also mark inside it.
                return;
            }
            collect_fields(value, value_offset, &mut marks);
        }
    });

    marks.sort_by_key(|mark| mark.start);

    let mut out: Vec<Piece> = Vec::new();
    let mut cursor = 0usize;
    for mark in marks {
        // The walk can report overlapping regions in odd templates;
        // the first mark wins rather than producing torn output.
        if mark.start < cursor {
            continue;
        }
        if mark.start > cursor {
            push_literal(&mut out, &template[cursor..mark.start]);
        }
        let text = template[mark.start..mark.end].to_owned();
        out.push(if mark.command {
            Piece::Command {
                name: mark.name,
                text,
            }
        } else {
            Piece::Field {
                name: mark.name,
                text,
            }
        });
        cursor = mark.end;
    }
    if cursor < template.len() {
        push_literal(&mut out, &template[cursor..]);
    }
    out
}

/// Every `{field}` in one run, at absolute offsets.
fn collect_fields(run: &str, base: usize, marks: &mut Vec<Mark>) {
    let mut rest = run;
    let mut consumed = 0usize;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            break;
        };
        let name = &after[..close];
        // `{}` names nothing and `{a{b}` is not a reference either;
        // the renderer's parser would read neither as one.
        if !name.is_empty() && !name.contains('{') {
            marks.push(Mark {
                start: base + consumed + open,
                end: base + consumed + open + name.len() + 2,
                command: false,
                name: name.to_owned(),
            });
        }
        let step = open + 1 + close + 1;
        consumed += step;
        rest = &rest[step..];
    }
}

/// The command an attribute binds, if it binds one.
///
/// Both forms: `on:<name>` names a declaration, and `on<event>` is the
/// older one. A template read off the branch has not been through the
/// renderer's rewrite pass, so `on<event>` appears in its raw spelling
/// here rather than as `data-on<event>`; both are accepted, since the
/// panel is also shown template text that has been through it.
///
/// The `on<event>` test deliberately mirrors `preprocess`'s
/// `strip_on_prefix`, ambiguity included: any `on<ascii-alpha>…`
/// attribute is a candidate binding there, so `once="yes"` really is
/// treated as a handler by the renderer. The panel's job is to report
/// what the renderer does, not what it ought to do — a panel that
/// quietly disagreed would send an author looking for the wrong bug.
fn command_name(attribute: &str, value: &str) -> Option<String> {
    let command = value.trim();
    if command.is_empty() {
        return None;
    }
    if tonk_template::event::event_name_for_attribute(attribute).is_some() {
        return Some(command.to_owned());
    }
    let rest = attribute
        .strip_prefix("data-on")
        .or_else(|| attribute.strip_prefix("on"))?;
    if !rest.chars().next()?.is_ascii_alphabetic() {
        return None;
    }
    Some(command.to_owned())
}

fn push_literal(out: &mut Vec<Piece>, text: &str) {
    if text.is_empty() {
        return;
    }
    out.push(Piece::Literal {
        text: text.to_owned(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(template: &str) -> Vec<String> {
        pieces(template)
            .into_iter()
            .filter_map(|piece| match piece {
                Piece::Field { name, .. } => Some(name),
                _ => None,
            })
            .collect()
    }

    fn commands(template: &str) -> Vec<String> {
        pieces(template)
            .into_iter()
            .filter_map(|piece| match piece {
                Piece::Command { name, .. } => Some(name),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_pieces_reassemble_into_the_template() {
        let template = "<p class=\"{kind}\">Hello {name}</p>";
        let rebuilt: String = pieces(template)
            .iter()
            .map(Piece::text)
            .collect::<Vec<_>>()
            .concat();
        assert_eq!(rebuilt, template);
    }

    #[test]
    fn it_marks_a_field_in_text_and_in_an_attribute() {
        assert_eq!(
            fields("<p class=\"{kind}\">Hello {name}</p>"),
            ["kind", "name"]
        );
    }

    #[test]
    fn it_keeps_every_occurrence_not_just_the_first() {
        assert_eq!(
            fields("<p>{name}</p><b>{name}</b>"),
            ["name", "name"],
            "the panel highlights all of them, unlike the diagnostic scan"
        );
    }

    #[test]
    fn a_reference_in_a_comment_is_prose() {
        assert!(fields("<!-- {name} is the title --><p>x</p>").is_empty());
    }

    #[test]
    fn braces_in_a_style_body_are_css() {
        assert!(fields("<style>.a { color: red }</style>").is_empty());
    }

    #[test]
    fn it_marks_both_binding_forms_as_commands() {
        assert_eq!(
            commands("<button on:click=\"space/create\">go</button>"),
            ["space/create"]
        );
        assert_eq!(
            commands("<button onclick=\"space/create\">go</button>"),
            ["space/create"]
        );
    }

    #[test]
    fn a_command_value_is_one_piece_even_with_braces_in_it() {
        let found = pieces("<button on:click=\"cmd/{x}\">go</button>");
        let commands: Vec<&Piece> = found
            .iter()
            .filter(|piece| matches!(piece, Piece::Command { .. }))
            .collect();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].text(), "cmd/{x}");
        assert!(fields("<button on:click=\"cmd/{x}\">go</button>").is_empty());
    }

    #[test]
    fn it_mirrors_the_renderers_loose_on_prefix_rule() {
        // `preprocess::strip_on_prefix` treats any `on<ascii-alpha>…`
        // attribute as a candidate binding and says so in its own
        // comment. The panel reports what the renderer does, so
        // `once` is marked here too — disagreeing would be a nicer
        // panel and a worse answer.
        assert_eq!(commands("<p once=\"yes\">x</p>"), ["yes"]);
        assert!(
            commands("<p on-thing=\"yes\">x</p>").is_empty(),
            "a non-alphabetic char after `on` is not a binding, there or here"
        );
    }

    #[test]
    fn an_empty_or_malformed_reference_is_literal() {
        assert!(fields("<p>{} {a{b} {ok}</p>").contains(&"ok".to_owned()));
        assert_eq!(fields("<p>{} {ok}</p>"), ["ok"]);
    }

    #[test]
    fn an_unterminated_reference_ends_the_scan_of_its_run() {
        assert!(fields("<p>{name</p>").is_empty());
    }
}
