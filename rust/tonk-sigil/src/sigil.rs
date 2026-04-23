use std::borrow::Cow;
use std::fmt;

pub struct Sigil {
    bits: [u8; 4],
    fill: Option<Cow<'static, str>>,
    sprite_href: Cow<'static, str>,
}

impl Sigil {
    /// Override the glyph color. Defaults to inheriting from the
    /// surrounding CSS `color` property via `currentColor`. Any CSS
    /// color works, including `var(--custom)` references.
    pub fn fill(mut self, color: impl Into<Cow<'static, str>>) -> Self {
        self.fill = Some(color.into());
        self
    }

    /// Override the URL where the sprite sheet is served.
    pub fn sprite_href(mut self, href: impl Into<Cow<'static, str>>) -> Self {
        self.sprite_href = href.into();
        self
    }

    pub fn render(&self) -> String {
        self.to_string()
    }
}

impl Default for Sigil {
    fn default() -> Self {
        Self {
            bits: [0; 4],
            fill: None,
            sprite_href: Cow::Borrowed("/sigils.svg"),
        }
    }
}

impl From<u32> for Sigil {
    fn from(n: u32) -> Self {
        Sigil {
            bits: n.to_be_bytes(),
            ..Self::default()
        }
    }
}

impl From<[u8; 4]> for Sigil {
    fn from(bits: [u8; 4]) -> Self {
        Sigil {
            bits,
            ..Self::default()
        }
    }
}

impl fmt::Display for Sigil {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 128-unit viewBox with no intrinsic pixel size — the SVG
        // scales to fill its CSS box.
        //
        // `--sigil-fg` is emitted inline only when the caller set it;
        // otherwise ancestor CSS cascades through to the sprite paths,
        // which use `var(--sigil-fg, currentColor)` as the fallback.
        write!(
            f,
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 128 128\" preserveAspectRatio=\"xMidYMid meet\" style=\"display:block;width:100%;height:100%"
        )?;
        if let Some(fill) = &self.fill {
            write!(f, ";--sigil-fg:{fill}")?;
        }
        write!(f, "\">")?;

        // Sigil-js alternates prefix/suffix symbols across the 4 grid cells:
        // even-indexed bytes (0, 2) draw from the suffix table; odd-indexed
        // (1, 3) from the prefix table. The sprite sheet exposes both as
        // `sfx-XX` and `pfx-XX` IDs keyed by byte value.
        for (index, byte) in self.bits.iter().enumerate() {
            let x = (index as u32 % 2) * 64;
            let y = (index as u32 / 2) * 64;
            let prefix = if index.is_multiple_of(2) {
                "sfx"
            } else {
                "pfx"
            };
            write!(
                f,
                "<use href=\"{href}#{prefix}-{byte:02x}\" transform=\"translate({x} {y}) scale(0.5)\"/>",
                href = self.sprite_href,
            )?;
        }

        write!(f, "</svg>")
    }
}
