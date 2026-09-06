//! The `{field}` references a template makes, found without a DOM.
//!
//! A template interpolates `{name}` and the renderer resolves it
//! against the row it is rendering: `{this}` is the subject, a
//! `{dom.host/attr}` is copied off the outer host element, and
//! anything else is a field of the model's conclusion. A name that
//! resolves to none of those renders as **nothing** — no error, no
//! placeholder, just a gap where the value should be. That is the same
//! failure an unresolvable `on:` binding used to have, on the other
//! half of the template, and it is caught the same way: find the
//! references here, check them against the model's declared fields in
//! the analyzer.
//!
//! This module is only the finding. What counts as a legal name
//! depends on the concept the view renders, which lives in the
//! analyzer's scope, so the check itself does too.

use crate::scan::{Found, walk};

/// The namespace of host-element attributes copied into the
/// conclusion (`{dom.host/model}`). These describe the *outer* host,
/// not the subject, so no concept declares them and nothing here can
/// check them.
pub const HOST_NAMESPACE: &str = "dom.host/";

/// The subject itself, legal in any template.
pub const THIS: &str = "this";

/// The suffix that names a keyed collection entry's key
/// (`{block/key}` beside `{block}`). The renderer inserts it into the
/// per-iteration shadow alongside the field, so it is legal wherever
/// its field is.
pub const KEY_SUFFIX: &str = "/key";

/// One `{field}` reference, with where it is written.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FieldReference {
    /// The name between the braces, verbatim.
    pub name: String,
    /// Byte offset of the `{` in the template. A repeated reference
    /// keeps the earliest — the report has to point somewhere, and
    /// that is the one an author reads first.
    pub offset: usize,
}

impl FieldReference {
    /// The name with a `/key` suffix removed, which is what a concept
    /// would declare. `{block/key}` is legal exactly when `block` is.
    pub fn declared_name(&self) -> &str {
        self.name.strip_suffix(KEY_SUFFIX).unwrap_or(&self.name)
    }

    /// True for a reference no concept can account for: the subject,
    /// or a host attribute.
    pub fn is_ambient(&self) -> bool {
        self.name == THIS || self.name.starts_with(HOST_NAMESPACE)
    }

    /// The `{name}` as written, for a diagnostic that quotes it.
    pub fn written(&self) -> String {
        format!("{{{}}}", self.name)
    }

    /// How many bytes the reference occupies, braces included.
    pub fn len(&self) -> usize {
        self.name.len() + 2
    }

    /// Never — a reference always spans at least `{}`. Present because
    /// clippy asks for it beside [`FieldReference::len`].
    pub fn is_empty(&self) -> bool {
        false
    }
}

/// Every `{field}` reference a template makes, deduplicated by name,
/// each carrying the earliest offset it appears at.
///
/// Only text runs and attribute values are read, and `<style>` /
/// `<script>` bodies are not — see [`crate::scan`]. Without that last
/// exclusion every CSS rule block in the library reads as a wall of
/// undefined fields, which is exactly what makes a naive version of
/// this check unusable.
pub fn scan(template: &str) -> Vec<FieldReference> {
    let mut out: Vec<FieldReference> = Vec::new();
    walk(template, &mut |found| {
        let (text, base) = match found {
            Found::Text { text, offset } => (text, offset),
            Found::Attribute {
                value,
                value_offset,
                ..
            } => (value, value_offset),
        };
        collect(text, base, &mut out);
    });

    // Ordered by name so a report does not shift when a template
    // moves; the offset only breaks ties, keeping the earliest.
    out.sort();
    out.dedup_by(|left, right| left.name == right.name);
    out
}

/// Push every `{...}` in one run of text.
///
/// Mirrors `parse_segments`: the first `}` closes, and an unterminated
/// `{` is literal text rather than a reference.
fn collect(text: &str, base: usize, out: &mut Vec<FieldReference>) {
    let mut rest = text;
    let mut consumed = 0usize;
    while let Some(open) = rest.find('{') {
        let after = open + 1;
        let Some(close) = rest[after..].find('}') else {
            return;
        };
        out.push(FieldReference {
            name: rest[after..after + close].to_owned(),
            offset: base + consumed + open,
        });
        let next = after + close + 1;
        consumed += next;
        rest = &rest[next..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_dedicated_worker);

    fn names(template: &str) -> Vec<String> {
        scan(template).into_iter().map(|f| f.name).collect()
    }

    #[dialog_common::test]
    fn it_finds_references_in_text_and_attributes() {
        assert_eq!(
            names(r#"<a href="/x/{this}" title={name}>{title}</a>"#),
            vec!["name", "this", "title"],
        );
    }

    /// The exclusion the whole check depends on. A rule block is not
    /// a field reference, and there are hundreds of them.
    #[dialog_common::test]
    fn it_ignores_css_and_script_braces() {
        let template = concat!(
            "<style>p { color: red; }</style>",
            "<script>if (x) { go(); }</script>",
            "<p>{title}</p>",
        );
        assert_eq!(names(template), vec!["title"]);
    }

    #[dialog_common::test]
    fn it_ignores_a_reference_inside_a_comment() {
        assert_eq!(names("<!-- {gone} --><p>{kept}</p>"), vec!["kept"]);
    }

    #[dialog_common::test]
    fn it_keeps_the_earliest_offset_of_a_repeated_reference() {
        let template = "<p>{title}</p><h1>{title}</h1>";
        let found = scan(template);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].offset, template.find("{title}").expect("present"));
        assert_eq!(
            &template[found[0].offset..found[0].offset + found[0].len()],
            "{title}",
        );
    }

    #[dialog_common::test]
    fn an_unterminated_brace_is_not_a_reference() {
        assert!(names("<p>a {b</p>").is_empty());
    }

    #[dialog_common::test]
    fn a_key_reference_is_declared_by_its_field() {
        let found = scan("<p>{block/key}</p>");
        assert_eq!(found[0].declared_name(), "block");
        assert!(!found[0].is_ambient());
    }

    #[dialog_common::test]
    fn the_subject_and_host_attributes_are_ambient() {
        for template in ["<p>{this}</p>", "<p>{dom.host/model}</p>"] {
            assert!(scan(template)[0].is_ambient(), "{template}");
        }
    }
}
