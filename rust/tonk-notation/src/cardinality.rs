//! The keywords an attribute's `cardinality:` and a concept field's
//! `aggregate:` take, and the `pick:` an earlier release spelled.
//!
//! An attribute is a relation, a type and a cardinality: `cardinality:
//! :one` (the default, so it is left out) holds one claim per entity,
//! and a write succeeds it; `cardinality: :many` holds every claim, and
//! a write adds one. A concept field over a cardinality-many attribute
//! may read one of its claims with a selective aggregator,
//! `aggregate: :top` (the value listed last in the attribute's `as:`,
//! which lists its values least first), `:max`, `:min` or `:last` (the
//! newest).
//!
//! The release before spelled both with one `pick:`. It is still read,
//! with a warning naming what to write: `pick: :last` is the default
//! cardinality, `pick: :all` is `cardinality: :many`, and a ranking pick
//! on a concept field is `cardinality: :many` read with the aggregator
//! of its name.

/// How many claims an attribute holds per entity, as the notation
/// spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cardinality {
    /// One claim, which a write succeeds: `:one`, the default.
    One,
    /// Every claim, which a write adds to: `:many`.
    Many,
}

impl Cardinality {
    /// Every cardinality.
    pub const ALL: [Cardinality; 2] = [Cardinality::One, Cardinality::Many];

    /// The keyword the notation spells this cardinality by, without
    /// its colon, which is also the name dialog's wire form writes.
    pub fn keyword(self) -> &'static str {
        match self {
            Cardinality::One => "one",
            Cardinality::Many => "many",
        }
    }

    /// The cardinality a keyword names: `one` or `many`, in either
    /// case an earlier release wrote.
    pub fn from_keyword(name: &str) -> Option<Self> {
        match name {
            "one" | "One" => Some(Cardinality::One),
            "many" | "Many" => Some(Cardinality::Many),
            _ => None,
        }
    }
}

/// The selective aggregator a concept field reads its claims with, as
/// the notation spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Aggregate {
    /// The newest claim: `:last`.
    Last,
    /// The greatest value: `:max`.
    Max,
    /// The least value: `:min`.
    Min,
    /// The value listed last in the attribute's `as:`: `:top`.
    Top,
}

impl Aggregate {
    /// Every aggregator.
    pub const ALL: [Aggregate; 4] = [
        Aggregate::Last,
        Aggregate::Max,
        Aggregate::Min,
        Aggregate::Top,
    ];

    /// The keyword the notation spells this aggregator by, without its
    /// colon, which is also the name dialog's wire form writes.
    pub fn keyword(self) -> &'static str {
        match self {
            Aggregate::Last => "last",
            Aggregate::Max => "max",
            Aggregate::Min => "min",
            Aggregate::Top => "top",
        }
    }

    /// The aggregator a keyword names.
    pub fn from_keyword(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|aggregate| aggregate.keyword() == name)
    }
}

/// A pick, as the release before cardinalities spelled it, and what it
/// reads as now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pick {
    /// The newest claim: cardinality one.
    Last,
    /// Every claim: cardinality many.
    All,
    /// The best ranked of the listed values: cardinality many, read
    /// with `aggregate: :top`.
    Top,
    /// The greatest value: cardinality many, read with `aggregate: :max`.
    Max,
    /// The least value: cardinality many, read with `aggregate: :min`.
    Min,
}

impl Pick {
    /// Every pick.
    pub const ALL: [Pick; 5] = [Pick::Last, Pick::All, Pick::Top, Pick::Max, Pick::Min];

    /// The entity the release before named this pick by, such as `all:`.
    pub fn uri(self) -> &'static str {
        match self {
            Pick::Last => "last:",
            Pick::All => "all:",
            Pick::Top => "top:",
            Pick::Max => "max:",
            Pick::Min => "min:",
        }
    }

    /// The pick's word, such as `all`, which dialog 0.2 also wrote.
    pub fn keyword(self) -> &'static str {
        let uri = self.uri();
        &uri[..uri.len() - 1]
    }

    /// The pick a stored or wire `pick` names: its entity (`all:`), or
    /// the plain name dialog 0.2 wrote (`all`).
    pub fn from_wire(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|pick| pick.uri() == name || pick.keyword() == name)
    }

    /// The pick a keyword names: `last`, `all`, `top`, `max` or `min`.
    pub fn from_keyword(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|pick| pick.keyword() == name)
    }

    /// The cardinality the pick's attribute is written with now.
    pub fn cardinality(self) -> Cardinality {
        match self {
            Pick::Last => Cardinality::One,
            _ => Cardinality::Many,
        }
    }

    /// The aggregator a field reading under the pick reads with now,
    /// for a ranking pick.
    pub fn aggregate(self) -> Option<Aggregate> {
        match self {
            Pick::Last | Pick::All => None,
            Pick::Top => Some(Aggregate::Top),
            Pick::Max => Some(Aggregate::Max),
            Pick::Min => Some(Aggregate::Min),
        }
    }

    /// What to write instead of `pick: :<this>`.
    pub fn replacement(self) -> &'static str {
        match self {
            Pick::Last => "leave it out: cardinality one is the default",
            Pick::All => "write `cardinality: :many`",
            Pick::Top => {
                "write `cardinality: :many` and `aggregate: :top` on the concept field, \
                 listing `as:` least first"
            }
            Pick::Max => "write `cardinality: :many` and `aggregate: :max` on the concept field",
            Pick::Min => "write `cardinality: :many` and `aggregate: :min` on the concept field",
        }
    }
}

/// The entity a built-in anchor names: a type, such as `text` for
/// `text:`.
pub fn builtin(name: &str) -> Option<&'static str> {
    crate::ValueType::from_anchor(name).map(crate::ValueType::uri)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn it_reads_both_wire_spellings_of_a_pick() {
        assert_eq!(Pick::from_wire("all:"), Some(Pick::All));
        assert_eq!(Pick::from_wire("all"), Some(Pick::All));
        assert_eq!(Pick::from_wire("All"), None);
        assert_eq!(Pick::from_keyword("all:"), None);
    }

    #[dialog_common::test]
    fn it_reads_a_pick_as_a_cardinality_and_an_aggregate() {
        assert_eq!(Pick::Last.cardinality(), Cardinality::One);
        assert_eq!(Pick::All.cardinality(), Cardinality::Many);
        assert_eq!(Pick::All.aggregate(), None);
        assert_eq!(Pick::Top.cardinality(), Cardinality::Many);
        assert_eq!(Pick::Top.aggregate(), Some(Aggregate::Top));
    }

    #[dialog_common::test]
    fn it_reads_cardinality_and_aggregate_keywords() {
        assert_eq!(Cardinality::from_keyword("many"), Some(Cardinality::Many));
        assert_eq!(Cardinality::from_keyword("Many"), Some(Cardinality::Many));
        assert_eq!(Cardinality::from_keyword("all"), None);
        assert_eq!(Aggregate::from_keyword("top"), Some(Aggregate::Top));
        assert_eq!(Aggregate::from_keyword("all"), None);
    }

    #[dialog_common::test]
    fn it_names_built_in_anchors() {
        assert_eq!(builtin("text"), Some("text:"));
        assert_eq!(builtin("unsigned-integer"), Some("natural:"));
        assert_eq!(builtin("max"), None, "a pick is no built-in anchor");
        assert_eq!(builtin("person"), None);
    }
}
