//! Semantic colour tokens and the capability ladder.
//!
//! Colour in a terminal is tiered, so a token carries a hand-picked
//! value for *each* rung rather than being approximated downward from
//! one truecolor triple (`plan/tui-views.md` §6.5). At the bottom rung
//! a token resolves to an ANSI name, which means the user's own
//! terminal theme supplies the actual colour — usually better than a
//! literal, and the reason to push authors toward tokens.
//!
//! `stripes` is expressible here as the theme whose every token
//! resolves to `Color::Reset`: emphasis only, no colour codes emitted.
//! A colourless renderer could not express anything else, which is why
//! this exists rather than the constraint being baked in.

use ratatui::style::Color;

/// How much colour the terminal can take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    /// 24-bit.
    TrueColor,
    /// 256 indexed.
    Indexed,
    /// The 16 ANSI names, resolved by the user's terminal theme.
    Ansi,
    /// No colour at all — `NO_COLOR`, or not a tty.
    None,
}

impl Capability {
    /// Parse the `--colour` flag.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "truecolor" | "truecolour" | "24bit" => Ok(Self::TrueColor),
            "256" | "indexed" => Ok(Self::Indexed),
            "ansi" | "16" => Ok(Self::Ansi),
            "none" | "mono" => Ok(Self::None),
            other => Err(format!(
                "unknown colour level {other:?} (truecolor | 256 | ansi | none)"
            )),
        }
    }
}

/// One token's value at each rung of the ladder.
struct Token {
    name: &'static str,
    /// `(r, g, b)`.
    rgb: (u8, u8, u8),
    indexed: u8,
    ansi: Color,
}

/// The default palette. Warm stone and aubergine, matching the tonk
/// design language, with each rung chosen rather than computed.
const TOKENS: &[Token] = &[
    Token {
        name: "ink",
        rgb: (0xe2, 0xdf, 0xdd),
        indexed: 253,
        ansi: Color::White,
    },
    Token {
        name: "muted",
        rgb: (0x8a, 0x82, 0x86),
        indexed: 245,
        ansi: Color::DarkGray,
    },
    Token {
        name: "surface",
        rgb: (0x26, 0x1f, 0x20),
        indexed: 235,
        ansi: Color::Black,
    },
    Token {
        name: "accent",
        rgb: (0x55, 0x2e, 0x44),
        indexed: 96,
        ansi: Color::Magenta,
    },
    Token {
        name: "on-accent",
        rgb: (0xf7, 0xf6, 0xf5),
        indexed: 255,
        ansi: Color::White,
    },
    Token {
        name: "danger",
        rgb: (0xb0, 0x4a, 0x4a),
        indexed: 131,
        ansi: Color::Red,
    },
];

/// Resolves colour names against a capability rung.
pub struct Theme {
    capability: Capability,
}

impl Theme {
    /// A theme rendering for `capability`.
    pub fn new(capability: Capability) -> Self {
        Self { capability }
    }

    /// Resolve a token name or a `#rrggbb` literal.
    ///
    /// An unknown name resolves to `None` rather than to an arbitrary
    /// colour: a typo should be visible as "no colour applied", not as
    /// a wrong colour that looks deliberate.
    pub fn resolve(&self, value: &str) -> Option<Color> {
        if self.capability == Capability::None {
            return None;
        }
        if let Some(hex) = value.strip_prefix('#') {
            return self.literal(hex);
        }
        let token = TOKENS.iter().find(|token| token.name == value)?;
        Some(match self.capability {
            Capability::TrueColor => Color::Rgb(token.rgb.0, token.rgb.1, token.rgb.2),
            Capability::Indexed => Color::Indexed(token.indexed),
            Capability::Ansi => token.ansi,
            Capability::None => return None,
        })
    }

    /// A literal has no per-rung value to fall back on, so below
    /// truecolor it is approximated — which is the argument for tokens.
    fn literal(&self, hex: &str) -> Option<Color> {
        if hex.len() != 6 {
            return None;
        }
        let channel = |range: std::ops::Range<usize>| u8::from_str_radix(&hex[range], 16).ok();
        let (r, g, b) = (channel(0..2)?, channel(2..4)?, channel(4..6)?);
        Some(match self.capability {
            Capability::TrueColor => Color::Rgb(r, g, b),
            Capability::Indexed => Color::Indexed(nearest_cube(r, g, b)),
            Capability::Ansi => nearest_ansi(r, g, b),
            Capability::None => return None,
        })
    }
}

/// Nearest entry in the 6x6x6 colour cube (indices 16..232).
fn nearest_cube(r: u8, g: u8, b: u8) -> u8 {
    let step = |channel: u8| ((u16::from(channel) * 5 + 127) / 255) as u8;
    16 + 36 * step(r) + 6 * step(g) + step(b)
}

/// Nearest of the eight ANSI hues, by which channels are dominant.
fn nearest_ansi(r: u8, g: u8, b: u8) -> Color {
    let bright = |channel: u8| channel >= 0x80;
    match (bright(r), bright(g), bright(b)) {
        (false, false, false) => Color::Black,
        (true, false, false) => Color::Red,
        (false, true, false) => Color::Green,
        (true, true, false) => Color::Yellow,
        (false, false, true) => Color::Blue,
        (true, false, true) => Color::Magenta,
        (false, true, true) => Color::Cyan,
        (true, true, true) => Color::White,
    }
}
