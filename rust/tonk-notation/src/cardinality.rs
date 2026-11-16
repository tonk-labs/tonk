//! The keywords an attribute's `cardinality:` and `pick:` take.
//!
//! An attribute is a relation, a type and a cardinality: `cardinality:
//! :one` (the default) holds one claim per entity, and a write succeeds
//! it; `cardinality: :many` holds every claim, and a write adds one.
//!
//! `pick:` says which claims a read returns: `:last` the newest, `:all`
//! every one, and on a concept field over a cardinality-many attribute
//! one claim ranked by `:top` (the value listed first in the attribute's
//! `as:`, which lists its values best first), `:max` or `:min`. The
//! cardinality follows from the pick unless it is written: `:last` is
//! one, every other pick many. Written beside a pick, it has to be one
//! the pick can read: `pick: :last` with `cardinality: :many` keeps every
//! claim and reads the newest, and `cardinality: :one` beside any other
//! pick is refused.

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

/// A pick: which of a relation's claims a read returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pick {
    /// The newest claim: cardinality one, unless written many.
    Last,
    /// Every claim: cardinality many.
    All,
    /// The claim holding the value listed first in the attribute's `as:`:
    /// cardinality many, on a concept field.
    Top,
    /// The greatest value: cardinality many, on a concept field.
    Max,
    /// The least value: cardinality many, on a concept field.
    Min,
}

impl Pick {
    /// Every pick.
    pub const ALL: [Pick; 5] = [Pick::Last, Pick::All, Pick::Top, Pick::Max, Pick::Min];

    /// The keyword the notation spells this pick by, without its colon,
    /// which is also the name dialog's wire form writes.
    pub fn keyword(self) -> &'static str {
        match self {
            Pick::Last => "last",
            Pick::All => "all",
            Pick::Top => "top",
            Pick::Max => "max",
            Pick::Min => "min",
        }
    }

    /// The pick a keyword names: `last`, `all`, `top`, `max` or `min`.
    pub fn from_keyword(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|pick| pick.keyword() == name)
    }

    /// The pick a wire `pick` names: its name (`all`), or the entity
    /// dialog 0.3 and 0.4 named it by (`all:`).
    pub fn from_wire(name: &str) -> Option<Self> {
        Self::from_keyword(name.strip_suffix(':').unwrap_or(name))
    }

    /// The cardinality the pick implies when none is written: `last` is
    /// one, every other pick many.
    pub fn cardinality(self) -> Cardinality {
        match self {
            Pick::Last => Cardinality::One,
            _ => Cardinality::Many,
        }
    }

    /// Whether the pick ranks one claim among many, which only a concept
    /// field can read: `top`, `max` or `min`.
    pub fn ranks(self) -> bool {
        matches!(self, Pick::Top | Pick::Max | Pick::Min)
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
    fn it_derives_a_cardinality_from_a_pick() {
        assert_eq!(Pick::Last.cardinality(), Cardinality::One);
        assert_eq!(Pick::All.cardinality(), Cardinality::Many);
        assert_eq!(Pick::Top.cardinality(), Cardinality::Many);
        assert!(Pick::Top.ranks());
        assert!(!Pick::All.ranks());
    }

    #[dialog_common::test]
    fn it_reads_cardinality_keywords() {
        assert_eq!(Cardinality::from_keyword("many"), Some(Cardinality::Many));
        assert_eq!(Cardinality::from_keyword("Many"), Some(Cardinality::Many));
        assert_eq!(Cardinality::from_keyword("all"), None);
    }

    #[dialog_common::test]
    fn it_names_built_in_anchors() {
        assert_eq!(builtin("text"), Some("text:"));
        assert_eq!(builtin("unsigned-integer"), Some("natural:"));
        assert_eq!(builtin("max"), None, "a pick is no built-in anchor");
        assert_eq!(builtin("person"), None);
    }
}
