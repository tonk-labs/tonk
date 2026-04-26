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

    /// Override the URL where the sprite sheet is served. The
    /// renderer composes `{href}#{prefix}-{byte:02x}` for each
    /// cell's `mask-image`. Default: `/sigils.svg`.
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
        // 2×2 CSS grid of `<div>` cells, each painted with
        // `currentColor` and clipped to a glyph via `mask-image`
        // referencing a fragment of the external sprite. Cross-
        // document fragment-addressable `mask-image` is supported
        // in modern Safari/Chrome/Firefox; using SVG `<use>` against
        // an external sprite was problematic in WebKit because
        // internal `mask="url(#m-id)"` references resolved against
        // the consuming document.
        //
        // The wrapper is itself a `display:grid` block sized to its
        // CSS box (consumers set the box dimensions). `aspect-ratio`
        // keeps the sigil square when only one dimension is set.
        write!(
            f,
            "<div style=\"display:grid;grid-template-columns:1fr 1fr;grid-template-rows:1fr 1fr;width:100%;height:100%;aspect-ratio:1/1"
        )?;
        if let Some(fill) = &self.fill {
            write!(f, ";--sigil-fg:{fill};color:var(--sigil-fg)")?;
        }
        write!(f, "\">")?;

        // Sigil-js alternates prefix/suffix symbols across the 4 grid cells:
        // even-indexed bytes (0, 2) draw from the suffix table; odd-indexed
        // (1, 3) from the prefix table. The sprite sheet exposes both as
        // `sfx-XX` and `pfx-XX` IDs keyed by byte value.
        for (index, byte) in self.bits.iter().enumerate() {
            let prefix = if index.is_multiple_of(2) {
                "sfx"
            } else {
                "pfx"
            };
            write!(
                f,
                "<div style=\"background:currentColor;mask-image:url({href}#{prefix}-{byte:02x});mask-size:100% 100%;mask-repeat:no-repeat;mask-mode:luminance;-webkit-mask-image:url({href}#{prefix}-{byte:02x});-webkit-mask-size:100% 100%;-webkit-mask-repeat:no-repeat;-webkit-mask-source-type:luminance\"></div>",
                href = self.sprite_href,
            )?;
        }

        write!(f, "</div>")
    }
}
