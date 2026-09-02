//! An elm-ui-shaped layout algebra, solved in whole terminal cells.
//!
//! The authoring surface is elm-ui's: every element has a [`Length`] on
//! each axis (`px` / `shrink` / `fill`), there is **no margin** — only
//! [`Style::pad`] (edge to content) and [`Style::spacing`] (between
//! children) — and alignment is declared on the *child* and interpreted
//! by the parent. The engine underneath is `taffy`, run at one CSS pixel
//! per terminal cell, which is the arrangement `ink` reaches via yoga.
//!
//! Nothing here knows about tonk, about templates, or about a terminal
//! backend: the input is an [`Element`] tree, the output is a [`Laid`]
//! tree of integer [`Rect`]s. That is what makes the engine swappable
//! (see `plan/tui-views.md` §6.3) and the tests cheap.
//!
//! ```
//! use tonk_layout::{Element, Length, Rect, layout};
//!
//! let tree = Element::row(vec![
//!     Element::text("left"),
//!     Element::text("right").width(Length::Fill(1)),
//! ]);
//! let laid = layout(&tree, Rect::new(0, 0, 20, 1));
//! assert_eq!(laid.children[0].rect.width, 4);
//! ```

#![forbid(unsafe_code)]

mod measure;
mod solve;
mod style;

pub use measure::{text_width, wrap};
pub use style::{AlignX, AlignY, Edges, Emphasis, Length, Style};

use std::collections::BTreeMap;

/// A rectangle in whole terminal cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    /// Column of the left edge.
    pub x: u16,
    /// Row of the top edge.
    pub y: u16,
    /// Width in cells.
    pub width: u16,
    /// Height in cells (rows).
    pub height: u16,
}

impl Rect {
    /// A rectangle at `(x, y)` sized `width` x `height`.
    pub fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// What an element *is*, as far as layout is concerned.
///
/// This is deliberately smaller than the template vocabulary: a
/// `<keybar>` and a `<box>` are both [`Kind::El`] here, and only the
/// painter cares about the difference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// Lays children out left to right.
    Row,
    /// Lays children out top to bottom.
    Column,
    /// A single-child container — elm-ui's `el`. Also the fallback for
    /// any tag the vocabulary does not recognise.
    El,
    /// A leaf of text, measured as one line.
    Text(String),
    /// A leaf of text that wraps: its height depends on its width.
    Paragraph(String),
}

/// One node of the input tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    /// What it is.
    pub kind: Kind,
    /// How it is sized, padded, aligned and emphasised.
    pub style: Style,
    /// Attributes the painter needs but layout does not (`onclick`,
    /// `data-*`, `title`, …). Carried through so the caller does not
    /// have to keep a parallel tree.
    pub attrs: BTreeMap<String, String>,
    /// Child nodes. Always empty for the text kinds.
    pub children: Vec<Element>,
}

impl Element {
    /// A node of `kind` with default style and no children.
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            style: Style::default(),
            attrs: BTreeMap::new(),
            children: Vec::new(),
        }
    }

    /// A single-line text leaf.
    pub fn text(content: impl Into<String>) -> Self {
        Self::new(Kind::Text(content.into()))
    }

    /// A wrapping text leaf.
    pub fn paragraph(content: impl Into<String>) -> Self {
        Self::new(Kind::Paragraph(content.into()))
    }

    /// A horizontal container.
    pub fn row(children: Vec<Element>) -> Self {
        let mut el = Self::new(Kind::Row);
        el.children = children;
        el
    }

    /// A vertical container.
    pub fn column(children: Vec<Element>) -> Self {
        let mut el = Self::new(Kind::Column);
        el.children = children;
        el
    }

    /// A single-child container.
    pub fn el(child: Element) -> Self {
        let mut el = Self::new(Kind::El);
        el.children = vec![child];
        el
    }

    /// Set the horizontal [`Length`].
    pub fn width(mut self, length: Length) -> Self {
        self.style.width = length;
        self
    }

    /// Set the vertical [`Length`].
    pub fn height(mut self, length: Length) -> Self {
        self.style.height = length;
        self
    }

    /// Replace the whole style.
    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }
}

/// One node of the output tree: an [`Element`] plus its resolved
/// rectangle, in absolute cell coordinates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Laid {
    /// Where it landed, absolute, in whole cells.
    pub rect: Rect,
    /// What it is.
    pub kind: Kind,
    /// The style it was laid out with — the painter needs the emphasis
    /// and colour fields, which layout ignores.
    pub style: Style,
    /// Passed-through non-layout attributes.
    pub attrs: BTreeMap<String, String>,
    /// For [`Kind::Paragraph`], the text already broken to the resolved
    /// width, so the painter never re-wraps and cannot disagree with the
    /// height layout reserved.
    pub lines: Vec<String>,
    /// Laid-out children.
    pub children: Vec<Laid>,
}

/// Lay `root` out inside `viewport` and return the resolved tree.
pub fn layout(root: &Element, viewport: Rect) -> Laid {
    solve::layout(root, viewport)
}
