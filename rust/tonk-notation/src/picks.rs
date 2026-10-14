//! The picks an attribute's `pick:` names, and the anchors the
//! notation spells them by.
//!
//! dialog names each pick by an entity (`last:`, `all:`, ...) and
//! still reads the plain names its 0.2 release wrote (`last`, `all`,
//! ...). The notation spells a pick by a built-in anchor, `pick: all`,
//! which resolves to that entity, as `as: text` resolves to `text:`. A
//! document that declares an anchor of the same name shadows the
//! built-in one within that document.

/// A pick, as dialog names it. The values a `top` ranks are the
/// attribute's `as:` list, not part of the pick's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pick {
    /// The newest claim: `last:`.
    Last,
    /// Every claim: `all:`.
    All,
    /// The best ranked of the listed values or relations: `top:`.
    Top,
    /// The greatest value: `max:`.
    Max,
    /// The least value: `min:`.
    Min,
}

impl Pick {
    /// Every pick.
    pub const ALL: [Pick; 5] = [Pick::Last, Pick::All, Pick::Top, Pick::Max, Pick::Min];

    /// The entity dialog names this pick by, such as `all:`.
    pub fn uri(self) -> &'static str {
        match self {
            Pick::Last => "last:",
            Pick::All => "all:",
            Pick::Top => "top:",
            Pick::Max => "max:",
            Pick::Min => "min:",
        }
    }

    /// The built-in anchor the notation spells this pick by, such as
    /// `all`, which is also the plain name dialog 0.2 wrote.
    pub fn anchor(self) -> &'static str {
        let uri = self.uri();
        &uri[..uri.len() - 1]
    }

    /// The pick a descriptor's `pick` names: its entity (`all:`), or
    /// the plain name dialog 0.2 wrote (`all`).
    pub fn from_wire(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|pick| pick.uri() == name || pick.anchor() == name)
    }

    /// The pick a built-in anchor names: `last`, `all`, `top`, `max` or
    /// `min`.
    pub fn from_anchor(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|pick| pick.anchor() == name)
    }
}

/// The entity a built-in anchor names: a type (`text` is `text:`) or a
/// pick (`all` is `all:`).
pub fn builtin(name: &str) -> Option<&'static str> {
    crate::ValueType::from_anchor(name)
        .map(crate::ValueType::uri)
        .or_else(|| Pick::from_anchor(name).map(Pick::uri))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn it_reads_both_wire_spellings() {
        assert_eq!(Pick::from_wire("all:"), Some(Pick::All));
        assert_eq!(Pick::from_wire("all"), Some(Pick::All));
        assert_eq!(Pick::from_wire("All"), None);
        assert_eq!(Pick::from_anchor("all:"), None);
    }

    #[dialog_common::test]
    fn it_names_built_in_anchors() {
        assert_eq!(builtin("text"), Some("text:"));
        assert_eq!(builtin("unsigned-integer"), Some("natural:"));
        assert_eq!(builtin("max"), Some("max:"));
        assert_eq!(builtin("person"), None);
    }
}
