//! The concept panel's row model: what to say about each field, and
//! why a value is or is not on screen.
//!
//! The panel exists to answer one question — *why isn't my value
//! showing up?* — and that question has four different answers which
//! look identical on a rendered page:
//!
//! - the concept declares the field, but no slot in the template reads
//!   it, so it was never going to appear;
//! - a slot reads it, but the concept does not declare it, so the query
//!   never projected it and the slot renders blank forever;
//! - both sides agree, but this particular subject has no value, so the
//!   slot renders an empty string;
//! - everything is fine and the value is on screen.
//!
//! Only the last one is visible from the page. The other three are a
//! blank where something should be. Naming them is most of the value of
//! having a panel at all, so [`rows`] classifies every field either
//! side knows about, whether or not it rendered.
//!
//! Pure: it takes a [`Snapshot`] and gives back rows, so it tests
//! without a browser.

use serde::{Deserialize, Serialize};

use super::slot::{Field, Origin, Slot, Snapshot};

/// Why a field's value is, or is not, on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// Declared, read by a slot, and the subject has a value. The
    /// ordinary case.
    Rendered,
    /// Declared and read, but this subject has no value for it. The
    /// slot rendered an empty string.
    Absent,
    /// Declared by the concept, read by no slot. The template never
    /// asked for it.
    Unrendered,
    /// Read by a slot the concept does not declare. The query never
    /// projected it, so the slot is blank for every subject — usually
    /// a typo, or a field the model lost.
    Undeclared,
}

impl Status {
    /// Whether this is a mismatch worth drawing attention to.
    pub fn is_finding(self) -> bool {
        !matches!(self, Status::Rendered)
    }

    /// A short phrase for the panel's status column.
    pub fn label(self) -> &'static str {
        match self {
            Status::Rendered => "",
            Status::Absent => "no value",
            Status::Unrendered => "not in the view",
            Status::Undeclared => "not on the concept",
        }
    }
}

/// One row of the concept panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
    /// The field name, as a template writes it between braces.
    pub name: String,
    /// The concept's declaration, when it has one.
    pub declared: Option<Field>,
    /// The selected subject's value, already spelled as the renderer
    /// spelled it. `None` when the subject carries no such field.
    pub value: Option<String>,
    /// Ids of every slot that reads this field — what a hover
    /// highlights on the page.
    pub slots: Vec<u32>,
    /// How many of those slots rendered nothing.
    pub blank_slots: usize,
    /// Why the value is or is not on screen.
    pub status: Status,
}

/// Build the panel's rows for one subject.
///
/// `subject` is the `this` of the entity the panel is showing; an
/// unknown or absent subject yields rows with no values, which is the
/// right answer for a directory whose pointer is not over any row.
///
/// Rows come out in declaration order, then any undeclared field a
/// slot reads, so the concept reads as itself and the surprises land
/// at the bottom. Synthesized references (`{this}`, `{dom.host/*}`,
/// iteration keys) are not fields of the concept and get no row.
pub fn rows(snapshot: &Snapshot, subject: Option<&str>) -> Vec<Row> {
    let entity = subject.and_then(|subject| {
        snapshot
            .entities
            .iter()
            .find(|entity| entity.this == subject)
    });

    let mut out: Vec<Row> = Vec::new();
    for field in &snapshot.fields {
        let slots = reading(&snapshot.slots, &field.name);
        let value = entity.and_then(|entity| entity.fields.get(&field.name).cloned());
        let blank_slots = blank(&snapshot.slots, &slots);
        let status = if slots.is_empty() {
            Status::Unrendered
        } else if value.is_none() {
            Status::Absent
        } else {
            Status::Rendered
        };
        out.push(Row {
            name: field.name.clone(),
            declared: Some(field.clone()),
            value,
            slots,
            blank_slots,
            status,
        });
    }

    for name in undeclared(snapshot) {
        let slots = reading(&snapshot.slots, &name);
        let blank_slots = blank(&snapshot.slots, &slots);
        out.push(Row {
            name,
            declared: None,
            value: None,
            slots,
            blank_slots,
            status: Status::Undeclared,
        });
    }

    out
}

/// The subject the panel shows by default: the one under the pointer
/// if it is in the frame, else the lead conclusion.
pub fn default_subject<'a>(snapshot: &'a Snapshot, hovered: Option<&str>) -> Option<&'a str> {
    if let Some(hovered) = hovered
        && let Some(entity) = snapshot
            .entities
            .iter()
            .find(|entity| entity.this == hovered)
    {
        return Some(&entity.this);
    }
    snapshot.entities.first().map(|entity| entity.this.as_str())
}

/// Ids of the slots reading `name`.
fn reading(slots: &[Slot], name: &str) -> Vec<u32> {
    slots
        .iter()
        .filter(|slot| slot.reads(name))
        .map(|slot| slot.id)
        .collect()
}

/// How many of `ids` rendered an empty string.
fn blank(slots: &[Slot], ids: &[u32]) -> usize {
    slots
        .iter()
        .filter(|slot| ids.contains(&slot.id) && slot.value.is_empty())
        .count()
}

/// Concept-origin field names a slot reads that the concept does not
/// declare, in first-seen order.
fn undeclared(snapshot: &Snapshot) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for slot in &snapshot.slots {
        for name in &slot.fields {
            if Origin::of(name) == Origin::Concept
                && snapshot.declared(name).is_none()
                && !out.contains(name)
            {
                out.push(name.clone());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::introspect::slot::{Entity, SlotKind, SlotScope};
    use std::collections::BTreeMap;

    fn field(name: &str, optional: bool) -> Field {
        Field {
            name: name.to_owned(),
            attribute: Some(format!("xyz.tonk.test/{name}")),
            value_type: Some("Text".to_owned()),
            cardinality: Some("one".to_owned()),
            optional,
        }
    }

    fn slot(id: u32, field: &str, value: &str) -> Slot {
        Slot {
            id,
            fields: vec![field.to_owned()],
            origin: Origin::of(field),
            kind: SlotKind::Text,
            scope: SlotScope::Chrome,
            value: value.to_owned(),
        }
    }

    fn entity(this: &str, values: &[(&str, &str)]) -> Entity {
        let mut fields = BTreeMap::new();
        for (name, value) in values {
            fields.insert((*name).to_owned(), (*value).to_owned());
        }
        Entity {
            this: this.to_owned(),
            fields,
        }
    }

    fn snapshot(fields: Vec<Field>, slots: Vec<Slot>, entities: Vec<Entity>) -> Snapshot {
        Snapshot {
            fields,
            slots,
            entities,
            ..Snapshot::default()
        }
    }

    #[test]
    fn a_field_with_a_slot_and_a_value_is_simply_rendered() {
        let snap = snapshot(
            vec![field("title", false)],
            vec![slot(0, "title", "Hello")],
            vec![entity("did:key:a", &[("title", "Hello")])],
        );
        let rows = rows(&snap, Some("did:key:a"));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, Status::Rendered);
        assert_eq!(rows[0].value.as_deref(), Some("Hello"));
        assert_eq!(rows[0].slots, [0]);
        assert!(!rows[0].status.is_finding());
    }

    #[test]
    fn a_declared_field_no_slot_reads_is_not_in_the_view() {
        let snap = snapshot(
            vec![field("title", false), field("body", false)],
            vec![slot(0, "title", "Hello")],
            vec![entity("did:key:a", &[("title", "Hello"), ("body", "text")])],
        );
        let rows = rows(&snap, Some("did:key:a"));
        let body = rows.iter().find(|row| row.name == "body").expect("row");
        assert_eq!(body.status, Status::Unrendered);
        assert!(
            body.value.is_some(),
            "the subject has the value; the template just never asks for it"
        );
    }

    #[test]
    fn a_slot_reading_an_undeclared_field_is_reported_last() {
        let snap = snapshot(
            vec![field("title", false)],
            vec![slot(0, "title", "Hello"), slot(1, "titel", "")],
            vec![entity("did:key:a", &[("title", "Hello")])],
        );
        let rows = rows(&snap, Some("did:key:a"));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].name, "titel");
        assert_eq!(rows[1].status, Status::Undeclared);
        assert_eq!(rows[1].declared, None);
    }

    #[test]
    fn a_field_this_subject_lacks_reads_as_absent_not_missing() {
        let snap = snapshot(
            vec![field("cover", true)],
            vec![slot(0, "cover", "")],
            vec![entity("did:key:a", &[])],
        );
        let rows = rows(&snap, Some("did:key:a"));
        assert_eq!(rows[0].status, Status::Absent);
        assert_eq!(rows[0].blank_slots, 1);
        assert_eq!(rows[0].status.label(), "no value");
    }

    #[test]
    fn a_subject_not_in_the_frame_yields_rows_without_values() {
        let snap = snapshot(
            vec![field("title", false)],
            vec![slot(0, "title", "Hello")],
            vec![entity("did:key:a", &[("title", "Hello")])],
        );
        let rows = rows(&snap, Some("did:key:zzz"));
        assert_eq!(rows[0].value, None);
        assert_eq!(rows[0].status, Status::Absent);
    }

    #[test]
    fn synthesized_references_never_become_rows() {
        let snap = snapshot(
            vec![field("title", false)],
            vec![
                slot(0, "title", "Hello"),
                slot(1, "this", "did:key:a"),
                slot(2, "dom.host/data-active", "yes"),
                slot(3, "title/key", "0"),
            ],
            vec![entity("did:key:a", &[("title", "Hello")])],
        );
        let rows = rows(&snap, Some("did:key:a"));
        let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(names, ["title"]);
    }

    #[test]
    fn a_row_collects_every_slot_reading_its_field() {
        let snap = snapshot(
            vec![field("title", false)],
            vec![slot(0, "title", "Hello"), slot(7, "title", "Hello")],
            vec![entity("did:key:a", &[("title", "Hello")])],
        );
        assert_eq!(rows(&snap, Some("did:key:a"))[0].slots, [0, 7]);
    }

    #[test]
    fn the_panel_follows_the_pointer_when_it_is_over_a_known_subject() {
        let snap = snapshot(
            vec![field("title", false)],
            vec![],
            vec![
                entity("did:key:a", &[("title", "One")]),
                entity("did:key:b", &[("title", "Two")]),
            ],
        );
        assert_eq!(default_subject(&snap, Some("did:key:b")), Some("did:key:b"));
    }

    #[test]
    fn it_falls_back_to_the_lead_subject_off_any_row() {
        let snap = snapshot(
            vec![field("title", false)],
            vec![],
            vec![
                entity("did:key:a", &[("title", "One")]),
                entity("did:key:b", &[("title", "Two")]),
            ],
        );
        assert_eq!(default_subject(&snap, None), Some("did:key:a"));
        assert_eq!(
            default_subject(&snap, Some("did:key:gone")),
            Some("did:key:a"),
            "a stale hover is not a subject"
        );
    }

    #[test]
    fn a_declaration_spells_its_type_and_cardinality() {
        assert_eq!(field("title", false).signature(), "Text one");
        assert_eq!(field("cover", true).signature(), "Text one?");
    }
}
