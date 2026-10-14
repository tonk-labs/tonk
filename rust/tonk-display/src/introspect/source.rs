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

/// How a run of template text reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Markup {
    /// Character data between tags.
    Text,
    /// A tag name.
    Tag,
    /// An attribute name.
    Attribute,
    /// A quoted attribute value.
    Value,
    /// Angle brackets, slashes, equals signs, quotes.
    Punct,
    /// An HTML comment, contents included.
    Comment,
    /// A `{field}` reference — the template asking for a value.
    Field,
    /// The value of an attribute that binds an interaction.
    Command,
}

/// One classified run of the template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Piece {
    /// How it reads.
    pub markup: Markup,
    /// The text, verbatim.
    pub text: String,
    /// For a field or command, the name it refers to — the key the
    /// inspector highlights by. `None` for plain markup.
    pub name: Option<String>,
}

impl Piece {
    /// The template text this piece covers.
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// A byte range the walk found worth marking.
struct Mark {
    start: usize,
    end: usize,
    markup: Markup,
    name: Option<String>,
}

/// Cut `template` into classified runs, in order, covering every byte
/// exactly once.
///
/// Two passes over one buffer rather than one clever merge. The first
/// decorates the markup — tags, attribute names, quoted values,
/// comments — which is cosmetic and can use a loose tokenizer. The
/// second overwrites it with the field and command marks, which are
/// not cosmetic and come from [`walk`], the analyzer's own lexer. So
/// where it matters the panel and the build agree, and where it does
/// not the colouring is allowed to be approximate.
///
/// Classifying per byte and then run-length encoding is deliberately
/// the dull way to do it. The clever way is merging two overlapping
/// range lists with splitting, and it is the kind of code that is
/// wrong in one corner for a year.
pub fn pieces(template: &str) -> Vec<Piece> {
    let bytes = template.len();
    let mut classes: Vec<Markup> = vec![Markup::Text; bytes];
    let mut names: Vec<Option<usize>> = vec![None; bytes];
    let mut table: Vec<String> = Vec::new();

    for mark in decorate(template) {
        paint(&mut classes, &mut names, &mut table, mark);
    }
    for mark in bindings(template) {
        paint(&mut classes, &mut names, &mut table, mark);
    }

    let mut out: Vec<Piece> = Vec::new();
    let mut start = 0usize;
    for index in 1..=bytes {
        let ended = index == bytes
            || classes[index] != classes[start]
            || names[index] != names[start]
            // Only split on a character boundary, or the slice below
            // would panic on a multi-byte character.
            || !template.is_char_boundary(index);
        if !ended || !template.is_char_boundary(index) {
            continue;
        }
        out.push(Piece {
            markup: classes[start],
            text: template[start..index].to_owned(),
            name: names[start].map(|id| table[id].clone()),
        });
        start = index;
    }
    out
}

/// Write a mark into the per-byte classification.
fn paint(classes: &mut [Markup], names: &mut [Option<usize>], table: &mut Vec<String>, mark: Mark) {
    let id = mark.name.map(|name| {
        table
            .iter()
            .position(|seen| *seen == name)
            .unwrap_or_else(|| {
                table.push(name);
                table.len() - 1
            })
    });
    for index in mark.start..mark.end.min(classes.len()) {
        classes[index] = mark.markup;
        names[index] = id;
    }
}

/// The cosmetic pass: tags, attribute names, quoted values, comments.
///
/// A loose tokenizer on purpose. It is colour, not meaning — nothing
/// downstream acts on it, and the marks that do carry meaning are
/// painted over it afterwards.
fn decorate(template: &str) -> Vec<Mark> {
    let bytes = template.as_bytes();
    let mut marks = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] != b'<' {
            index += 1;
            continue;
        }
        if template[index..].starts_with("<!--") {
            let end = template[index + 4..]
                .find("-->")
                .map(|offset| index + 4 + offset + 3)
                .unwrap_or(bytes.len());
            marks.push(Mark {
                start: index,
                end,
                markup: Markup::Comment,
                name: None,
            });
            index = end;
            continue;
        }
        let after = index + 1;
        if after >= bytes.len() || !(bytes[after].is_ascii_alphabetic() || bytes[after] == b'/') {
            index += 1;
            continue;
        }
        let open = index;
        index = after;
        if bytes[index] == b'/' {
            index += 1;
        }
        let name_start = index;
        while index < bytes.len() && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'-')
        {
            index += 1;
        }
        marks.push(Mark {
            start: open,
            end: name_start,
            markup: Markup::Punct,
            name: None,
        });
        marks.push(Mark {
            start: name_start,
            end: index,
            markup: Markup::Tag,
            name: None,
        });
        index = decorate_attributes(template, index, &mut marks);
    }
    marks
}

/// Classify the inside of one tag, from just past its name to just
/// past its `>`. Returns where to resume.
fn decorate_attributes(template: &str, from: usize, marks: &mut Vec<Mark>) -> usize {
    let bytes = template.as_bytes();
    let mut index = from;
    while index < bytes.len() {
        match bytes[index] {
            b'>' => {
                marks.push(Mark {
                    start: index,
                    end: index + 1,
                    markup: Markup::Punct,
                    name: None,
                });
                return index + 1;
            }
            b'/' | b'=' => {
                marks.push(Mark {
                    start: index,
                    end: index + 1,
                    markup: Markup::Punct,
                    name: None,
                });
                index += 1;
            }
            b'"' | b'\'' => {
                let quote = bytes[index];
                let mut end = index + 1;
                while end < bytes.len() && bytes[end] != quote {
                    end += 1;
                }
                let end = (end + 1).min(bytes.len());
                marks.push(Mark {
                    start: index,
                    end,
                    markup: Markup::Value,
                    name: None,
                });
                index = end;
            }
            byte if byte.is_ascii_whitespace() => {
                // Space inside a tag is structure, not character
                // data; leaving it unclassified would colour it as
                // text and make the tag look like prose.
                let start = index;
                while index < bytes.len() && bytes[index].is_ascii_whitespace() {
                    index += 1;
                }
                marks.push(Mark {
                    start,
                    end: index,
                    markup: Markup::Punct,
                    name: None,
                });
            }
            _ => {
                let start = index;
                while index < bytes.len()
                    && !bytes[index].is_ascii_whitespace()
                    && !matches!(bytes[index], b'=' | b'>' | b'/' | b'"' | b'\'')
                {
                    index += 1;
                }
                if index == start {
                    index += 1;
                }
                marks.push(Mark {
                    start,
                    end: index,
                    markup: Markup::Attribute,
                    name: None,
                });
            }
        }
    }
    index
}

/// The authoritative pass: every `{field}` and every command-bound
/// attribute value, from the analyzer's own walk.
fn bindings(template: &str) -> Vec<Mark> {
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
                    markup: Markup::Command,
                    name: Some(command),
                });
                // A command's value is the command, whatever braces it
                // happens to contain; do not also mark inside it.
                return;
            }
            collect_fields(value, value_offset, &mut marks);
        }
    });
    marks
}

/// Every `{field}` in one run, at absolute offsets./// Every `{field}` in one run, at absolute offsets.
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
                markup: Markup::Field,
                name: Some(name.to_owned()),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn of(template: &str, markup: Markup) -> Vec<String> {
        pieces(template)
            .into_iter()
            .filter(|piece| piece.markup == markup)
            .map(|piece| piece.text)
            .collect()
    }

    fn names(template: &str, markup: Markup) -> Vec<String> {
        pieces(template)
            .into_iter()
            .filter(|piece| piece.markup == markup)
            .filter_map(|piece| piece.name)
            .collect()
    }

    #[test]
    fn the_pieces_reassemble_into_the_template() {
        for template in [
            "<p class=\"{kind}\">Hello {name}</p>",
            "<!-- a --><style>.a { color: red }</style><b on:click=\"go\"/>",
            "plain text with no markup at all",
            "<p>caf\u{e9} \u{2014} {name}</p>",
        ] {
            let rebuilt: String = pieces(template)
                .iter()
                .map(Piece::text)
                .collect::<Vec<_>>()
                .concat();
            assert_eq!(rebuilt, template, "lost bytes in: {template}");
        }
    }

    #[test]
    fn it_colours_tags_attributes_and_values() {
        let template = "<p class=\"card\">hi</p>";
        assert_eq!(of(template, Markup::Tag), ["p", "p"]);
        assert_eq!(of(template, Markup::Attribute), ["class"]);
        assert_eq!(of(template, Markup::Value), ["\"card\""]);
        assert_eq!(of(template, Markup::Text), ["hi"]);
    }

    #[test]
    fn a_field_outranks_the_attribute_value_it_sits_in() {
        let template = "<p class=\"{kind}\">x</p>";
        assert_eq!(
            names(template, Markup::Field),
            ["kind"],
            "the authoritative pass paints over the cosmetic one"
        );
    }

    #[test]
    fn it_keeps_every_occurrence_not_just_the_first() {
        assert_eq!(
            names("<p>{name}</p><b>{name}</b>", Markup::Field),
            ["name", "name"],
            "the panel highlights all of them, unlike the diagnostic scan"
        );
    }

    #[test]
    fn a_reference_in_a_comment_is_prose() {
        let template = "<!-- {name} is the title --><p>x</p>";
        assert!(names(template, Markup::Field).is_empty());
        assert_eq!(
            of(template, Markup::Comment),
            ["<!-- {name} is the title -->"]
        );
    }

    #[test]
    fn braces_in_a_style_body_are_css() {
        assert!(names("<style>.a { color: red }</style>", Markup::Field).is_empty());
    }

    #[test]
    fn it_marks_both_binding_forms_as_commands() {
        assert_eq!(
            names(
                "<button on:click=\"space/create\">go</button>",
                Markup::Command
            ),
            ["space/create"]
        );
        assert_eq!(
            names(
                "<button onclick=\"space/create\">go</button>",
                Markup::Command
            ),
            ["space/create"]
        );
    }

    #[test]
    fn a_command_value_is_one_piece_even_with_braces_in_it() {
        let template = "<button on:click=\"cmd/{x}\">go</button>";
        assert_eq!(of(template, Markup::Command), ["cmd/{x}"]);
        assert!(names(template, Markup::Field).is_empty());
    }

    #[test]
    fn it_mirrors_the_renderers_loose_on_prefix_rule() {
        // `preprocess::strip_on_prefix` treats any `on<ascii-alpha>…`
        // attribute as a candidate binding and says so in its own
        // comment. The panel reports what the renderer does, so
        // `once` is marked here too — disagreeing would be a nicer
        // panel and a worse answer.
        assert_eq!(names("<p once=\"yes\">x</p>", Markup::Command), ["yes"]);
        assert!(
            names("<p on-thing=\"yes\">x</p>", Markup::Command).is_empty(),
            "a non-alphabetic char after `on` is not a binding, there or here"
        );
    }

    #[test]
    fn an_empty_or_malformed_reference_is_literal() {
        assert_eq!(names("<p>{} {ok}</p>", Markup::Field), ["ok"]);
    }

    #[test]
    fn an_unterminated_reference_ends_the_scan_of_its_run() {
        assert!(names("<p>{name</p>", Markup::Field).is_empty());
    }

    #[test]
    fn a_stray_less_than_in_prose_is_not_a_tag() {
        assert!(
            of("<p>a < b</p>", Markup::Tag).len() == 2,
            "only the two <p> tags"
        );
    }

    #[test]
    fn an_unterminated_tag_does_not_run_off_the_end() {
        let template = "<div class=\"x";
        let rebuilt: String = pieces(template)
            .iter()
            .map(Piece::text)
            .collect::<Vec<_>>()
            .concat();
        assert_eq!(rebuilt, template);
    }
}
