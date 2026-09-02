//! The elm-ui attribute surface: lengths, padding, spacing, alignment,
//! and the emphasis/colour fields layout carries but does not read.

/// How an element is sized on one axis — elm-ui's `Length`.
///
/// There is no `auto`: every element has a length on every axis, and
/// [`Length::Shrink`] (elm-ui's default on `el`, `row` and `column`) is
/// the content-sized case. That default is the reason this crate needs a
/// content-measuring engine at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Length {
    /// Exactly this many cells.
    Px(u16),
    /// As small as the content allows. elm-ui's default on every
    /// container, which is why this engine has to measure content.
    #[default]
    Shrink,
    /// Share the free space, weighted. `Fill(1)` is elm-ui's `fill`;
    /// `Fill(n)` is `fillPortion n`.
    Fill(u16),
}

/// Per-edge cell counts, for padding and borders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Edges {
    /// Cells above.
    pub top: u16,
    /// Cells to the right.
    pub right: u16,
    /// Cells below.
    pub bottom: u16,
    /// Cells to the left.
    pub left: u16,
}

impl Edges {
    /// `x` cells left and right, `y` cells top and bottom.
    ///
    /// Terminal cells are roughly 1:2, so a *visually* even inset is
    /// about `xy(2, 1)`, not `xy(1, 1)`. Callers that expose a single
    /// `pad=n` attribute should decide which they mean and say so;
    /// this crate does not guess.
    pub fn xy(x: u16, y: u16) -> Self {
        Self {
            top: y,
            right: x,
            bottom: y,
            left: x,
        }
    }

    /// The same count on every edge.
    pub fn all(n: u16) -> Self {
        Self::xy(n, n)
    }
}

/// Horizontal alignment, declared on the child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignX {
    /// Against the left edge.
    Left,
    /// Centred horizontally.
    Center,
    /// Against the right edge.
    Right,
}

/// Vertical alignment, declared on the child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignY {
    /// Against the top edge.
    Top,
    /// Centred vertically.
    Center,
    /// Against the bottom edge.
    Bottom,
}

/// SGR emphasis, orthogonal to colour and available on every terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Emphasis {
    /// SGR 1.
    pub bold: bool,
    /// SGR 2.
    pub dim: bool,
    /// SGR 7 — the "plate" treatment.
    pub reverse: bool,
    /// SGR 4.
    pub underline: bool,
}

/// Everything an element declares. Layout reads the geometry fields;
/// `fg` / `bg` / `emphasis` ride along for the painter.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Style {
    /// Horizontal length.
    pub width: Length,
    /// Vertical length.
    pub height: Length,
    /// Lower bound on width, in cells.
    pub min_width: Option<u16>,
    /// Upper bound on width, in cells.
    pub max_width: Option<u16>,
    /// Lower bound on height, in cells.
    pub min_height: Option<u16>,
    /// Upper bound on height, in cells.
    pub max_height: Option<u16>,
    /// Distance from the element's edge to its content.
    pub pad: Edges,
    /// Space between children, horizontally then vertically.
    pub spacing: (u16, u16),
    /// A one-cell drawn border, which participates in layout exactly
    /// like padding. Terminals have no sub-cell strokes, so this is a
    /// flag, not a width.
    pub border: bool,
    /// Horizontal alignment of this element within its parent.
    pub align_x: Option<AlignX>,
    /// Vertical alignment of this element within its parent.
    pub align_y: Option<AlignY>,
    /// Wrap children onto new lines when they overflow the main axis
    /// (elm-ui's `wrappedRow`).
    pub wrap: bool,
    /// Clip descendants to this element's content box.
    ///
    /// Nothing shrinks below its content — elm-ui has no such state, so
    /// neither does this — which means a subtree can be larger than the
    /// space it was given. A terminal's answer to that is to clip, and
    /// this says where. Layout itself ignores the flag; it is the
    /// painter that honours it.
    pub clip: bool,
    /// Foreground colour token or literal, resolved by the theme.
    pub fg: Option<String>,
    /// Background colour token or literal, resolved by the theme.
    pub bg: Option<String>,
    /// SGR emphasis.
    pub emphasis: Emphasis,
}
