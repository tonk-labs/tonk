//! Compile-time resource embeds: what a view's templates embed, and
//! where the content comes from.
//!
//! A template embeds content it does not want to inline with
//! `with:href=<name>@<entity>`:
//!
//! ```html
//! <link rel=stylesheet with:href=base@my/concept>
//! ```
//!
//! Both halves are optional. `base` alone reads the enclosing view's
//! own `base` style, and an empty value reads its [`DEFAULT_NAME`]
//! one. So the common case — a view embedding a style it declares
//! itself — is written `with:href=ui`.
//!
//! The attribute must carry a value: the shared template walk reports
//! only attributes that have one, so a bare `with:href` embeds
//! nothing rather than quietly meaning `ui`.
//!
//! The shape mirrors `with="branch@repo"`, which is the same idea one
//! level over: a name resolved against an entity's context. What it
//! does NOT mirror is `on:<name>=<command>`, where the attribute names
//! the declaration and the value names the target — here the whole
//! reference is the value, because an embed has one referent, not two.
//!
//! Resolution happens at lowering: the entity half is captured then,
//! so renaming something else to `my/concept` later cannot change what
//! an already-lowered view embeds. The *content* behind the name stays
//! live — it is an ordinary keyed fact — so editing one style does not
//! disturb the others or require re-lowering.
//!
//! This module is DOM-free: it runs in the analyzer, which has the
//! template text but no browser.

/// The attribute prefix reserved for embeds. `with:` rather than a new
/// word because this IS routing — resolving a name against an entity's
/// context, the same thing `with="branch@repo"` does for a branch.
pub const WITH_PREFIX: &str = "with:";

/// The embed attribute a template writes. Only `href` today; the
/// prefix leaves room for a sibling without changing the scan.
pub const HREF_ATTRIBUTE: &str = "with:href";

/// The style a bare reference reads. Matches `show:`'s primary facet,
/// so a view's default style and its default template are spelled the
/// same.
pub const DEFAULT_NAME: &str = "ui";

/// One `with:href=<name>@<entity>` embed found in a template.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Embed {
    /// The style key, defaulted to [`DEFAULT_NAME`] when the
    /// reference omits it.
    pub name: String,
    /// The entity the key is read from — a bare name
    /// (`my/concept`) or a URI (`tonk:chrome`). `None` means the
    /// reference named none, so it reads the enclosing view's own.
    pub entity: Option<String>,
    /// Byte offset of the attribute name within the template, so a
    /// diagnostic underlines `with:href` rather than the whole
    /// template. A repeated embed keeps the earliest.
    pub offset: usize,
}

impl Embed {
    /// Parse the attribute's value: `name@entity`, `name`, `@entity`,
    /// or empty.
    ///
    /// The `@` splits on the FIRST occurrence so an entity URI
    /// containing one still parses — the name half cannot contain `@`,
    /// the entity half may.
    fn parse(value: &str, offset: usize) -> Self {
        let value = value.trim();
        let (name, entity) = match value.split_once('@') {
            Some((name, entity)) => (name.trim(), Some(entity.trim())),
            None => (value, None),
        };
        Self {
            name: if name.is_empty() {
                DEFAULT_NAME.to_owned()
            } else {
                name.to_owned()
            },
            entity: entity.filter(|e| !e.is_empty()).map(str::to_owned),
            offset,
        }
    }
}

/// Map a template attribute name to the embed it makes.
///
/// Only the exact `with:href` counts. An attribute merely starting
/// with `with:` is left alone so the prefix can grow a sibling later
/// without silently reinterpreting markup written today.
pub fn is_embed_attribute(attribute: &str) -> bool {
    attribute == HREF_ATTRIBUTE
}

/// Every `with:href` embed a template's raw HTML makes, deduplicated,
/// ordered by what the embed says.
///
/// The walk is [`crate::scan::walk`], shared with the interpolation
/// and binding scans so none of the three can disagree about what a
/// template contains. Attributes count even on `<style>` / `<link>`,
/// whose *bodies* the walk refuses to descend into — which is exactly
/// what makes an embed on a `<link>` visible here.
pub fn scan(template: &str) -> Vec<Embed> {
    let mut out: Vec<Embed> = Vec::new();
    crate::scan::walk(template, &mut |found| {
        let crate::scan::Found::Attribute {
            name,
            name_offset,
            value,
            ..
        } = found
        else {
            return;
        };
        if !is_embed_attribute(name) {
            return;
        }
        out.push(Embed::parse(value, name_offset));
    });

    // The derived order puts the offset last, so a repeated embed's
    // occurrences land next to each other and `dedup_by` — which keeps
    // the first of a run — keeps the earliest place it is written.
    out.sort();
    out.dedup_by(|left, right| (&left.name, &left.entity) == (&right.name, &right.entity));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_dedicated_worker);

    fn refs(template: &str) -> Vec<(String, Option<String>)> {
        scan(template)
            .into_iter()
            .map(|e| (e.name, e.entity))
            .collect()
    }

    #[dialog_common::test]
    fn it_reads_both_halves_of_a_full_reference() {
        assert_eq!(
            refs(r#"<link rel=stylesheet with:href=base@my/concept>"#),
            vec![("base".to_owned(), Some("my/concept".to_owned()))],
        );
    }

    /// The common case: a view embedding a style it declares itself.
    #[dialog_common::test]
    fn it_defaults_the_entity_to_the_enclosing_view() {
        assert_eq!(
            refs(r#"<link rel=stylesheet with:href=base>"#),
            vec![("base".to_owned(), None)],
        );
    }

    /// `ui` is the default style for the same reason it is the default
    /// template: a view's primary presentation.
    ///
    /// A VALUELESS `with:href` is not an embed, because the shared
    /// template walk does not report an attribute that has no value —
    /// there is nothing there to interpolate or bind. So the shortest
    /// an embed gets is an empty value, not a bare name.
    #[dialog_common::test]
    fn it_defaults_the_name_when_the_reference_omits_it() {
        assert_eq!(
            refs(r#"<link rel=stylesheet with:href="">"#),
            vec![("ui".to_owned(), None)],
        );
    }

    /// The walk reports only attributes that carry a value, so a bare
    /// `with:href` embeds nothing rather than silently meaning `ui`.
    #[dialog_common::test]
    fn it_does_not_read_a_valueless_attribute_as_an_embed() {
        assert!(refs(r#"<link rel=stylesheet with:href>"#).is_empty());
    }

    /// Another view's default style.
    #[dialog_common::test]
    fn it_reads_an_entity_with_no_name() {
        assert_eq!(
            refs(r#"<link rel=stylesheet with:href=@my/concept>"#),
            vec![("ui".to_owned(), Some("my/concept".to_owned()))],
        );
    }

    /// An entity URI contains a `:` and may contain an `@`; the first
    /// `@` is the separator, so the rest survives intact.
    #[dialog_common::test]
    fn it_splits_on_the_first_at_only() {
        assert_eq!(
            refs(r#"<link with:href=base@did:key:z6Mk@x>"#),
            vec![("base".to_owned(), Some("did:key:z6Mk@x".to_owned()))],
        );
    }

    /// The prefix is reserved exactly, not by prefix match, so a
    /// sibling attribute added later cannot silently reinterpret
    /// markup written today.
    #[dialog_common::test]
    fn it_leaves_other_prefixed_attributes_alone() {
        assert!(refs(r##"<link with:rel=x xlink:href="#y" on:click=go>"##).is_empty());
    }

    #[dialog_common::test]
    fn it_ignores_an_embed_named_inside_a_comment() {
        let found = refs("<!-- with:href=gone --><link with:href=kept>");
        assert_eq!(found, vec![("kept".to_owned(), None)]);
    }

    /// The offset points at the attribute name, which is what a
    /// diagnostic underlines. A repeated embed keeps the first.
    #[dialog_common::test]
    fn it_records_where_each_attribute_is_written() {
        let template = "<p>hi</p>\n<link with:href=base>\n<link with:href=base>";
        let found = scan(template);
        assert_eq!(found.len(), 1, "the repeat is one embed, not two");
        assert_eq!(
            &template[found[0].offset..found[0].offset + HREF_ATTRIBUTE.len()],
            HREF_ATTRIBUTE,
        );
        assert_eq!(
            found[0].offset,
            template.find(HREF_ATTRIBUTE).expect("it is in there"),
        );
    }

    /// A stylesheet body is not scanned, but the attributes of the
    /// element carrying it are — which is the whole reason an embed on
    /// a `<link>` or `<style>` is findable.
    #[dialog_common::test]
    fn it_finds_an_embed_on_an_element_whose_body_is_not_scanned() {
        assert_eq!(
            refs("<style with:href=base>p { color: red }</style>"),
            vec![("base".to_owned(), None)],
        );
    }
}
