//! Lowering the elm-ui algebra onto `taffy`, at one CSS pixel per cell.
//!
//! Three translations carry the weight, and each is a place the two
//! models do not line up on their own:
//!
//! 1. **`Length` onto the right flexbox property, per axis.** On an
//!    element's *main* axis `Fill(n)` is `flex-grow: n` over a zero
//!    basis, which is what makes elm-ui's "available space split evenly
//!    between the fills" true. On its *cross* axis flex-grow does not
//!    apply at all, so `Fill` becomes `align-self: stretch`.
//! 2. **Main-axis alignment by inserted spacers.** In elm-ui an aligned
//!    child *pushes* its siblings — `[el, alignLeft, centerX,
//!    alignRight]` renders `|-|-|    |-|    |-|`. That is not
//!    `align-self`, and no single `justify-content` value produces it.
//!    Grouping the children by alignment and growing a spacer between
//!    the groups does, exactly.
//! 3. **Whole cells.** `taffy` computes in `f32` and rounds *cumulative*
//!    positions rather than individual sizes, so adjacent boxes cannot
//!    gap or overlap by one cell. We keep that pass on and only cast at
//!    the end.

use taffy::prelude::*;
use taffy::{LayoutInput, LayoutOutput, Size as TaffySize, TaffyTree};

use crate::measure::{text_width, wrap};
use crate::style::{AlignX, AlignY, Length};
use crate::{Element, Kind, Laid, Rect};

/// What a leaf carries into the measure function.
#[derive(Debug, Clone)]
struct LeafText {
    text: String,
    wrapping: bool,
}

/// The taffy tree we built, remembering which source child each taffy
/// child came from. Spacers are simply absent from `children`, so the
/// collect pass never has to recognise one.
struct Built {
    id: NodeId,
    /// `(subtree, index into the source element's children)`, in taffy
    /// order.
    children: Vec<(Built, usize)>,
}

/// Resolve a root `Length` directly against the viewport.
fn against_viewport(len: Length, available: u16) -> Dimension {
    match len {
        Length::Px(n) => length(f32::from(n)),
        Length::Fill(_) => length(f32::from(available)),
        Length::Shrink => Dimension::AUTO,
    }
}

/// Which axis a *parent* lays its children out along. A child's own
/// `Length`s are interpreted relative to this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Axis {
    Horizontal,
    Vertical,
}

impl Axis {
    fn of(kind: &Kind) -> Self {
        match kind {
            // `El` is a one-child row, so a single child's `align_x`
            // goes through the same spacer machinery as any row's.
            Kind::Row | Kind::El | Kind::Text(_) | Kind::Paragraph(_) => Axis::Horizontal,
            Kind::Column => Axis::Vertical,
        }
    }
}

/// Lay `root` out inside `viewport`.
pub fn layout(root: &Element, viewport: Rect) -> Laid {
    let mut tree: TaffyTree<LeafText> = TaffyTree::new();
    let built = build(&mut tree, root, Axis::Vertical);
    // The root has no flex parent, so neither `flex-grow` nor
    // `align-self` can act on it: the *viewport* is its parent, and a
    // root `Fill` means "the whole viewport" on that axis.
    let mut root_style = taffy_style(root, Axis::Vertical);
    root_style.flex_grow = 0.0;
    root_style.align_self = None;
    root_style.size = TaffySize {
        width: against_viewport(root.style.width, viewport.width),
        height: against_viewport(root.style.height, viewport.height),
    };
    tree.set_style(built.id, root_style)
        .expect("taffy root style");

    let available = TaffySize {
        width: AvailableSpace::Definite(f32::from(viewport.width)),
        height: AvailableSpace::Definite(f32::from(viewport.height)),
    };
    tree.compute_layout_with_measure(built.id, available, measure)
        .expect("taffy layout");

    collect(&tree, &built, root, viewport.x, viewport.y)
}

/// Build the taffy subtree for `element`, whose parent lays out along
/// `parent_axis`.
fn build(tree: &mut TaffyTree<LeafText>, element: &Element, parent_axis: Axis) -> Built {
    let style = taffy_style(element, parent_axis);
    match &element.kind {
        Kind::Text(text) | Kind::Paragraph(text) => {
            let context = LeafText {
                text: text.clone(),
                wrapping: matches!(element.kind, Kind::Paragraph(_)),
            };
            Built {
                id: tree
                    .new_leaf_with_context(style, context)
                    .expect("taffy leaf"),
                children: Vec::new(),
            }
        }
        _ => {
            let axis = Axis::of(&element.kind);
            let (children, ids) = build_children(tree, element, axis);
            Built {
                id: tree.new_with_children(style, &ids).expect("taffy branch"),
                children,
            }
        }
    }
}

/// Build the children of `element`, inserting growing spacers between
/// main-axis alignment groups (translation 2 in the module docs).
///
/// Returns the real children paired with their source index, plus the
/// full id list *including* spacers for `new_with_children`.
fn build_children(
    tree: &mut TaffyTree<LeafText>,
    element: &Element,
    axis: Axis,
) -> (Vec<(Built, usize)>, Vec<NodeId>) {
    let groups = alignment_groups(element, axis);
    // Only pay for spacers when some child actually asked to be pushed.
    let aligned = !groups[1].is_empty() || !groups[2].is_empty();

    let mut children = Vec::new();
    let mut ids = Vec::new();
    for group in groups.iter().filter(|group| !group.is_empty()) {
        if aligned && !ids.is_empty() {
            ids.push(spacer(tree, axis));
        }
        for &index in group {
            let child = build(tree, &element.children[index], axis);
            ids.push(child.id);
            children.push((child, index));
        }
    }
    (children, ids)
}

/// Partition child indices into (start, centre, end) by their main-axis
/// alignment, preserving document order within each group.
fn alignment_groups(element: &Element, axis: Axis) -> [Vec<usize>; 3] {
    let mut groups: [Vec<usize>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for (index, child) in element.children.iter().enumerate() {
        let group = match axis {
            Axis::Horizontal => match child.style.align_x {
                Some(AlignX::Center) => 1,
                Some(AlignX::Right) => 2,
                _ => 0,
            },
            Axis::Vertical => match child.style.align_y {
                Some(AlignY::Center) => 1,
                Some(AlignY::Bottom) => 2,
                _ => 0,
            },
        };
        groups[group].push(index);
    }
    groups
}

/// A zero-basis growing node, used to push alignment groups apart.
fn spacer(tree: &mut TaffyTree<LeafText>, axis: Axis) -> NodeId {
    let mut style = Style {
        flex_grow: 1.0,
        flex_shrink: 1.0,
        flex_basis: length(0.0),
        ..Default::default()
    };
    match axis {
        Axis::Horizontal => style.size.height = length(0.0),
        Axis::Vertical => style.size.width = length(0.0),
    }
    tree.new_leaf(style).expect("taffy spacer")
}

/// Lower one element's style, given the axis its parent lays out along.
fn taffy_style(element: &Element, parent_axis: Axis) -> Style {
    let source = &element.style;
    let mut style = Style {
        display: Display::Flex,
        flex_direction: match Axis::of(&element.kind) {
            Axis::Horizontal => FlexDirection::Row,
            Axis::Vertical => FlexDirection::Column,
        },
        ..Default::default()
    };

    if source.wrap {
        style.flex_wrap = FlexWrap::Wrap;
    }

    // Padding and the one-cell border both inset content, and in a
    // terminal a border *is* a whole cell, so taffy sees them alike.
    let border = f32::from(u16::from(source.border));
    style.padding = taffy::Rect {
        left: length(f32::from(source.pad.left)),
        right: length(f32::from(source.pad.right)),
        top: length(f32::from(source.pad.top)),
        bottom: length(f32::from(source.pad.bottom)),
    };
    style.border = taffy::Rect {
        left: length(border),
        right: length(border),
        top: length(border),
        bottom: length(border),
    };
    style.gap = TaffySize {
        width: length(f32::from(source.spacing.0)),
        height: length(f32::from(source.spacing.1)),
    };

    // elm-ui's container defaults: a row centres its children on the
    // cross axis, a column packs them to the start.
    style.align_items = Some(match Axis::of(&element.kind) {
        Axis::Horizontal => AlignItems::CENTER,
        Axis::Vertical => AlignItems::START,
    });

    apply_lengths(&mut style, element, parent_axis);
    apply_bounds(&mut style, element);
    apply_cross_alignment(&mut style, element, parent_axis);
    style
}

/// Translation 1: `Length` means `flex-grow` on the main axis and a
/// size (or `stretch`) on the cross axis.
fn apply_lengths(style: &mut Style, element: &Element, parent_axis: Axis) {
    let (main, cross) = match parent_axis {
        Axis::Horizontal => (element.style.width, element.style.height),
        Axis::Vertical => (element.style.height, element.style.width),
    };

    let mut main_dimension = Dimension::AUTO;
    match main {
        Length::Px(n) => main_dimension = length(f32::from(n)),
        Length::Shrink => style.flex_grow = 0.0,
        Length::Fill(portion) => {
            style.flex_grow = f32::from(portion.max(1));
            style.flex_basis = length(0.0);
        }
    }
    // elm-ui has no "shrink below content" state, so a greedy sibling
    // must not squeeze a `Shrink` element out of its own text.
    style.flex_shrink = 0.0;

    let mut cross_dimension = Dimension::AUTO;
    match cross {
        Length::Px(n) => cross_dimension = length(f32::from(n)),
        Length::Shrink => {}
        // flex-grow does not act on the cross axis; stretch does.
        Length::Fill(_) => style.align_self = Some(AlignSelf::STRETCH),
    }

    match parent_axis {
        Axis::Horizontal => {
            style.size.width = main_dimension;
            style.size.height = cross_dimension;
        }
        Axis::Vertical => {
            style.size.height = main_dimension;
            style.size.width = cross_dimension;
        }
    }
}

fn apply_bounds(style: &mut Style, element: &Element) {
    let source = &element.style;
    if let Some(n) = source.min_width {
        style.min_size.width = length(f32::from(n));
    }
    if let Some(n) = source.max_width {
        style.max_size.width = length(f32::from(n));
    }
    if let Some(n) = source.min_height {
        style.min_size.height = length(f32::from(n));
    }
    if let Some(n) = source.max_height {
        style.max_size.height = length(f32::from(n));
    }
}

/// Cross-axis alignment *is* `align-self`, exactly. (Main-axis
/// alignment is not — see [`build_children`].)
fn apply_cross_alignment(style: &mut Style, element: &Element, parent_axis: Axis) {
    let align = match parent_axis {
        Axis::Horizontal => element.style.align_y.map(|align| match align {
            AlignY::Top => AlignSelf::START,
            AlignY::Center => AlignSelf::CENTER,
            AlignY::Bottom => AlignSelf::END,
        }),
        Axis::Vertical => element.style.align_x.map(|align| match align {
            AlignX::Left => AlignSelf::START,
            AlignX::Center => AlignSelf::CENTER,
            AlignX::Right => AlignSelf::END,
        }),
    };
    if let Some(align) = align {
        style.align_self = Some(align);
    }
}

/// The leaf measure function — the seam `taffy` leaves for us, and the
/// reason terminal text measurement ([`crate::measure`]) is
/// load-bearing rather than incidental.
fn measure(
    inputs: LayoutInput,
    _node: NodeId,
    context: Option<&mut LeafText>,
    style: &Style,
) -> LayoutOutput {
    taffy::compute_leaf_layout(
        inputs,
        style,
        |_, _| 0.0,
        |known, available| {
            let Some(leaf) = context else {
                return TaffySize::ZERO;
            };
            if !leaf.wrapping {
                return TaffySize {
                    width: f32::from(text_width(&leaf.text)),
                    height: 1.0,
                };
            }
            // Wrapping makes height a function of width, which is exactly
            // what `known_dimensions` and `available_space` are for.
            let width = known.width.unwrap_or(match available.width {
                AvailableSpace::Definite(width) => width,
                AvailableSpace::MaxContent => f32::from(text_width(&leaf.text)),
                AvailableSpace::MinContent => leaf
                    .text
                    .split_whitespace()
                    .map(|word| f32::from(text_width(word)))
                    .fold(0.0, f32::max),
            });
            let lines = wrap(&leaf.text, width.max(0.0) as u16);
            let widest = lines
                .iter()
                .map(|line| f32::from(text_width(line)))
                .fold(0.0, f32::max);
            TaffySize {
                width: widest,
                height: lines.len() as f32,
            }
        },
    )
}

/// Walk the taffy result back onto the source tree, converting
/// parent-relative `f32` positions into absolute whole cells and
/// restoring document order.
fn collect(
    tree: &TaffyTree<LeafText>,
    built: &Built,
    element: &Element,
    parent_x: u16,
    parent_y: u16,
) -> Laid {
    let layout = tree.layout(built.id).expect("taffy layout result");
    let x = parent_x.saturating_add(layout.location.x.max(0.0).round() as u16);
    let y = parent_y.saturating_add(layout.location.y.max(0.0).round() as u16);
    let rect = Rect::new(
        x,
        y,
        layout.size.width.max(0.0).round() as u16,
        layout.size.height.max(0.0).round() as u16,
    );

    // Re-break a paragraph at the width layout actually gave it, so the
    // painter can never disagree with the height that was reserved.
    let lines = match &element.kind {
        Kind::Paragraph(text) => wrap(text, rect.width),
        Kind::Text(text) => vec![text.clone()],
        _ => Vec::new(),
    };

    // Layout order is the alignment-group order; the caller wants
    // document order, which is what the recorded source indices are for.
    let mut children: Vec<(usize, Laid)> = built
        .children
        .iter()
        .map(|(child, index)| {
            (
                *index,
                collect(tree, child, &element.children[*index], x, y),
            )
        })
        .collect();
    children.sort_by_key(|(index, _)| *index);

    Laid {
        rect,
        kind: element.kind.clone(),
        style: element.style.clone(),
        attrs: element.attrs.clone(),
        lines,
        children: children.into_iter().map(|(_, laid)| laid).collect(),
    }
}
