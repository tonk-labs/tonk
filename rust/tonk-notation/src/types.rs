//! The value types an attribute's `as:` names, and the anchors the
//! notation spells them by.
//!
//! dialog names each type by an entity (`text:`, `integer:`, ...) and
//! still reads the names an earlier release wrote (`Text`,
//! `SignedInteger`, ...). The notation spells a type by a built-in
//! anchor, `as: text`, which resolves to that entity; `signed-integer`
//! and `unsigned-integer` are kept as aliases of `integer` and
//! `natural`. A document that declares an anchor of the same name
//! shadows the built-in one within that document.

/// A value type, as dialog names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValueType {
    /// UTF-8 text: `text:`.
    Text,
    /// A signed integer: `integer:`.
    Integer,
    /// An unsigned integer: `natural:`.
    Natural,
    /// A floating point number: `float:`.
    Float,
    /// `true` or `false`: `boolean:`.
    Boolean,
    /// A byte buffer: `bytes:`.
    Bytes,
    /// An entity URI: `entity:`.
    Entity,
    /// A symbol: `symbol:`.
    Symbol,
    /// A structured record: `record:`.
    Record,
}

impl ValueType {
    /// Every type.
    pub const ALL: [ValueType; 9] = [
        ValueType::Text,
        ValueType::Integer,
        ValueType::Natural,
        ValueType::Float,
        ValueType::Boolean,
        ValueType::Bytes,
        ValueType::Entity,
        ValueType::Symbol,
        ValueType::Record,
    ];

    /// The entity dialog names this type by, such as `text:`.
    pub fn uri(self) -> &'static str {
        match self {
            ValueType::Text => "text:",
            ValueType::Integer => "integer:",
            ValueType::Natural => "natural:",
            ValueType::Float => "float:",
            ValueType::Boolean => "boolean:",
            ValueType::Bytes => "bytes:",
            ValueType::Entity => "entity:",
            ValueType::Symbol => "symbol:",
            ValueType::Record => "record:",
        }
    }

    /// The built-in anchor the notation spells this type by, such as
    /// `text`.
    pub fn anchor(self) -> &'static str {
        let uri = self.uri();
        &uri[..uri.len() - 1]
    }

    /// The name the release before type entities wrote for this type.
    fn legacy_name(self) -> &'static str {
        match self {
            ValueType::Text => "Text",
            ValueType::Integer => "SignedInteger",
            ValueType::Natural => "UnsignedInteger",
            ValueType::Float => "Float",
            ValueType::Boolean => "Boolean",
            ValueType::Bytes => "Bytes",
            ValueType::Entity => "Entity",
            ValueType::Symbol => "Symbol",
            ValueType::Record => "Record",
        }
    }

    /// The type a descriptor's `as` names: its entity (`text:`), or
    /// the name an earlier release wrote (`Text`).
    pub fn from_wire(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.uri() == name || kind.legacy_name() == name)
    }

    /// The type a built-in anchor names: `text`, `integer`, ..., and the
    /// aliases `signed-integer` and `unsigned-integer`.
    pub fn from_anchor(name: &str) -> Option<Self> {
        match name {
            "signed-integer" => Some(ValueType::Integer),
            "unsigned-integer" => Some(ValueType::Natural),
            _ => Self::ALL.into_iter().find(|kind| kind.anchor() == name),
        }
    }

    /// Every built-in anchor, aliases included, each with the type it
    /// names.
    pub fn anchors() -> impl Iterator<Item = (&'static str, ValueType)> {
        Self::ALL
            .into_iter()
            .map(|kind| (kind.anchor(), kind))
            .chain([
                ("signed-integer", ValueType::Integer),
                ("unsigned-integer", ValueType::Natural),
            ])
    }

    /// Whether this type is a number.
    pub fn is_numeric(self) -> bool {
        matches!(
            self,
            ValueType::Integer | ValueType::Natural | ValueType::Float
        )
    }
}

/// The widths a sized number type may have, by its kind.
const NATURAL_WIDTHS: [&str; 5] = ["8", "16", "32", "64", "128"];
const FLOAT_WIDTHS: [&str; 2] = ["32", "64"];

/// The entity dialog names a type by, from any spelling the notation
/// accepts: a built-in anchor (`text`, `natural`), an alias
/// (`unsigned-integer`), the entity itself (`natural:`), the name an
/// earlier release wrote (`UnsignedInteger`), `number` (every number),
/// or a sized number type by its entity, `natural:64`, `integer:8`,
/// `float:32`. `None` for anything else.
pub fn type_uri(spelling: &str) -> Option<String> {
    if let Some(kind) = ValueType::from_wire(spelling).or_else(|| ValueType::from_anchor(spelling))
    {
        return Some(kind.uri().to_string());
    }
    if spelling == "number" || spelling == "number:" {
        return Some("number:".to_string());
    }
    let (kind, width) = spelling.split_once(':')?;
    let widths: &[&str] = match kind {
        "natural" | "integer" => &NATURAL_WIDTHS,
        "float" => &FLOAT_WIDTHS,
        _ => return None,
    };
    widths.contains(&width).then(|| format!("{kind}:{width}"))
}

/// The anchor the notation spells a type's entity by: the entity less
/// its trailing colon (`text:` is `text`), and a sized type as it is
/// (`natural:64`).
pub fn type_anchor(uri: &str) -> &str {
    uri.strip_suffix(':').unwrap_or(uri)
}

#[cfg(test)]
mod sized_tests {
    use super::*;

    #[dialog_common::test]
    fn it_names_every_spelling_of_a_type() {
        assert_eq!(type_uri("text").as_deref(), Some("text:"));
        assert_eq!(type_uri("text:").as_deref(), Some("text:"));
        assert_eq!(type_uri("Text").as_deref(), Some("text:"));
        assert_eq!(type_uri("unsigned-integer").as_deref(), Some("natural:"));
        assert_eq!(type_uri("number").as_deref(), Some("number:"));
        assert_eq!(type_uri("natural:64").as_deref(), Some("natural:64"));
        assert_eq!(type_uri("integer:8").as_deref(), Some("integer:8"));
        assert_eq!(type_uri("float:32").as_deref(), Some("float:32"));
        assert_eq!(type_uri("float:8"), None);
        assert_eq!(type_uri("natural:7"), None);
        assert_eq!(type_uri("text:8"), None);
    }

    #[dialog_common::test]
    fn it_spells_a_type_by_its_anchor() {
        assert_eq!(type_anchor("text:"), "text");
        assert_eq!(type_anchor("natural:64"), "natural:64");
        assert_eq!(type_anchor("number:"), "number");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn it_reads_both_wire_spellings() {
        assert_eq!(ValueType::from_wire("text:"), Some(ValueType::Text));
        assert_eq!(ValueType::from_wire("Text"), Some(ValueType::Text));
        assert_eq!(
            ValueType::from_wire("UnsignedInteger"),
            Some(ValueType::Natural)
        );
        assert_eq!(ValueType::from_wire("integer:"), Some(ValueType::Integer));
        assert_eq!(ValueType::from_wire("text"), None);
    }

    #[dialog_common::test]
    fn it_resolves_anchors_and_aliases() {
        assert_eq!(ValueType::from_anchor("text"), Some(ValueType::Text));
        assert_eq!(
            ValueType::from_anchor("signed-integer"),
            Some(ValueType::Integer)
        );
        assert_eq!(
            ValueType::from_anchor("unsigned-integer"),
            Some(ValueType::Natural)
        );
        assert_eq!(ValueType::from_anchor("text:"), None);
        assert_eq!(ValueType::anchors().count(), 11);
    }
}
