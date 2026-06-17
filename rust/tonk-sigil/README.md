# tonk-sigil

Generate 32-bit visual identifiers ("sigils") as SVG. Ports the rendering
logic of [`urbit/sigil-js`](https://github.com/urbit/sigil-js) to Rust, but
treats the input as arbitrary 32 bits, with no Urbit identity semantics and no
Feistel scrambling.

Every 32-bit value maps to a deterministic 2×2 tile of glyphs. Useful for
giving hashes, user IDs, document IDs, etc. a memorable visual handle.

## Quick start

```rust
use tonk_sigil::Sigil;

let svg: String = Sigil::from(0xdeadbeef_u32).render();
// <svg ...><use href="/sigils.svg#sfx-de" .../>...</svg>
```

The returned SVG references a sprite sheet. Serve
[`assets/sigils.svg`](assets/sigils.svg) at the URL the sigil expects (the
default is `/sigils.svg` at the document root).

## Inputs

```rust
Sigil::from(0xdeadbeef_u32);           // u32, big-endian byte order
Sigil::from([0xde, 0xad, 0xbe, 0xef]); // explicit bytes

// Truncate a longer value (e.g. a hash prefix)
let hash = blake3::hash(b"hello world");
let prefix: [u8; 4] = hash.as_bytes()[..4].try_into().unwrap();
Sigil::from(prefix);
```

Anything wider than 32 bits is your problem to truncate; this crate has no
opinion on how. The recommended pattern is "take the first 4 bytes of
whatever identifier you already have."

## Customization

```rust
Sigil::from(0xdeadbeef_u32)
    .fill("black")                     // glyph color, default `currentColor`
    .sprite_href("/assets/sigils.svg") // where to find the sprite sheet
    .render();
```

`fill` accepts any CSS color value (`"black"`, `"#fff"`, `"rgb(1,2,3)"`,
`"var(--my-color)"`, …). Sigils are single-color: interior holes and
contrast lines are true SVG cutouts via a `<mask>`, so the glyph is
transparent wherever it isn't the fill color.

The rendered SVG has no intrinsic pixel size; it scales to fill its
parent container. Size the element from CSS:

```css
tonk-sigil { width: 2rem; height: 2rem; display: inline-block; }
```

### Theming with CSS

The sprite's fill references `var(--sigil-fg, currentColor)`. Either set
`--sigil-fg` on any ancestor, or just use `color:` directly: the default
`currentColor` fallback means sigils inherit surrounding text color:

```css
.sidebar { color: white; }      /* all sigils in sidebar render white */
:root    { --sigil-fg: black; } /* or override via the variable */
```

## Web component

Enable the `web` feature to get a `<tonk-sigil>` custom element.

```toml
[dependencies]
tonk-sigil = { version = "0.1", features = ["web"] }
```

```rust
// Call once at app startup
tonk_sigil::Sigil::install();
```

Now anywhere in the DOM:

```html
<!-- Render from an explicit 32-bit value -->
<tonk-sigil value="3735928559"></tonk-sigil>
<tonk-sigil value="0xdeadbeef"></tonk-sigil>

<!-- Render from arbitrary text (hashed with blake3, first 4 bytes) -->
<tonk-sigil>my-repo-name</tonk-sigil>
<tonk-sigil>alice@example.com</tonk-sigil>

<!-- Styling attributes -->
<tonk-sigil value="0xdeadbeef" fill="purple"></tonk-sigil>

<!-- Override the sprite sheet location -->
<tonk-sigil value="0xdeadbeef" sprite="/static/sigils.svg"></tonk-sigil>
```

Size is set via CSS (`tonk-sigil { width: 2rem; height: 2rem }`), not as
an attribute.

Resolution rules, in order:
1. If `value` parses as a u32 (decimal, or `0x`-prefixed hex), use it.
2. Otherwise hash the element's text content with blake3 and use the first
   4 bytes.
3. Empty element with no `value` renders as if the input were zero.

The element observes `value`, `fill`, and `sprite` and re-renders on
change. Text content is preserved across re-renders: the
sigil is inserted as a child `<span data-sigil>` rather than replacing
the element's contents.

## Serving the sprite sheet

The crate ships `assets/sigils.svg`, which you need to serve at the
URL your sigils reference (default `/sigils.svg`). With Trunk, the simplest
setup is to copy it into your app's asset directory and add:

```html
<link data-trunk rel="copy-file" href="./assets/sigils.svg" />
```

Or point sigils somewhere else with `.sprite_href(…)` / the `sprite=""`
attribute.

## Regenerating the sprite sheet

`scripts/gen-sprites.mjs` regenerates `assets/sigils.svg` from a local
clone of `urbit/sigil-js`:

```bash
git clone https://github.com/urbit/sigil-js /tmp/sigil-js
node scripts/gen-sprites.mjs /tmp/sigil-js
```

Rerun this if sigil-js publishes new glyphs or if you want to adjust the
CSS variable names used for theming. The generated SVG is checked into the
repo; you do not need to run it during normal builds.

## What changed from sigil-js

- **Input is bytes, not strings.** No `@p` parsing, no Urbit ID format.
- **No Feistel obfuscation.** If you pass the same bytes twice, you get
  the same sigil. (Sigil-js intentionally scrambled the input to hide
  parent-child identity relationships, which is not relevant here.)
- **Sprite sheet instead of inline SVG.** The 275KB of glyph path data
  lives in a separate file that browsers can cache. The crate itself is
  tiny; the rendered SVG is short.
- **Real transparency, not painted holes.** Sigil-js renders interior
  "holes" by painting them in the background color; our sprites use
  an SVG `<mask>` so cutouts are truly transparent and the glyph
  composites cleanly over any surface.
- **Single-color API.** Sigil-js took a foreground and a background
  color (the latter being both the tile background *and* the "hole"
  paint); we take only a fill.
- **CSS-first theming.** Glyph color inherits from `color:` via
  `currentColor`, or overridable through `--sigil-fg`.
- **No background rectangle.** The SVG is transparent by default. If
  you want a filled tile, wrap the element in a styled container.
- **No planet/star/galaxy distinction.** All sigils are 2×2 tiles of 32
  bits. Narrower inputs are not supported.
