//! Compile-time resource embeds: what a view's templates embed, and
//! where the content comes from.
//!
//! A template embeds content it does not want to inline with
//! `with:src=<name>@<entity>`:
//!
//! ```html
//! <link rel=stylesheet with:src=base@my/concept>
//! ```
//!
//! Both halves are optional. `base` alone reads the enclosing view's
//! own `base` style, and an empty value reads its [`DEFAULT_NAME`]
//! one. So the common case — a view embedding a style it declares
//! itself — is written `with:src=ui`.
//!
//! The attribute must carry a value: the shared template walk reports
//! only attributes that have one, so a bare `with:src` embeds
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

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The attribute prefix reserved for embeds. `with:` rather than a new
/// word because this IS routing — resolving a name against an entity's
/// context, the same thing `with="branch@repo"` does for a branch.
pub const WITH_PREFIX: &str = "with:";

/// The embed attribute a template writes.
///
/// `src` because that is what CSS calls it. An `@font-face` rule names
/// its family and then points at the bytes with `src:`, and
/// `<font-family name="Body" with:src=body>` is that rule written as
/// markup — same two parts, same words. A style embed follows the font
/// one rather than splitting the vocabulary in two.
///
/// Only `src` today; the prefix leaves room for a sibling without
/// changing the scan.
pub const SRC_ATTRIBUTE: &str = "with:src";

/// The style a bare reference reads. Matches `show:`'s primary facet,
/// so a view's default style and its default template are spelled the
/// same.
pub const DEFAULT_NAME: &str = "ui";

/// One `with:src=<name>@<entity>` embed found in a template.
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
    /// diagnostic underlines `with:src` rather than the whole
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
/// Only the exact `with:src` counts. An attribute merely starting
/// with `with:` is left alone so the prefix can grow a sibling later
/// without silently reinterpreting markup written today.
pub fn is_embed_attribute(attribute: &str) -> bool {
    attribute == SRC_ATTRIBUTE
}

/// Every `with:src` embed a template's raw HTML makes, deduplicated,
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

/// What every embeds artifact says it is, in its own `kind` field.
///
/// Same reasoning as [`crate::bindings::KIND`]: the `Record` storage
/// type says "structured", not *which* structure, and flattens to
/// plain bytes on every wire projection — so the payload names its own
/// format and a decoder handed some other CBOR fails loudly instead of
/// quietly reading an empty table.
pub const EMBEDS_KIND: &str = "tonk/view-embeds@1";

/// One embed with its subject resolved.
///
/// The reference as written says `ui` or `ui@space`; what the renderer
/// needs is the pair actually to ask for. Capturing it at lowering is
/// what makes the analyzer's check and the runtime's query the same
/// question — while the reference was re-read at render time, the two
/// could disagree about the subject and nothing would say so.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ResolvedEmbed {
    /// The entity the content is read from, resolved: the view's own
    /// subject for a bare reference, the named one otherwise.
    pub entity: String,
    /// The key within that view's `style:` / `font:` map.
    pub name: String,
}

/// The embeds a view's templates make, resolved at lowering.
///
/// A struct rather than a bare map for the same reason
/// [`crate::bindings::Bindings`] is one: the artifact can grow a field
/// without a format break. Encoded as dag-cbor, which is canonical, so
/// re-lowering an unchanged view yields byte-identical output and the
/// claim does not churn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Embeds {
    /// Always [`EMBEDS_KIND`]. Private so it cannot be set to anything
    /// else; checked on decode.
    kind: String,
    /// The reference as written (`ui`, `ui@space`) -> the pair it
    /// resolved to. Keyed by the written form so the renderer can look
    /// up exactly what a template says without re-parsing it.
    pub embeds: BTreeMap<String, ResolvedEmbed>,
}

impl Default for Embeds {
    fn default() -> Self {
        Self::new(BTreeMap::new())
    }
}

impl Embeds {
    /// An artifact carrying `embeds`.
    pub fn new(embeds: BTreeMap<String, ResolvedEmbed>) -> Self {
        Self {
            kind: EMBEDS_KIND.to_owned(),
            embeds,
        }
    }

    /// True when there is nothing to store.
    pub fn is_empty(&self) -> bool {
        self.embeds.is_empty()
    }

    /// Encode as dag-cbor.
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        serde_ipld_dagcbor::to_vec(self).map_err(|error| error.to_string())
    }

    /// Decode from dag-cbor, refusing anything that is not this format.
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let decoded: Self =
            serde_ipld_dagcbor::from_slice(bytes).map_err(|error| error.to_string())?;
        if decoded.kind != EMBEDS_KIND {
            return Err(format!(
                "expected `{EMBEDS_KIND}`, got `{}`",
                decoded.kind.escape_debug()
            ));
        }
        Ok(decoded)
    }
}

impl Embed {
    /// How this embed is written in the template — the key an
    /// [`Embeds`] artifact stores it under.
    pub fn reference(&self) -> String {
        match &self.entity {
            Some(entity) => format!("{}@{entity}", self.name),
            None => self.name.clone(),
        }
    }
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
            refs(r#"<link rel=stylesheet with:src=base@my/concept>"#),
            vec![("base".to_owned(), Some("my/concept".to_owned()))],
        );
    }

    /// The common case: a view embedding a style it declares itself.
    #[dialog_common::test]
    fn it_defaults_the_entity_to_the_enclosing_view() {
        assert_eq!(
            refs(r#"<link rel=stylesheet with:src=base>"#),
            vec![("base".to_owned(), None)],
        );
    }

    /// `ui` is the default style for the same reason it is the default
    /// template: a view's primary presentation.
    ///
    /// A VALUELESS `with:src` is not an embed, because the shared
    /// template walk does not report an attribute that has no value —
    /// there is nothing there to interpolate or bind. So the shortest
    /// an embed gets is an empty value, not a bare name.
    #[dialog_common::test]
    fn it_defaults_the_name_when_the_reference_omits_it() {
        assert_eq!(
            refs(r#"<link rel=stylesheet with:src="">"#),
            vec![("ui".to_owned(), None)],
        );
    }

    /// The walk reports only attributes that carry a value, so a bare
    /// `with:src` embeds nothing rather than silently meaning `ui`.
    #[dialog_common::test]
    fn it_does_not_read_a_valueless_attribute_as_an_embed() {
        assert!(refs(r#"<link rel=stylesheet with:src>"#).is_empty());
    }

    /// Another view's default style.
    #[dialog_common::test]
    fn it_reads_an_entity_with_no_name() {
        assert_eq!(
            refs(r#"<link rel=stylesheet with:src=@my/concept>"#),
            vec![("ui".to_owned(), Some("my/concept".to_owned()))],
        );
    }

    /// An entity URI contains a `:` and may contain an `@`; the first
    /// `@` is the separator, so the rest survives intact.
    #[dialog_common::test]
    fn it_splits_on_the_first_at_only() {
        assert_eq!(
            refs(r#"<link with:src=base@did:key:z6Mk@x>"#),
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
        let found = refs("<!-- with:src=gone --><link with:src=kept>");
        assert_eq!(found, vec![("kept".to_owned(), None)]);
    }

    /// The offset points at the attribute name, which is what a
    /// diagnostic underlines. A repeated embed keeps the first.
    #[dialog_common::test]
    fn it_records_where_each_attribute_is_written() {
        let template = "<p>hi</p>\n<link with:src=base>\n<link with:src=base>";
        let found = scan(template);
        assert_eq!(found.len(), 1, "the repeat is one embed, not two");
        assert_eq!(
            &template[found[0].offset..found[0].offset + SRC_ATTRIBUTE.len()],
            SRC_ATTRIBUTE,
        );
        assert_eq!(
            found[0].offset,
            template.find(SRC_ATTRIBUTE).expect("it is in there"),
        );
    }

    /// The written form round-trips: what [`scan`] reports is what
    /// [`Embed::reference`] spells, which is the key an [`Embeds`]
    /// artifact stores the resolved pair under.
    ///
    /// The renderer looks the pair up by exactly this string, so a
    /// quoted attribute value that scanned differently than it was
    /// stored would find nothing — and an embed that finds nothing is
    /// silently unstyled, which is the failure mode this whole path
    /// exists to remove.
    #[dialog_common::test]
    fn the_written_reference_round_trips() {
        for (template, expected) in [
            (r#"<link rel="stylesheet" with:src="ui@space">"#, "ui@space"),
            (r#"<link rel=stylesheet with:src=ui@space>"#, "ui@space"),
            (r#"<link rel="stylesheet" with:src="ui">"#, "ui"),
            (r#"<link with:src=" ui@space ">"#, "ui@space"),
            (r#"<link with:src="">"#, DEFAULT_NAME),
        ] {
            let found = scan(template);
            assert_eq!(found.len(), 1, "one embed in {template}");
            assert_eq!(
                found[0].reference(),
                expected,
                "the reference {template} spells",
            );
        }
    }

    /// The artifact round-trips, and re-encoding is byte-identical so a
    /// re-lowered view does not churn its claim.
    #[dialog_common::test]
    fn the_artifact_round_trips_byte_identically() {
        let embeds = Embeds::new(BTreeMap::from([
            (
                "ui".to_string(),
                ResolvedEmbed {
                    entity: "tonk:space".into(),
                    name: "ui".into(),
                },
            ),
            (
                "base@other".to_string(),
                ResolvedEmbed {
                    entity: "tonk:other".into(),
                    name: "base".into(),
                },
            ),
        ]));
        let encoded = embeds.encode().expect("encodes");
        let decoded = Embeds::decode(&encoded).expect("decodes");
        assert_eq!(decoded, embeds);
        assert_eq!(
            decoded.encode().expect("re-encodes"),
            encoded,
            "dag-cbor is canonical, so an unchanged view yields identical bytes",
        );
    }

    /// Bytes that are not this artifact are REFUSED, not read as an
    /// empty table.
    ///
    /// An empty table would be read as authoritative — "this view
    /// embeds nothing" — which is a different claim from "I could not
    /// read this". The `kind` field exists to tell them apart, because
    /// the `Record` storage tag says only "structured".
    #[dialog_common::test]
    fn the_artifact_refuses_anything_that_is_not_itself() {
        assert!(
            Embeds::decode(b"not cbor at all").is_err(),
            "arbitrary bytes are not an artifact",
        );

        // Valid dag-cbor of the WRONG kind: the bindings artifact.
        // Refused on shape here — it has no `embeds` field to read —
        // which is refusal all the same.
        let foreign = crate::bindings::Bindings::default()
            .encode()
            .expect("the sibling artifact encodes");
        assert!(
            Embeds::decode(&foreign).is_err(),
            "a sibling artifact is not this one",
        );
    }

    /// The `kind` guard refuses a payload of the right SHAPE but the
    /// wrong format.
    ///
    /// This is the case the storage tag cannot catch: a `Record` says
    /// "structured", not which structure, and it flattens to plain
    /// bytes on every wire projection. A future format change has to be
    /// a version bump rather than a silent misparse, and this is what
    /// makes that true.
    #[dialog_common::test]
    fn the_artifact_refuses_a_lookalike_of_another_version() {
        #[derive(serde::Serialize)]
        struct Lookalike {
            kind: String,
            embeds: BTreeMap<String, ResolvedEmbed>,
        }
        let lookalike = Lookalike {
            kind: "tonk/view-embeds@2".to_owned(),
            embeds: BTreeMap::from([(
                "ui".to_string(),
                ResolvedEmbed {
                    entity: "tonk:demo".into(),
                    name: "ui".into(),
                },
            )]),
        };
        let encoded = serde_ipld_dagcbor::to_vec(&lookalike).expect("the lookalike encodes");
        let error = Embeds::decode(&encoded).expect_err("a later version is refused");
        assert!(
            error.contains(EMBEDS_KIND) && error.contains("tonk/view-embeds@2"),
            "the error names both formats: {error}",
        );
    }

    /// An empty artifact is still a well-formed one — the analyzer
    /// omits it rather than storing it, but nothing here forbids it.
    #[dialog_common::test]
    fn an_empty_artifact_is_well_formed_and_says_so() {
        let empty = Embeds::default();
        assert!(empty.is_empty());
        let decoded = Embeds::decode(&empty.encode().expect("encodes")).expect("decodes");
        assert!(decoded.is_empty());
        assert!(
            !Embeds::new(BTreeMap::from([(
                "ui".to_string(),
                ResolvedEmbed {
                    entity: "tonk:demo".into(),
                    name: "ui".into(),
                },
            )]))
            .is_empty()
        );
    }

    /// A stylesheet body is not scanned, but the attributes of the
    /// element carrying it are — which is the whole reason an embed on
    /// a `<link>` or `<style>` is findable.
    #[dialog_common::test]
    fn it_finds_an_embed_on_an_element_whose_body_is_not_scanned() {
        assert_eq!(
            refs("<style with:src=base>p { color: red }</style>"),
            vec![("base".to_owned(), None)],
        );
    }
}
