//! How tonk spells the way an attribute is read, and what that is in
//! the dialog tonk runs on. The one place the two meet.
//!
//! Tonk's YAML gives an attribute a `pick:` (`last`, `all`, `top`,
//! `max`, `min`), and a listed `as:` with no pick ranks its values,
//! best first, as `top`. Tonk's meta vocabulary stores that pick as an
//! entity on the attribute (`db.attribute/pick top:`).
//!
//! Dialog reads an attribute under a cardinality alone, `one` or
//! `many`, and ranks claims only in a concept's field, by the field's
//! pick. So a ranked tonk attribute lowers to a cardinality-many
//! attribute, and every concept field that reads it picks the rank
//! over the same list, in the same order. `top` is deprecated in
//! dialog and kept there so an embedder can move later; moving tonk to
//! `max` over lists written least first is that later migration.

use dialog_query::selective::Selective;
use dialog_query::{AttributeDescriptor, Cardinality, ConceptFieldDescriptor};

/// How a tonk attribute is read: the cardinality dialog writes and
/// reads it under, and the rank a concept field reading it picks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Reading {
    /// How many claims a cell keeps.
    pub cardinality: Cardinality,
    /// The claim a field reading the attribute elects, for a ranked
    /// attribute. Always over cardinality many.
    pub rank: Option<Selective>,
}

impl Reading {
    /// A plain attribute under `cardinality`.
    pub fn plain(cardinality: Cardinality) -> Self {
        Self {
            cardinality,
            rank: None,
        }
    }

    /// A ranked attribute: cardinality many, read by `rank`.
    pub fn ranked(rank: Selective) -> Self {
        Self {
            cardinality: Cardinality::Many,
            rank: Some(rank),
        }
    }

    /// The reading a tonk pick names, by its plain name (`top`) or the
    /// entity tonk's meta vocabulary stores it as (`top:`). `None` for
    /// any other name.
    pub fn of_pick(name: &str) -> Option<Self> {
        match name.strip_suffix(':').unwrap_or(name) {
            "last" => Some(Self::plain(Cardinality::One)),
            "all" => Some(Self::plain(Cardinality::Many)),
            "top" => Some(Self::ranked(Selective::Top)),
            "max" => Some(Self::ranked(Selective::Max)),
            "min" => Some(Self::ranked(Selective::Min)),
            _ => None,
        }
    }

    /// The pick's plain name: `last`, `all`, `top`, `max` or `min`.
    pub fn pick_name(&self) -> &'static str {
        match self.rank {
            Some(Selective::Top) => "top",
            Some(Selective::Max) => "max",
            Some(Selective::Min) => "min",
            Some(Selective::Last) | None => match self.cardinality {
                Cardinality::One => "last",
                Cardinality::Many => "all",
            },
        }
    }

    /// The entity tonk's meta vocabulary stores the pick as
    /// (`db.attribute/pick`): `last:`, `all:`, `top:`, `max:` or `min:`.
    pub fn pick_entity(&self) -> String {
        format!("{}:", self.pick_name())
    }

    /// The reading of a field dialog built: its attribute's cardinality
    /// and the rank it picks.
    pub fn of_field(field: &ConceptFieldDescriptor) -> Self {
        Self {
            cardinality: field.descriptor().cardinality(),
            rank: field.pick().filter(|pick| *pick != Selective::Last),
        }
    }

    /// A concept field over `descriptor` read this way: the rank picked
    /// over the attribute's cardinality.
    pub fn field(
        &self,
        descriptor: AttributeDescriptor,
        optional: bool,
    ) -> Result<ConceptFieldDescriptor, String> {
        let descriptor = descriptor.with_cardinality(self.cardinality);
        let field = if optional {
            ConceptFieldDescriptor::optional(descriptor)
        } else {
            ConceptFieldDescriptor::required(descriptor)
        };
        match self.rank {
            Some(rank) => field.picking(rank).map_err(|error| error.to_string()),
            None => Ok(field),
        }
    }

    /// The JSON a field reading the attribute adds to the attribute's
    /// own: its pick, when it ranks.
    pub fn field_pick(&self) -> Option<&'static str> {
        self.rank.map(|_| self.pick_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_reads_every_pick_by_name_and_by_entity() {
        for (name, cardinality, rank) in [
            ("last", Cardinality::One, None),
            ("all", Cardinality::Many, None),
            ("top", Cardinality::Many, Some(Selective::Top)),
            ("max", Cardinality::Many, Some(Selective::Max)),
            ("min", Cardinality::Many, Some(Selective::Min)),
        ] {
            let reading = Reading { cardinality, rank };
            assert_eq!(Reading::of_pick(name), Some(reading));
            assert_eq!(Reading::of_pick(&format!("{name}:")), Some(reading));
            assert_eq!(reading.pick_name(), name);
            assert_eq!(reading.pick_entity(), format!("{name}:"));
        }
        assert_eq!(Reading::of_pick("select"), None);
    }
}
