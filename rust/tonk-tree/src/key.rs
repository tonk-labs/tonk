//! Decoding a composite index key into its colored components, and
//! formatting fact values so their type reads from the value itself.
//!
//! A key is the 162 bytes carried as a `0x<hex>` string. The byte layout
//! depends on the leading tag byte (the index ordering): entity, attribute,
//! and value index keys place the parts at different offsets (see `layout`).

/// One component of a key: its label (for a tooltip), which `Part` of the
/// key it is (selects the CSS class / background color), and the short
/// displayed text plus the full value.
pub struct Component {
    pub label: String,
    pub part: Part,
    pub text: String,
    pub full: String,
}

/// Which part of the composite key a segment is. Each maps to a CSS class
/// (`seg-<part>`) carrying that part's Bauhaus background color, so all
/// styling lives in the stylesheet, not inline: entity → circle/blue,
/// attribute → triangle/yellow, value-type & value-ref → square/red. The
/// index-type chip reuses the matching index part's color (via `tag_fill`)
/// and `append_key` adds `seg-index-type` for its chip shape.
#[derive(Clone, Copy, PartialEq)]
pub enum Part {
    Entity,
    Attribute,
    ValueType,
    ValueRef,
    Unknown,
}

impl Part {
    /// The CSS class for this part — `seg-entity`, `seg-attribute`, etc.
    /// The stylesheet keys the background color off it.
    pub fn class(self) -> &'static str {
        match self {
            Part::Entity => "seg-entity",
            Part::Attribute => "seg-attribute",
            Part::ValueType => "seg-vtype",
            Part::ValueRef => "seg-value",
            Part::Unknown => "seg-unknown",
        }
    }
}

/// The index ordering a key belongs to, from its tag byte.
pub fn tag_of(byte: u8) -> &'static str {
    match byte {
        0 => "entity",
        1 => "attribute",
        2 => "value",
        _ => "unknown",
    }
}

/// The index-type chip's tooltip label — `Entity Index`, `Attribute
/// Index`, `Value Index`.
fn index_label(byte: u8) -> String {
    match byte {
        0 => "Entity Index",
        1 => "Attribute Index",
        2 => "Value Index",
        _ => "Unknown Index",
    }
    .to_owned()
}

/// Decode a `0x`-prefixed hex key into bytes. Keys travel as hex (they
/// are 162 bytes — too long for base58 decode buffers). Returns `None`
/// on malformed input.
fn decode(key: &str) -> Option<Vec<u8>> {
    let raw = key.strip_prefix("0x").unwrap_or(key);
    if !raw.len().is_multiple_of(2) {
        return None;
    }
    (0..raw.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(raw.get(i..i + 2)?, 16).ok())
        .collect()
}

/// Lowercase hex of a byte slice. Hex is order-preserving — the chips'
/// left-to-right order then reflects the key's actual lexicographic sort
/// (base58 would hide it, since base58 string order ≠ byte order).
fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn trunc(s: &str, head: usize, tail: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() > head + tail + 1 {
        let h: String = chars[..head].iter().collect();
        let t: String = chars[chars.len() - tail..].iter().collect();
        format!("{h}…{t}")
    } else {
        s.to_owned()
    }
}

const TYPE_NAMES: [&str; 9] = [
    "Bytes",
    "Entity",
    "Boolean",
    "Text",
    "UnsignedInt",
    "SignedInt",
    "Float",
    "Record",
    "Symbol",
];

fn type_name(byte: u8) -> &'static str {
    TYPE_NAMES.get(byte as usize).copied().unwrap_or("?")
}

/// A human, kebab-cased value-type name for tooltips
/// (`SignedInt` → `signed-integer`).
fn type_label(byte: u8) -> &'static str {
    match byte {
        0 => "bytes",
        1 => "entity",
        2 => "boolean",
        3 => "text",
        4 => "unsigned-integer",
        5 => "signed-integer",
        6 => "float",
        7 => "record",
        8 => "symbol",
        _ => "unknown",
    }
}

/// Where each part sits in the 162 key bytes for a given index tag. The
/// byte order is index-dependent (the sort key puts the leading part
/// first), so the offsets shift per tag — matching dialog-artifacts'
/// `key/{entity,attribute,value}.rs`.
struct Layout {
    entity: usize,
    attribute: usize,
    value_type: usize,
    value_ref: usize,
}

fn layout(tag: u8) -> Layout {
    match tag {
        // value : [tag][type 1][value 32][attribute 64][entity 64]
        2 => Layout {
            value_type: 1,
            value_ref: 2,
            attribute: 34,
            entity: 98,
        },
        // attribute : [tag][attribute 64][entity 64][type 1][value 32]
        1 => Layout {
            attribute: 1,
            entity: 65,
            value_type: 129,
            value_ref: 130,
        },
        // entity : [tag][entity 64][attribute 64][type 1][value 32]
        _ => Layout {
            entity: 1,
            attribute: 65,
            value_type: 129,
            value_ref: 130,
        },
    }
}

/// Decode a key into its ordered, labeled, colored components. The tag
/// segment is first and always present; the rest follow the index's byte
/// order so the sort prefix reads left to right.
pub fn components(key: &str) -> Vec<Component> {
    let Some(bytes) = decode(key) else {
        return vec![Component {
            label: "key".into(),
            part: Part::Unknown,
            text: key.into(),
            full: key.into(),
        }];
    };
    if bytes.len() < 162 {
        return vec![Component {
            label: "key".into(),
            part: Part::Unknown,
            text: hex(&bytes),
            full: key.into(),
        }];
    }

    let tag = bytes[0];
    let l = layout(tag);
    let slice = |off: usize, len: usize| hex(&bytes[off..off + len]);

    let entity = slice(l.entity, 64);
    let attribute = slice(l.attribute, 64);
    let value_ref = slice(l.value_ref, 32);
    let type_byte = bytes[l.value_type];

    let entity_seg = Component {
        label: "Entity".into(),
        part: Part::Entity,
        text: trunc(&entity, 10, 4),
        full: entity,
    };
    let attribute_seg = Component {
        label: "Attribute".into(),
        part: Part::Attribute,
        text: trunc(&attribute, 10, 4),
        full: attribute,
    };
    // The value-type chip: its content is the type byte; the human type
    // name is in the tooltip. It shares the value background so it reads as
    // the value's type.
    let type_seg = Component {
        label: format!("Value type: {}", type_label(type_byte)),
        part: Part::ValueType,
        text: type_byte.to_string(),
        full: format!("{type_byte} ({})", type_name(type_byte)),
    };
    let value_seg = Component {
        label: "Value".into(),
        part: Part::ValueRef,
        text: trunc(&value_ref, 10, 4),
        full: value_ref,
    };
    // The index-type chip is neutral (mode-inverse background via
    // `seg-index-type`); its content is the tag byte (0/1/2).
    let index_seg = Component {
        label: index_label(tag),
        part: Part::Unknown,
        text: tag.to_string(),
        full: format!("{tag} ({})", tag_of(tag)),
    };

    match tag {
        2 => vec![index_seg, type_seg, value_seg, attribute_seg, entity_seg],
        1 => vec![index_seg, attribute_seg, entity_seg, type_seg, value_seg],
        _ => vec![index_seg, entity_seg, attribute_seg, type_seg, value_seg],
    }
}

/// How many leading components of `key` are identical to `prev` (the
/// previous sibling). Those are the shared prefix — front coding dims
/// them so only the divergent tail stands out.
pub fn shared_prefix_len(key: &str, prev: Option<&str>) -> usize {
    let Some(prev) = prev else { return 0 };
    let a = components(key);
    let b = components(prev);
    let mut n = 0;
    while n < a.len() && n < b.len() && a[n].full == b[n].full {
        n += 1;
    }
    n
}

/// Format a fact value so its type is legible from the value itself
/// (entities underlined elsewhere; here we shape the text):
///   string → quoted, float → always a `.`, signed → ±, bytes → hex.
/// `value` is the already-decoded JSON value the worker sent; `type_name`
/// is its dialog `ValueDataType` name.
pub fn format_value(value: &serde_json::Value, type_name: &str) -> String {
    use serde_json::Value as J;
    match type_name {
        "Text" => match value {
            J::String(s) => format!("\"{s}\""),
            other => other.to_string(),
        },
        "Float" => match value {
            J::Number(n) => {
                let f = n.as_f64().unwrap_or(0.0);
                if f.fract() == 0.0 {
                    format!("{f:.1}")
                } else {
                    f.to_string()
                }
            }
            other => other.to_string(),
        },
        "SignedInt" => match value {
            J::Number(n) => {
                let i = n.as_i64().unwrap_or(0);
                if i >= 0 {
                    format!("+{i}")
                } else {
                    i.to_string()
                }
            }
            J::String(s) => s.clone(),
            other => other.to_string(),
        },
        "Bytes" => match value {
            J::String(s) => s.clone(),
            J::Array(a) => a
                .iter()
                .filter_map(|x| x.as_u64())
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .join(" "),
            other => other.to_string(),
        },
        // Entity / Symbol / UnsignedInt / Boolean / Record: plain.
        _ => match value {
            J::String(s) => s.clone(),
            other => other.to_string(),
        },
    }
}
