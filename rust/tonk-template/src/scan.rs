//! One template lexer, shared by everything that reads a `show:`
//! template without a DOM.
//!
//! The analyzer has no DOM, so anything it wants to know about a
//! template it has to read from the text. Two passes want to: the
//! event-binding scan ([`crate::bindings::scan`]) reads `on:`
//! attributes, and the interpolation scan ([`crate::fields::scan`])
//! reads `{field}` references out of text and attribute values.
//!
//! They read the same template and must agree with the *browser* about
//! what is in it, so they share one walk rather than each carrying
//! their own. The two rules that matter are the two the renderer's DOM
//! walk applies:
//!
//! * An HTML comment carries nothing — the parser drops its content,
//!   so a binding or a `{field}` mentioned in prose inside `<!-- -->`
//!   is not one.
//! * `<style>` and `<script>` content is verbatim CSS/JS, where `{ … }`
//!   are real braces. The renderer refuses to descend into them
//!   (`is_raw_text_element` in `tonk-display`), so neither does this.
//!   Their *attributes* still count: the renderer visits the element
//!   before deciding not to descend.
//!
//! Getting either wrong is not a subtle difference. Without the first,
//! a documented example fails the build; without the second, every
//! stylesheet in the library becomes a wall of undefined fields.

/// Something the walk found, with its byte offset in the template so a
/// diagnostic can point at it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found<'a> {
    /// An attribute inside a tag. `value` is the unquoted value.
    Attribute {
        /// The attribute name as written (`on:click`, `href`).
        name: &'a str,
        /// Byte offset of the name.
        name_offset: usize,
        /// The value with any quotes stripped.
        value: &'a str,
        /// Byte offset of the value's first character.
        value_offset: usize,
    },
    /// A run of character data between tags.
    Text {
        /// The run, verbatim.
        text: &'a str,
        /// Byte offset of the run.
        offset: usize,
    },
}

/// Walk a template, reporting every attribute and every text run.
///
/// A text scan rather than a parse: it does not build a tree, because
/// no caller needs one. What it does reproduce is the renderer's view
/// of which bytes are markup — see the module docs for the two
/// exclusions that matter.
pub fn walk(template: &str, visit: &mut impl FnMut(Found<'_>)) {
    let bytes = template.as_bytes();
    let mut index = 0usize;
    let mut text_start = 0usize;

    while index < bytes.len() {
        if bytes[index] != b'<' {
            index += 1;
            continue;
        }
        if template[index..].starts_with("<!--") {
            emit_text(template, text_start, index, visit);
            index = match template[index + 4..].find("-->") {
                Some(offset) => index + 4 + offset + 3,
                None => bytes.len(),
            };
            text_start = index;
            continue;
        }
        // A `<` that opens no tag (a stray less-than in prose) is not
        // a tag start; only a name or a closing slash follows one.
        let after = index + 1;
        if after >= bytes.len() || !(bytes[after].is_ascii_alphabetic() || bytes[after] == b'/') {
            index += 1;
            continue;
        }
        emit_text(template, text_start, index, visit);
        let (end, raw_text_name) = scan_tag(template, after, visit);
        index = end;
        // A `<style>` / `<script>` body is verbatim, so skip to the
        // matching close tag without reporting the text between.
        if let Some(name) = raw_text_name {
            index = skip_raw_text(template, index, name);
        }
        text_start = index;
    }
    emit_text(template, text_start, bytes.len(), visit);
}

/// Report the text between two offsets, if there is any.
fn emit_text(template: &str, start: usize, end: usize, visit: &mut impl FnMut(Found<'_>)) {
    if start < end {
        visit(Found::Text {
            text: &template[start..end],
            offset: start,
        });
    }
}

/// Read one tag's attributes starting just after its `<`.
///
/// Returns the offset just past the tag's `>` (or the end of input),
/// and the lowercase name of a raw-text element whose body the caller
/// must skip.
fn scan_tag(
    template: &str,
    mut index: usize,
    visit: &mut impl FnMut(Found<'_>),
) -> (usize, Option<&'static str>) {
    let bytes = template.as_bytes();
    let closing = bytes[index] == b'/';
    let name_start = index;
    while index < bytes.len() && !bytes[index].is_ascii_whitespace() && bytes[index] != b'>' {
        index += 1;
    }
    let tag_name = &template[name_start..index];
    let raw_text = match tag_name.to_ascii_lowercase().as_str() {
        "style" if !closing => Some("style"),
        "script" if !closing => Some("script"),
        _ => None,
    };

    loop {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() {
            return (bytes.len(), raw_text);
        }
        if bytes[index] == b'>' {
            return (index + 1, raw_text);
        }
        // A self-closing `/` before `>` is not an attribute name; nor
        // is there a body to skip after one.
        if bytes[index] == b'/' {
            index += 1;
            return (scan_to_tag_end(template, index), None);
        }

        let name_start = index;
        while index < bytes.len()
            && !bytes[index].is_ascii_whitespace()
            && !matches!(bytes[index], b'=' | b'>' | b'/')
        {
            index += 1;
        }
        let name = &template[name_start..index];

        let mut probe = index;
        while probe < bytes.len() && bytes[probe].is_ascii_whitespace() {
            probe += 1;
        }
        if probe >= bytes.len() || bytes[probe] != b'=' {
            // A valueless attribute — nothing to interpolate or bind.
            continue;
        }
        index = probe + 1;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() {
            return (bytes.len(), raw_text);
        }

        let value_start;
        let value_end;
        match bytes[index] {
            quote @ (b'"' | b'\'') => {
                index += 1;
                value_start = index;
                while index < bytes.len() && bytes[index] != quote {
                    index += 1;
                }
                value_end = index;
                index = (index + 1).min(bytes.len());
            }
            _ => {
                value_start = index;
                while index < bytes.len()
                    && !bytes[index].is_ascii_whitespace()
                    && bytes[index] != b'>'
                {
                    index += 1;
                }
                value_end = index;
            }
        }

        visit(Found::Attribute {
            name,
            name_offset: name_start,
            value: &template[value_start..value_end],
            value_offset: value_start,
        });
    }
}

/// The offset just past the next `>`, or the end of input.
fn scan_to_tag_end(template: &str, index: usize) -> usize {
    match template[index..].find('>') {
        Some(offset) => index + offset + 1,
        None => template.len(),
    }
}

/// The offset just past `</name>`, or the end of input. Case-insensitive
/// on the tag name, as HTML is.
fn skip_raw_text(template: &str, index: usize, name: &str) -> usize {
    let haystack = template[index..].to_ascii_lowercase();
    let needle = format!("</{name}");
    match haystack.find(&needle) {
        Some(offset) => scan_to_tag_end(template, index + offset),
        None => template.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_dedicated_worker);

    /// Every attribute and text run, as `(name_or_text, offset)`.
    fn collect(template: &str) -> Vec<(String, usize)> {
        let mut out = Vec::new();
        walk(template, &mut |found| match found {
            Found::Attribute {
                name,
                name_offset,
                value,
                ..
            } => out.push((format!("@{name}={value}"), name_offset)),
            Found::Text { text, offset } => out.push((format!("#{text}"), offset)),
        });
        out
    }

    #[dialog_common::test]
    fn it_reports_text_and_attributes_with_their_offsets() {
        let template = r#"<a href="/x/{this}">go</a>"#;
        assert_eq!(
            collect(template),
            vec![("@href=/x/{this}".to_string(), 3), ("#go".to_string(), 20),],
        );
    }

    /// A stylesheet's braces are CSS, not fields. The renderer refuses
    /// to descend into `<style>`; so does this.
    #[dialog_common::test]
    fn it_does_not_read_a_style_body() {
        let found = collect("<style>p { color: red; }</style><p>hi</p>");
        assert_eq!(found, vec![("#hi".to_string(), 35)]);
    }

    #[dialog_common::test]
    fn it_does_not_read_a_script_body() {
        let found = collect("<script>if (x) { go({y}); }</script><b>hi</b>");
        assert_eq!(found, vec![("#hi".to_string(), 39)]);
    }

    /// A raw-text element's own attributes still count: the renderer
    /// visits the element before deciding not to walk into it.
    #[dialog_common::test]
    fn it_still_reads_a_raw_text_elements_attributes() {
        let found = collect(r#"<script src="{module}"></script>"#);
        assert_eq!(found, vec![("@src={module}".to_string(), 8)]);
    }

    #[dialog_common::test]
    fn it_reports_nothing_inside_a_comment() {
        assert_eq!(
            collect("<!-- {gone} --><i>{kept}</i>"),
            vec![("#{kept}".to_string(), 18)]
        );
    }

    #[dialog_common::test]
    fn it_reads_a_self_closing_tags_attributes() {
        let found = collect(r#"<img src={a} /><p>x</p>"#);
        assert_eq!(
            found,
            vec![("@src={a}".to_string(), 5), ("#x".to_string(), 18)],
        );
    }
}
