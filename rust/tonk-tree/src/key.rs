//! Rendering the worker-decoded key components as colored chips, and
//! formatting fact values so their type reads from the value itself.
//!
//! The M3 key format is variable-length and schema-driven, so the *decoding*
//! lives on the worker (`dialog-reactor`'s `key_parts`): it hands us a list of
//! `{kind, text, hex}` components already split and textualized (a `did:key:…`
//! entity, a `db.meta/name` attribute, a typed value, a history `origin` /
//! `edition`). This module maps each component to a display [`Component`]
//! (label, color part, text with structural-byte glyphs) — no byte slicing.

use crate::model::KeyPart;

/// One component of a key, ready to render: its label (for a tooltip), which
/// [`Part`] it is (the CSS color class), and the displayed text plus the full
/// text for the detail pane.
pub struct Component {
    pub label: String,
    pub part: Part,
    pub text: String,
    pub full: String,
    /// The byte range this component occupies in the raw key, for the routing
    /// pivot. Derived from the parts' `hex` lengths in document order.
    pub bytes: std::ops::Range<usize>,
}

/// Which part of a key a component is — selects the `seg-<part>` CSS color.
/// entity → circle/blue, attribute → triangle/yellow, value & type →
/// square/red, structural (index/origin/edition/blob/spill) → neutral.
#[derive(Clone, Copy, PartialEq)]
pub enum Part {
    Entity,
    Attribute,
    ValueType,
    ValueRef,
    Structural,
    Unknown,
}

impl Part {
    /// The CSS class for this part — the stylesheet keys the color off it.
    pub fn class(self) -> &'static str {
        match self {
            Part::Entity => "seg-entity",
            Part::Attribute => "seg-attribute",
            Part::ValueType => "seg-vtype",
            Part::ValueRef => "seg-value",
            Part::Structural => "seg-index-type",
            Part::Unknown => "seg-unknown",
        }
    }
}

/// Map a worker `kind` to its color part and human label.
fn part_of(kind: &str) -> (Part, &'static str) {
    match kind {
        "index" => (Part::Structural, "Index"),
        "entity" => (Part::Entity, "Entity"),
        "attribute" => (Part::Attribute, "Attribute"),
        "vtype" => (Part::ValueType, "Value type"),
        "value" => (Part::ValueRef, "Value"),
        "spill" => (Part::ValueRef, "Spilled value (hash reference)"),
        "origin" => (Part::Structural, "Revision origin"),
        "edition" => (Part::Structural, "Revision edition"),
        "blob" => (Part::ValueRef, "Blob (content-addressed)"),
        "min" => (Part::Structural, "Leftmost subtree (no lower bound)"),
        "opaque" => (Part::Unknown, "Key"),
        _ => (Part::Unknown, "Key"),
    }
}

/// Substitute glyphs for non-printing / structural bytes so a UTF-8-ish
/// component reads as text: a NUL terminator becomes `␀`, other control bytes
/// become `·`. Printable text passes through unchanged. Used for the `text`
/// of entity/attribute chips (which are UTF-8 in the key) so a trailing `0x00`
/// separator is visible rather than an invisible gap.
fn glyphs(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\0' => '␀',
            c if c.is_control() => '·',
            c => c,
        })
        .collect()
}

/// The number of raw bytes a hex string represents (2 hex digits per byte).
fn hex_len(hex: &str) -> usize {
    hex.strip_prefix("0x").unwrap_or(hex).len() / 2
}

/// Map the worker-decoded parts into display components, threading each part's
/// byte offset (from its `hex` length) so the pivot logic can locate the
/// routing divergence. `text` gets glyph substitution; the index chip shows
/// its ordering name.
pub fn components(parts: &[KeyPart]) -> Vec<Component> {
    let mut out = Vec::with_capacity(parts.len());
    let mut offset = 0usize;
    for p in parts {
        let (part, label) = part_of(&p.kind);
        // The index and value-type chips carry a NAME in `hex` (not real
        // bytes) for the tooltip, and always span one byte (the tag). Every
        // other part's byte span comes from its `hex` length.
        let (len, label) = match p.kind.as_str() {
            "index" => (1usize, capitalize(&p.hex)),
            "vtype" => (1usize, format!("Value type: {}", p.hex)),
            _ => (hex_len(&p.hex).max(1), label.to_owned()),
        };
        let text = glyphs(&p.text);
        out.push(Component {
            label,
            part,
            full: text.clone(),
            text,
            bytes: offset..offset + len,
        });
        offset += len;
    }
    out
}

/// Title-case an ordering name for the index tooltip (`entity` → `Entity
/// index`).
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => format!("{}{} index", first.to_uppercase(), chars.as_str()),
        None => "Index".to_owned(),
    }
}

/// Decode a `0x`-prefixed hex key into raw bytes, for the pivot comparison.
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

/// The routing pivot: the index of the last byte that must stay bright for
/// this bound to be distinguishable from *both* its neighbors. A row's bright
/// prefix has to show where it diverges from the previous sibling (proving it
/// sorts after) AND from the next sibling (proving it sorts before) — the
/// pivot is the *max* of the two divergence points. Computed on the raw key
/// hex, so it is independent of how the components decode.
///
/// `prev` is the previous sibling's bound (or the all-zero minimum for a first
/// child); `next` is the next sibling's bound, if any.
pub fn pivot_byte(key: &str, prev: Option<&str>, next: Option<&str>) -> Option<usize> {
    let a = decode(key)?;
    let diverge = |other: &[u8]| -> Option<usize> {
        let n = a.len().min(other.len());
        (0..n).find(|&i| a[i] != other[i])
    };
    let lower = match prev {
        Some(p) => decode(p)?,
        None => vec![0u8; a.len()],
    };
    let from_prev = diverge(&lower);
    let from_next = next.and_then(decode).and_then(|n| diverge(&n));
    match (from_prev, from_next) {
        (Some(p), Some(n)) => Some(p.max(n)),
        (Some(p), None) => Some(p),
        (None, Some(n)) => Some(n),
        (None, None) => Some(a.len()),
    }
}

/// Format a fact value so its type is legible from the value itself:
///   string → quoted, float → always a `.`, signed → ±, bytes → hex.
/// `value` is the already-decoded JSON value the worker sent; `type_name` is
/// its dialog `ValueDataType` name.
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
        _ => match value {
            J::String(s) => s.clone(),
            other => other.to_string(),
        },
    }
}
