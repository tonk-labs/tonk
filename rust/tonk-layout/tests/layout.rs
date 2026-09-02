//! Geometry tests: an attribute tree in, integer cell rectangles out.
//!
//! No terminal, no tonk, no template pipeline — which is the point of
//! `tonk-layout` being its own crate. Each test names the elm-ui
//! behaviour it is pinning, so a future engine swap has something to be
//! judged against.

use tonk_layout::{AlignX, AlignY, Element, Kind, Laid, Length, Rect, Style, layout};

/// Lay out in an 80x24 viewport unless a test says otherwise.
fn lay(root: &Element) -> Laid {
    layout(root, Rect::new(0, 0, 80, 24))
}

fn sized(root: &Element, width: u16, height: u16) -> Laid {
    layout(root, Rect::new(0, 0, width, height))
}

fn text(content: &str) -> Element {
    Element::text(content)
}

fn filled(root: Element) -> Element {
    root.width(Length::Fill(1)).height(Length::Fill(1))
}

/// `[x, width]` of each child, the shape most of these assertions want.
fn spans(laid: &Laid) -> Vec<(u16, u16)> {
    laid.children
        .iter()
        .map(|child| (child.rect.x, child.rect.width))
        .collect()
}

#[test]
fn shrink_sizes_a_row_to_its_content() {
    // elm-ui: `el`, `row` and `column` all default to `shrink`.
    let tree = Element::row(vec![text("ab"), text("cde")]);
    let laid = lay(&tree);
    assert_eq!(laid.rect.width, 5);
    assert_eq!(spans(&laid), vec![(0, 2), (2, 3)]);
}

#[test]
fn spacing_sits_between_children_and_not_at_the_edges() {
    // elm-ui has no margin: `spacing` is the only inter-child gap, and
    // it must not add an outer inset.
    let tree = Element::row(vec![text("ab"), text("cd"), text("ef")]).style(Style {
        spacing: (2, 0),
        ..Default::default()
    });
    let laid = lay(&tree);
    assert_eq!(spans(&laid), vec![(0, 2), (4, 2), (8, 2)]);
    assert_eq!(laid.rect.width, 10);
}

#[test]
fn padding_insets_content_without_moving_the_parent() {
    let tree = Element::row(vec![text("ab")]).style(Style {
        pad: tonk_layout::Edges::xy(3, 1),
        ..Default::default()
    });
    let laid = lay(&tree);
    assert_eq!(laid.rect, Rect::new(0, 0, 8, 3));
    assert_eq!(laid.children[0].rect, Rect::new(3, 1, 2, 1));
}

#[test]
fn a_border_costs_a_whole_cell_on_each_edge() {
    // Terminals have no sub-cell strokes, so a border is padding that
    // happens to be drawn.
    let tree = Element::row(vec![text("ab")]).style(Style {
        border: true,
        ..Default::default()
    });
    let laid = lay(&tree);
    assert_eq!(laid.rect, Rect::new(0, 0, 4, 3));
    assert_eq!(laid.children[0].rect, Rect::new(1, 1, 2, 1));
}

#[test]
fn fill_splits_the_available_space_evenly() {
    // elm-ui: "The available space will be split evenly between
    // elements that have `width fill`."
    let tree = filled(Element::row(vec![
        text("a").width(Length::Fill(1)),
        text("b").width(Length::Fill(1)),
    ]));
    let laid = sized(&tree, 20, 1);
    assert_eq!(spans(&laid), vec![(0, 10), (10, 10)]);
}

#[test]
fn fill_portions_are_weighted() {
    let tree = filled(Element::row(vec![
        text("a").width(Length::Fill(1)),
        text("b").width(Length::Fill(3)),
    ]));
    let laid = sized(&tree, 20, 1);
    assert_eq!(spans(&laid), vec![(0, 5), (5, 15)]);
}

#[test]
fn fill_shares_only_what_fixed_siblings_leave() {
    let tree = filled(Element::row(vec![
        text("fixed").width(Length::Px(6)),
        text("rest").width(Length::Fill(1)),
    ]));
    let laid = sized(&tree, 20, 1);
    assert_eq!(spans(&laid), vec![(0, 6), (6, 14)]);
}

#[test]
fn an_uneven_fill_split_tiles_without_gaps_or_overlaps() {
    // 80 does not divide by 3. Whatever the rounding, the segments must
    // still tile the row exactly — this is the property that makes
    // `taffy`'s cumulative rounding pass worth keeping on.
    let tree = filled(Element::row(vec![
        text("a").width(Length::Fill(1)),
        text("b").width(Length::Fill(1)),
        text("c").width(Length::Fill(1)),
    ]));
    let laid = sized(&tree, 80, 1);
    let spans = spans(&laid);
    assert_eq!(spans.iter().map(|(_, w)| w).sum::<u16>(), 80);
    for window in spans.windows(2) {
        assert_eq!(window[0].0 + window[0].1, window[1].0, "no gap or overlap");
    }
}

#[test]
fn layout_is_stable_across_repeated_solves() {
    // An immediate-mode renderer re-solves every frame. If an uneven
    // split resolved differently between frames the UI would shimmer.
    let tree = filled(Element::row(vec![
        text("a").width(Length::Fill(1)),
        text("b").width(Length::Fill(1)),
        text("c").width(Length::Fill(1)),
    ]));
    let first = spans(&sized(&tree, 80, 1));
    for _ in 0..16 {
        assert_eq!(spans(&sized(&tree, 80, 1)), first);
    }
}

#[test]
fn fill_on_the_cross_axis_stretches_instead_of_growing() {
    // flex-grow does nothing across the axis, so `Fill` there has to
    // lower to `align-self: stretch`. Without that translation a
    // `height=fill` child of a row silently collapses to one row.
    let tree = filled(Element::row(vec![
        text("tall").height(Length::Fill(1)),
        text("short"),
    ]));
    let laid = sized(&tree, 20, 5);
    assert_eq!(laid.children[0].rect.height, 5);
    assert_eq!(laid.children[1].rect.height, 1);
}

#[test]
fn a_row_centres_its_children_vertically_by_default() {
    // elm-ui's `row` default is `contentCenterY`.
    let tree = filled(Element::row(vec![text("mid")]));
    let laid = sized(&tree, 20, 5);
    assert_eq!(laid.children[0].rect.y, 2);
}

#[test]
fn a_column_packs_its_children_to_the_top_left() {
    // elm-ui's `column` default is `contentTop ++ contentLeft`.
    let tree = filled(Element::column(vec![text("one"), text("two")]));
    let laid = sized(&tree, 20, 5);
    assert_eq!(laid.children[0].rect, Rect::new(0, 0, 3, 1));
    assert_eq!(laid.children[1].rect, Rect::new(0, 1, 3, 1));
}

#[test]
fn cross_axis_alignment_is_declared_on_the_child() {
    let tree = filled(Element::row(vec![
        text("t").height(Length::Shrink).style(Style {
            align_y: Some(AlignY::Top),
            ..Default::default()
        }),
        text("b").style(Style {
            align_y: Some(AlignY::Bottom),
            ..Default::default()
        }),
    ]));
    let laid = sized(&tree, 20, 5);
    assert_eq!(laid.children[0].rect.y, 0);
    assert_eq!(laid.children[1].rect.y, 4);
}

#[test]
fn an_aligned_child_pushes_its_siblings() {
    // The elm-ui doc's own example:
    //
    //     row [] [ el [] none, el [alignLeft] none
    //            , el [centerX] none, el [alignRight] none ]
    //
    // renders `|-|-|    |-|    |-|`: the unaligned and left children
    // pack left, the centred one sits in the middle of what is left,
    // and the right one goes to the edge. This is not `align-self` and
    // no single `justify-content` value produces it — it is why
    // `solve.rs` inserts spacers between alignment groups.
    let cell = |align: Option<AlignX>| {
        text("--").style(Style {
            align_x: align,
            ..Default::default()
        })
    };
    let tree = filled(Element::row(vec![
        cell(None),
        cell(Some(AlignX::Left)),
        cell(Some(AlignX::Center)),
        cell(Some(AlignX::Right)),
    ]));
    let laid = sized(&tree, 20, 1);

    // Document order is preserved in the output regardless of the
    // layout-order regrouping.
    let spans = spans(&laid);
    assert_eq!(spans[0], (0, 2), "unaligned packs left");
    assert_eq!(spans[1], (2, 2), "alignLeft packs left, after it");
    assert_eq!(spans[3], (18, 2), "alignRight reaches the right edge");
    // Centred within the run the left and right groups leave free —
    // columns 4..18, so a 2-cell child starts at 4 + (14 - 2) / 2.
    assert_eq!(spans[2], (10, 2), "centerX centres in the remaining space");
}

#[test]
fn a_paragraph_wraps_and_reports_the_height_it_needs() {
    let tree = Element::paragraph("one two three four five").width(Length::Px(9));
    let laid = sized(&tree, 20, 10);
    assert_eq!(laid.rect.width, 9);
    assert_eq!(laid.lines, vec!["one two", "three", "four five"]);
    assert_eq!(laid.rect.height, 3, "height follows from the wrap");
}

#[test]
fn a_paragraph_rewraps_when_the_width_changes() {
    let tree = filled(Element::paragraph("one two three four five"));
    assert_eq!(
        sized(&tree, 20, 10).lines,
        vec!["one two three four", "five"]
    );
    assert_eq!(
        sized(&tree, 12, 10).lines,
        vec!["one two", "three four", "five"]
    );
    assert_eq!(sized(&tree, 8, 10).lines.len(), 4);
}

#[test]
fn wide_characters_are_measured_in_cells_not_characters() {
    // Three CJK characters are three `char`s but six cells. A row that
    // measured `chars().count()` would size this at 3 and clip.
    let tree = Element::row(vec![text("日本語")]);
    assert_eq!(lay(&tree).rect.width, 6);
}

#[test]
fn min_and_max_clamp_a_fill() {
    let tree = filled(Element::row(vec![
        text("a").width(Length::Fill(1)).style(Style {
            width: Length::Fill(1),
            max_width: Some(4),
            ..Default::default()
        }),
        text("b").width(Length::Fill(1)),
    ]));
    let laid = sized(&tree, 20, 1);
    assert_eq!(laid.children[0].rect.width, 4);
}

#[test]
fn an_unknown_kind_lays_out_as_a_plain_container() {
    // The vocabulary degrades: a tag the painter does not know still
    // has geometry, so an unrecognised element is invisible rather than
    // fatal.
    let mut unknown = Element::new(Kind::El);
    unknown.children = vec![text("inside")];
    let laid = lay(&unknown);
    assert_eq!(laid.rect.width, 6);
    assert_eq!(laid.children[0].rect.width, 6);
}
