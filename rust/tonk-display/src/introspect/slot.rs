//! What a rendered slot is, described without reference to the DOM.
//!
//! A *slot* is one place a template interpolation landed: the text
//! node `{title}` filled, or the attribute `with="main@{repo}"` wrote.
//! The renderer already knows all of them — it keeps the binding plan
//! and the last string each binding produced — so describing a mounted
//! view is a walk over state that exists, not a re-parse of the DOM.
//!
//! These types are the wire between that walk and the overlay that
//! paints it. They are pure `std` + `serde` so they can be tested
//! natively and, later, serialized to a panel running in another frame.

use serde::{Deserialize, Serialize};

/// Where a slot's value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Origin {
    /// A field projected from the model concept — the interesting
    /// case, and the one a concept panel can cross-highlight.
    Concept,
    /// `{this}`: the subject URI of the conclusion being rendered,
    /// synthesized rather than projected.
    Subject,
    /// `{dom.host/<attr>}`: copied off the outer host element's
    /// attributes, not from the branch at all.
    Host,
    /// `{<field>/key}`: the key of the current row inside an
    /// iteration over a many-valued field.
    Key,
}

impl Origin {
    /// Classify a field name as it appears between `{` and `}`.
    pub fn of(field: &str) -> Self {
        if field == "this" {
            Self::Subject
        } else if field.starts_with(tonk_template::fields::HOST_NAMESPACE) {
            Self::Host
        } else if field.ends_with("/key") {
            Self::Key
        } else {
            Self::Concept
        }
    }
}

/// Where a slot's output lands in the DOM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SlotKind {
    /// The slot fills a text node. This is the only kind with a
    /// visible region of its own — the overlay can box the glyphs it
    /// produced.
    Text,
    /// The slot writes an element attribute or property. It has no
    /// region; the overlay marks the element that carries it and
    /// leaves the detail to the panel.
    Attribute {
        /// The attribute (or property) name written.
        name: String,
        /// The author wrote `html:name={x}`, forcing `setAttribute`
        /// over a property assignment.
        forced: bool,
    },
}

/// Which rendering scope a slot belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "kebab-case")]
pub enum SlotScope {
    /// Outside the repeat: rendered once against the lead conclusion.
    Chrome,
    /// Inside a repeat row for one subject.
    Row {
        /// The row's subject URI.
        this: String,
    },
    /// Inside an iteration over one many-valued field of a subject.
    Iteration {
        /// The row's subject URI.
        this: String,
        /// The field being iterated.
        field: String,
        /// The key of this particular iteration row.
        key: String,
    },
}

/// One rendered slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slot {
    /// A stable-per-snapshot identifier, so the overlay can talk
    /// about a slot without holding its node.
    pub id: u32,
    /// Every field the slot reads, in template order. A plain
    /// `{title}` has one; `with="main@{repo}"` has one plus literal
    /// text; `"{a}-{b}"` has two.
    pub fields: Vec<String>,
    /// The origin of `fields[0]` — what the slot is *mostly* about.
    /// A mixed slot is rare enough that the panel can spell out the
    /// rest from `fields`.
    pub origin: Origin,
    /// Text or attribute.
    pub kind: SlotKind,
    /// Which scope it rendered in.
    pub scope: SlotScope,
    /// The string the renderer last wrote here.
    pub value: String,
}

impl Slot {
    /// The short label the overlay puts on the slot's badge:
    /// `title`, `a+b`, or `@with` for an attribute.
    pub fn label(&self) -> String {
        let fields = self
            .fields
            .iter()
            .map(|field| field.trim_start_matches(tonk_template::fields::HOST_NAMESPACE))
            .collect::<Vec<_>>()
            .join("+");
        match &self.kind {
            SlotKind::Text => fields,
            SlotKind::Attribute { name, .. } => format!("{name}={fields}"),
        }
    }

    /// Whether this slot reads `field`. Used to cross-highlight from
    /// a concept panel row back onto the rendered page.
    pub fn reads(&self, field: &str) -> bool {
        self.fields.iter().any(|candidate| candidate == field)
    }
}

/// Everything the overlay knows about one observed `<tonk-display>`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// The `model` attribute as the author wrote it.
    pub model: Option<String>,
    /// The concept URI that attribute resolved to.
    pub model_entity: Option<String>,
    /// The show facet actually rendered (`ui`, `directory`, ...).
    pub facet: Option<String>,
    /// Directory mode — every instance of the model, not one entity.
    pub directory: bool,
    /// The subject URI of every conclusion in the last frame.
    pub subjects: Vec<String>,
    /// The template HTML the mounted view was built from.
    pub template: Option<String>,
    /// The fields the model concept declares, from its descriptor's
    /// `with:` map. The concept panel's rows.
    pub fields: Vec<String>,
    /// Every slot the mounted view rendered.
    pub slots: Vec<Slot>,
}

impl Snapshot {
    /// The fields the concept declares that no slot reads — declared
    /// but unrendered. Worth surfacing: it is the usual reason a value
    /// "isn't showing up".
    pub fn unbound_fields(&self) -> Vec<&str> {
        self.fields
            .iter()
            .map(String::as_str)
            .filter(|field| !self.slots.iter().any(|slot| slot.reads(field)))
            .collect()
    }

    /// The concept fields a slot reads that the concept does not
    /// declare — a typo in the template, or a field the model lost.
    pub fn undeclared_fields(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for slot in &self.slots {
            for field in &slot.fields {
                if Origin::of(field) == Origin::Concept
                    && !self.fields.iter().any(|declared| declared == field)
                    && !out.contains(&field.as_str())
                {
                    out.push(field);
                }
            }
        }
        out
    }
}

/// The fields a model concept's descriptor declares, in declaration
/// order across its `with:` (required) and `maybe:` (optional) maps.
///
/// These are the rows a concept panel lists and the names a template's
/// `{field}` references are checked against. A descriptor that will not
/// parse yields nothing rather than an error — an introspection overlay
/// showing no fields is a better failure than one that will not open.
pub fn declared_fields(descriptor_json: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(descriptor_json) else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for block in ["with", "maybe"] {
        let Some(map) = value.get(block).and_then(|v| v.as_object()) else {
            continue;
        };
        for field in map.keys() {
            if !out.contains(field) {
                out.push(field.clone());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(id: u32, fields: &[&str], kind: SlotKind) -> Slot {
        Slot {
            id,
            fields: fields.iter().map(|f| (*f).to_owned()).collect(),
            origin: Origin::of(fields[0]),
            kind,
            scope: SlotScope::Chrome,
            value: String::new(),
        }
    }

    #[test]
    fn it_classifies_a_plain_field_as_a_concept_field() {
        assert_eq!(Origin::of("title"), Origin::Concept);
    }

    #[test]
    fn it_classifies_the_subject_and_host_and_key_references() {
        assert_eq!(Origin::of("this"), Origin::Subject);
        assert_eq!(Origin::of("dom.host/data-active"), Origin::Host);
        assert_eq!(Origin::of("tags/key"), Origin::Key);
    }

    #[test]
    fn a_text_slot_is_labelled_by_its_fields() {
        assert_eq!(slot(0, &["a", "b"], SlotKind::Text).label(), "a+b");
    }

    #[test]
    fn an_attribute_slot_names_the_attribute_it_writes() {
        let kind = SlotKind::Attribute {
            name: "with".to_owned(),
            forced: false,
        };
        assert_eq!(slot(0, &["repo"], kind).label(), "with=repo");
    }

    #[test]
    fn a_host_slot_label_drops_the_namespace() {
        assert_eq!(
            slot(0, &["dom.host/data-active"], SlotKind::Text).label(),
            "data-active"
        );
    }

    #[test]
    fn it_reports_a_declared_field_that_no_slot_renders() {
        let snapshot = Snapshot {
            fields: vec!["title".to_owned(), "body".to_owned()],
            slots: vec![slot(0, &["title"], SlotKind::Text)],
            ..Snapshot::default()
        };
        assert_eq!(snapshot.unbound_fields(), vec!["body"]);
    }

    #[test]
    fn it_reports_a_rendered_field_the_concept_does_not_declare() {
        let snapshot = Snapshot {
            fields: vec!["title".to_owned()],
            slots: vec![
                slot(0, &["title"], SlotKind::Text),
                slot(1, &["titel"], SlotKind::Text),
            ],
            ..Snapshot::default()
        };
        assert_eq!(snapshot.undeclared_fields(), vec!["titel"]);
    }

    #[test]
    fn synthesized_references_are_never_undeclared() {
        let snapshot = Snapshot {
            fields: vec!["title".to_owned()],
            slots: vec![
                slot(0, &["this"], SlotKind::Text),
                slot(1, &["dom.host/data-active"], SlotKind::Text),
                slot(2, &["title/key"], SlotKind::Text),
            ],
            ..Snapshot::default()
        };
        assert!(snapshot.undeclared_fields().is_empty());
    }

    #[test]
    fn it_reads_required_and_optional_fields_out_of_a_descriptor() {
        let descriptor = r#"{
            "with": { "title": { "the": "x/title" }, "body": { "the": "x/body" } },
            "maybe": { "cover": { "the": "x/cover" } }
        }"#;
        assert_eq!(declared_fields(descriptor), ["title", "body", "cover"]);
    }

    #[test]
    fn an_unparseable_descriptor_declares_nothing() {
        assert!(declared_fields("not json").is_empty());
    }
}
