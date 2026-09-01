//! Pure notation builders for the noun-first authoring verbs
//! (`tonk concept add`, `tonk view add`, `tonk space home`). No I/O — every
//! function here takes already-parsed arguments and returns (or
//! errors on) a `String` of asserted notation; callers (later tasks)
//! own reading CLI flags and handing the result to
//! `eval::run_against_site`.
//!
//! `build_home_recipe` in particular reproduces a verified shape.
//! The root concept's
//! one `with:` field must map to `the: dialog.replica/subject` /
//! `as: entity` — the only attribute guaranteed already-asserted on
//! the entity the space-home route renders — and the inline
//! attribute under `with:` must carry its own `description:` (a hard
//! analyzer requirement, independent of the concept's own
//! description). Deviating from the recipe reproduces the "blank
//! canvas" or "Concept mismatch" failures documented there.

use std::fmt::Write as _;

use crate::schema::SPACE_HOME_CONCEPT;

/// One of the four standard `show` facets `tonk view add` authors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewKind {
    /// The detail presentation (`show: {ui: …}`).
    Detail,
    /// The entity-set presentation (`show: {directory: …}`).
    Directory,
    /// A compact reference label (`show: {label: …}`).
    Label,
    /// A browser-tab title (`show: {title: …}`).
    Title,
}

impl ViewKind {
    /// The `show` entry key this kind writes.
    pub fn facet(self) -> &'static str {
        match self {
            Self::Detail => "ui",
            Self::Directory => "directory",
            Self::Label => "label",
            Self::Title => "title",
        }
    }

    /// Whether a first view of this kind can sensibly surface a model's
    /// directory on an otherwise blank home.
    pub fn can_auto_surface(self) -> bool {
        matches!(self, Self::Detail | Self::Directory)
    }
}

/// One parsed `--field field:type:cardinality` flag, ready to render
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

/// Error parsing a raw `--field` flag or building notation from it.
#[derive(Debug, thiserror::Error)]
pub enum AuthoringError {
    /// The raw value didn't split into exactly three `:`-separated
    /// parts (`<field>:<type>:<cardinality>`).
    #[error(
        "--field '{raw}' is malformed; expected <field>:<type>:<cardinality>, e.g. title:text:one"
    )]
    BadAttrSpec {
        /// The raw, unparsed `--field` value.
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
/// `schema::type_to_notation`'s output (which `tonk show --notation` proves
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

/// Parse a raw `--field field:type:cardinality` flag value into an
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

/// Lint a view template against its model's field names. Returns
/// warning lines (empty when clean). Both classes are live-shell
/// failures that a headless render won't necessarily catch:
///
/// - a `{placeholder}` that names no field of the model renders
///   blank (or literally) with no error anywhere;
/// - a nested `<tonk-display entity=…>` whose value is a bare name
///   is rejected by the browser shell ("`entity` must be an entity
///   URI"), even though headless `tonk render` resolves it.
///
/// Purely lexical — warnings, never errors, because templates can
/// carry CSS braces and exotic-but-valid values. Specials (`{this}`,
/// `{dom.host/model}`-style paths, anything with `.`/`/`/`:` or
/// whitespace inside the braces) are skipped.
pub fn lint_view_template(template: &str, fields: &[String]) -> Vec<String> {
    let mut warnings = Vec::new();
    let mut flagged: Vec<String> = Vec::new();
    for token in brace_tokens(template) {
        if token == "this" || fields.iter().any(|f| f == &token) || flagged.contains(&token) {
            continue;
        }
        flagged.push(token.clone());
        warnings.push(format!(
            "template references {{{token}}} but the model has no field '{token}' \
             (fields: {}) — it will render blank",
            fields.join(", "),
        ));
    }
    for value in entity_attr_values(template) {
        if value.starts_with('{') || value.contains(':') {
            continue;
        }
        warnings.push(format!(
            "entity={value} is a bare name; the browser shell requires an entity URI \
             (did:key:…, id:…) — use {{this}} or a URI"
        ));
    }
    warnings
}

/// Collect candidate `{field}` placeholder tokens: brace-enclosed
/// runs that look like field names (lowercase start; lowercase,
/// digits, `-` after). CSS blocks and path specials don't match —
/// anything containing whitespace, `.`, `/`, `:`, `{`, or longer
/// than 64 chars is skipped.
fn brace_tokens(template: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{'
            && let Some(end) = template[i + 1..].find(['}', '{']).map(|o| i + 1 + o)
            && bytes[end] == b'}'
            && end - i - 1 <= 64
        {
            let token = &template[i + 1..end];
            let mut chars = token.chars();
            let head_ok = chars.next().is_some_and(|c| c.is_ascii_lowercase());
            let tail_ok = chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
            if head_ok && tail_ok {
                out.push(token.to_string());
            }
            i = end + 1;
            continue;
        }
        i += 1;
    }
    out
}

/// Collect the values of `entity=` attributes in the template.
/// Lexical: an `entity=` preceded by whitespace, with the value read
/// to the closing quote (when quoted) or to the next whitespace /
/// `>` / `/>` (when bare).
fn entity_attr_values(template: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = template;
    while let Some(pos) = rest.find("entity=") {
        let preceded_ok = pos == 0
            || rest[..pos]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_whitespace());
        let after = &rest[pos + "entity=".len()..];
        if !preceded_ok {
            rest = after;
            continue;
        }
        let value = match after.chars().next() {
            Some(quote @ ('"' | '\'')) => after[1..].split(quote).next().unwrap_or(""),
            _ => after
                .split(|c: char| c.is_ascii_whitespace() || c == '>')
                .next()
                .unwrap_or("")
                .trim_end_matches('/'),
        };
        if !value.is_empty() {
            out.push(value.to_string());
        }
        rest = after;
    }
    out
}

/// Build a `view!:` declaration document. The view instance IS the
/// model (`this: <model>`), and the template lands under the kind's
/// facet in the `show` dictionary — one entry per facet, cardinality
/// one, so re-authoring the same facet supersedes rather than
/// duplicates it. `template` is emitted verbatim, line by line, under
/// the facet's block scalar.
pub fn build_view_decl(kind: ViewKind, model: &str, template: &str) -> String {
    let mut out = String::new();
    out.push_str("view!:\n");
    let _ = writeln!(out, "  this: {model}");
    out.push_str("  show:\n");
    let _ = writeln!(out, "    {}: |", kind.facet());
    for line in template.lines() {
        let _ = writeln!(out, "      {line}");
    }
    out
}

/// Build the space-home recipe: the origin-keyed root concept, its
/// view (one `<tonk-display model=X />` per model — wrapped in a
/// `<section>` with an `<h2>` heading when there are 2+ models, a
/// bare tag when there's exactly one), and the `name!:` repoint of
/// `id:tonk/space` onto it.
///
/// See the module documentation for why each piece is load-bearing.
pub fn build_home_recipe(models: &[String]) -> String {
    let mut out = String::new();

    // The anchor doubles as the concept's published name, and
    // agent-facing listings filter on that name — so it comes from
    // the same const the filter reads, not a second copy of the
    // literal.
    let _ = writeln!(out, "concept!: &{SPACE_HOME_CONCEPT}");
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
    out.push_str("      the: dialog.replica/subject\n");
    out.push_str("      as: entity\n");
    out.push('\n');

    out.push_str("view!:\n");
    out.push_str("  this: space:home\n");
    out.push_str("  show:\n");
    out.push_str("    ui: |\n");
    let multi = models.len() >= 2;
    for m in models {
        if multi {
            let _ = writeln!(
                out,
                "      <section><h2>{m}</h2><tonk-display model={m} /></section>"
            );
        } else {
            let _ = writeln!(out, "      <tonk-display model={m} />");
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
    fn it_builds_a_view_decl_on_the_model_entity() {
        let doc = build_view_decl(ViewKind::Detail, "note", "<b>{title}</b>");
        assert!(doc.starts_with("view!:\n"), "{doc}");
        assert!(doc.contains("this: note"));
        assert!(doc.contains("show:\n    ui: |\n"));
        assert!(doc.contains("<b>{title}</b>"));
    }
    #[test]
    fn it_builds_each_view_kind_under_its_facet() {
        for (kind, facet) in [
            (ViewKind::Detail, "ui"),
            (ViewKind::Directory, "directory"),
            (ViewKind::Label, "label"),
            (ViewKind::Title, "title"),
        ] {
            assert_eq!(kind.facet(), facet);
            let doc = build_view_decl(kind, "note", "<b>{title}</b>");
            assert!(doc.contains(&format!("    {facet}: |")), "{doc}");
            assert!(doc.contains("this: note"), "{doc}");
        }
    }

    fn fields(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn it_passes_a_clean_template() {
        let warnings = lint_view_template(
            "<article><h2>{name}</h2><p>{age}</p><b>{this}</b></article>",
            &fields(&["name", "age"]),
        );
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn it_flags_an_unknown_placeholder_once() {
        let warnings = lint_view_template("<p>{nmae}</p><p>{nmae}</p>", &fields(&["name"]));
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("{nmae}") && warnings[0].contains("name"));
    }

    #[test]
    fn it_skips_css_blocks_and_path_specials() {
        let warnings = lint_view_template(
            "<style>.a { color: red; }</style>\
             <div>{dom.host/model}</div>\
             <span style=\"{color:red}\"></span>",
            &fields(&["name"]),
        );
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn it_flags_a_bare_name_entity_reference() {
        let warnings = lint_view_template(
            "<tonk-display entity=alice model=person></tonk-display>",
            &fields(&["name"]),
        );
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("entity=alice"), "{warnings:?}");
    }

    #[test]
    fn it_accepts_uri_and_interpolated_entity_references() {
        let warnings = lint_view_template(
            "<tonk-display entity={author} model=person></tonk-display>\
             <tonk-display entity=did:key:zAbc model=person></tonk-display>\
             <tonk-display entity=\"id:alice\" model=person></tonk-display>",
            &fields(&["author"]),
        );
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn it_ignores_data_entity_attributes() {
        let warnings = lint_view_template(
            "<button data-entity=alice onclick=poke>go</button>",
            &fields(&["name"]),
        );
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn it_builds_the_home_recipe_per_the_verified_shape() {
        let doc = build_home_recipe(&["habit".into(), "entry".into()]);
        assert!(doc.contains("concept!: &space-home"));
        assert!(doc.contains("this: space:home"));
        assert!(doc.contains("the: dialog.replica/subject"));
        assert!(doc.contains("view!:\n  this: space:home\n  show:\n    ui: |"));
        assert!(doc.contains("<tonk-display model=habit />"));
        assert!(doc.contains("<tonk-display model=entry />"));
        assert!(doc.contains("this: id:tonk/space"));
        assert!(doc.contains("entity: space:home"));
    }
}
