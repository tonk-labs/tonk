//! Pure notation builders for the noun-first authoring verbs
//! (`tonk concept add`, `tonk view add`, `tonk home`). No I/O — every
//! function here takes already-parsed arguments and returns (or
//! errors on) a `String` of asserted notation; callers (later tasks)
//! own reading CLI flags and handing the result to
//! `eval::run_against_site`.
//!
//! `build_home_recipe` in particular is not a free design — it
//! reproduces the verified shape from
//! `.superpowers/sdd/repoint-findings.md` §"THE MINIMAL WORKING
//! RECIPE". That document records a from-scratch investigation into
//! why a home page renders (or silently doesn't): the root concept's
//! one `with:` field must map to `the: dialog.origin/subject` /
//! `as: entity` — the only attribute guaranteed already-asserted on
//! the entity the space-home route renders — and the inline
//! attribute under `with:` must carry its own `description:` (a hard
//! analyzer requirement, independent of the concept's own
//! description). Deviating from the recipe reproduces the "blank
//! canvas" or "Concept mismatch" failures documented there.

use std::fmt::Write as _;

/// One parsed `--attr field:type:cardinality` flag, ready to render
/// into an `attribute!:` block and a `with:` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttrSpec {
    /// Field name — the key under the owning concept's `with:` map.
    pub field: String,
    /// Canonical `as:` type spelling (e.g. `Text`, `UnsignedInteger`),
    /// matched case-insensitively from the raw input.
    pub type_name: String,
    /// Cardinality: `one` or `many`.
    pub cardinality: String,
}

/// Error parsing a raw `--attr` flag or building notation from it.
#[derive(Debug, thiserror::Error)]
pub enum AuthoringError {
    /// The raw value didn't split into exactly three `:`-separated
    /// parts (`<field>:<type>:<cardinality>`).
    #[error(
        "--attr '{raw}' is malformed; expected <field>:<type>:<cardinality>, e.g. title:text:one"
    )]
    BadAttrSpec {
        /// The raw, unparsed `--attr` value.
        raw: String,
    },
    /// The type segment isn't one of the canonical spellings the
    /// analyzer accepts.
    #[error("unknown type '{raw}'; valid types: {}", valid.join(", "))]
    BadType {
        /// The offending type segment, as given.
        raw: String,
        /// The canonical type names that would have been accepted.
        valid: Vec<&'static str>,
    },
    /// The cardinality segment wasn't `one` or `many`.
    #[error("unknown cardinality '{raw}'; valid: one, many")]
    BadCardinality {
        /// The offending cardinality segment, as given.
        raw: String,
    },
    /// A view template was empty — neither `--template` nor
    /// `--template-file` supplied any content. No builder in this
    /// module produces this variant directly; it exists for the CLI
    /// wiring (a later task) to construct once it has resolved the
    /// template source and found it empty.
    #[error("the view template is empty; pass --template <html> or --template-file <path>")]
    EmptyTemplate,
}

/// Canonical `as:` type spellings the analyzer accepts, matching
/// `schema::type_to_notation`'s output (which `tonk schema` proves
/// re-submittable). Input is matched case-insensitively.
const VALID_TYPES: &[&str] = &[
    "Text",
    "Entity",
    "UnsignedInteger",
    "SignedInteger",
    "Float",
    "Boolean",
    "Symbol",
];

/// Parse a raw `--attr field:type:cardinality` flag value into an
/// [`AttrSpec`]. The type segment is matched case-insensitively
/// against [`VALID_TYPES`] and normalized to its canonical spelling;
/// the cardinality segment must be exactly `one` or `many`.
pub fn parse_attr_spec(raw: &str) -> Result<AttrSpec, AuthoringError> {
    let parts: Vec<&str> = raw.split(':').collect();
    let [field, ty, card] = parts.as_slice() else {
        return Err(AuthoringError::BadAttrSpec { raw: raw.into() });
    };
    let type_name = VALID_TYPES
        .iter()
        .find(|t| t.eq_ignore_ascii_case(ty))
        .ok_or_else(|| AuthoringError::BadType {
            raw: (*ty).into(),
            valid: VALID_TYPES.to_vec(),
        })?;
    if !matches!(*card, "one" | "many") {
        return Err(AuthoringError::BadCardinality {
            raw: (*card).into(),
        });
    }
    Ok(AttrSpec {
        field: (*field).into(),
        type_name: (*type_name).into(),
        cardinality: (*card).into(),
    })
}

/// Double-quote and escape a string for notation. Deliberately
/// duplicated from the private `quote_string` helpers in `data.rs`
/// and `schema.rs` rather than shared — each module keeps its own
/// copy to avoid widening any of their APIs for a five-line helper.
fn quote_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// Build a `concept!:` declaration document: one standalone
/// `attribute!:` block per attr (named `&{name}-{field}`), followed
/// by a `concept!: &{name}` block whose `with:` map references each
/// attribute by name. `description` defaults to `"A {name}."` when
/// `None` — the analyzer treats a concept description as optional,
/// but schema-aware help reads it, so every generated concept gets
/// one either way.
pub fn build_concept_decl(name: &str, description: Option<&str>, attrs: &[AttrSpec]) -> String {
    let mut out = String::new();
    for attr in attrs {
        let AttrSpec {
            field,
            type_name,
            cardinality,
        } = attr;
        let _ = writeln!(out, "attribute!: &{name}-{field}");
        let _ = writeln!(
            out,
            "  description: {}",
            quote_string(&format!("The {field} field of {name}."))
        );
        let _ = writeln!(out, "  the:         xyz.tonk.{name}/{field}");
        let _ = writeln!(out, "  as:          {type_name}");
        let _ = writeln!(out, "  cardinality: {cardinality}");
        out.push('\n');
    }
    let _ = writeln!(out, "concept!: &{name}");
    let description = description.map_or_else(|| format!("A {name}."), str::to_string);
    let _ = writeln!(out, "  description: {}", quote_string(&description));
    out.push_str("  with:\n");
    for attr in attrs {
        let _ = writeln!(out, "    {field}: {name}-{field}", field = attr.field);
    }
    out
}

/// Build a `view!:` declaration document with a stable, name-derived
/// `this:` (`id:{anchor}`) so re-authoring the same view (same
/// anchor) supersedes rather than duplicates it. `template` is
/// emitted verbatim, line by line, under a `display: |` block
/// indented four spaces.
pub fn build_view_decl(anchor: &str, model: &str, template: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "view!: &{anchor}");
    let _ = writeln!(out, "  this: id:{anchor}");
    let _ = writeln!(out, "  model: {model}");
    out.push_str("  display: |\n");
    for line in template.lines() {
        let _ = writeln!(out, "    {line}");
    }
    out
}

/// Build the space-home recipe: the origin-keyed root concept, its
/// view (one `<tonk-display model=X />` per model — wrapped in a
/// `<section>` with an `<h2>` heading when there are 2+ models, a
/// bare tag when there's exactly one), and the `name!:` repoint of
/// `id:tonk/space` onto it.
///
/// This is the exact, verified shape from
/// `.superpowers/sdd/repoint-findings.md` §"THE MINIMAL WORKING
/// RECIPE" — see the module doc for why each piece is load-bearing.
pub fn build_home_recipe(models: &[String]) -> String {
    let mut out = String::new();

    out.push_str("concept!: &space-home\n");
    out.push_str("  this: space:home\n");
    let _ = writeln!(
        out,
        "  description: {}",
        quote_string("The space home page, keyed by the repository's own subject DID.")
    );
    out.push_str("  with:\n");
    out.push_str("    subject:\n");
    let _ = writeln!(
        out,
        "      description: {}",
        quote_string("The repository's subject DID.")
    );
    out.push_str("      the: dialog.origin/subject\n");
    out.push_str("      as: entity\n");
    out.push('\n');

    out.push_str("view!: &space-home-view\n");
    out.push_str("  this: id:space:home/view\n");
    out.push_str("  model: space:home\n");
    out.push_str("  display: |\n");
    let multi = models.len() >= 2;
    for m in models {
        if multi {
            let _ = writeln!(
                out,
                "    <section><h2>{m}</h2><tonk-display model={m} /></section>"
            );
        } else {
            let _ = writeln!(out, "    <tonk-display model={m} />");
        }
    }
    out.push('\n');

    out.push_str("name!:\n");
    out.push_str("  this: id:tonk/space\n");
    out.push_str("  entity: space:home\n");

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_parses_an_attr_spec() {
        let spec = parse_attr_spec("title:text:one").unwrap();
        assert_eq!(
            (
                spec.field.as_str(),
                spec.type_name.as_str(),
                spec.cardinality.as_str()
            ),
            ("title", "Text", "one")
        );
    }
    #[test]
    fn it_accepts_canonical_type_spellings_case_insensitively() {
        assert_eq!(
            parse_attr_spec("n:UnsignedInteger:one").unwrap().type_name,
            "UnsignedInteger"
        );
        assert_eq!(
            parse_attr_spec("n:unsignedinteger:one").unwrap().type_name,
            "UnsignedInteger"
        );
        assert_eq!(
            parse_attr_spec("n:boolean:many").unwrap().type_name,
            "Boolean"
        );
    }
    #[test]
    fn it_rejects_an_unknown_type_enumerating_the_valid_ones() {
        let err = parse_attr_spec("n:string:one").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("Text") && msg.contains("Boolean"), "{msg}");
    }
    #[test]
    fn it_rejects_a_bad_cardinality() {
        let msg = format!("{}", parse_attr_spec("n:text:lots").unwrap_err());
        assert!(msg.contains("one") && msg.contains("many"), "{msg}");
    }
    #[test]
    fn it_rejects_a_malformed_spec() {
        let msg = format!("{}", parse_attr_spec("just-a-name").unwrap_err());
        assert!(msg.contains("<field>:<type>:<cardinality>"), "{msg}");
    }
    #[test]
    fn it_builds_an_anchored_concept_decl() {
        let attrs = vec![parse_attr_spec("title:text:one").unwrap()];
        let doc = build_concept_decl("note", Some("a note"), &attrs);
        assert!(doc.contains("attribute!: &note-title"));
        assert!(doc.contains("the:         xyz.tonk.note/title"));
        assert!(doc.contains("as:          Text"));
        assert!(doc.contains("concept!: &note"));
        assert!(doc.contains("title: note-title"));
    }
    #[test]
    fn it_builds_a_view_decl_with_a_stable_this() {
        let doc = build_view_decl("note-view", "note", "<b>{title}</b>");
        assert!(doc.contains("view!: &note-view"));
        assert!(doc.contains("this: id:note-view"));
        assert!(doc.contains("model: note"));
        assert!(doc.contains("<b>{title}</b>"));
    }
    #[test]
    fn it_builds_the_home_recipe_per_the_verified_shape() {
        let doc = build_home_recipe(&["habit".into(), "entry".into()]);
        assert!(doc.contains("concept!: &space-home"));
        assert!(doc.contains("this: space:home"));
        assert!(doc.contains("the: dialog.origin/subject"));
        assert!(doc.contains("view!: &space-home-view"));
        assert!(doc.contains("this: id:space:home/view"));
        assert!(doc.contains("<tonk-display model=habit />"));
        assert!(doc.contains("<tonk-display model=entry />"));
        assert!(doc.contains("this: id:tonk/space"));
        assert!(doc.contains("entity: space:home"));
    }
}
