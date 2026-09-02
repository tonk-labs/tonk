//! Pure geometry logic for the FAB element.
//!
//! No DOM imports — compiles and tests on the native target.

use serde_json::{Value, json};

/// Build the repository endpoint with the repository DID as one path
/// segment: the bar's presence probe, answered 404 for a space this device
/// does not hold.
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) fn repository_endpoint(space: &str) -> Result<String, &'static str> {
    let space = space.trim();
    if space.is_empty() || space.contains('{') || space.contains('}') {
        return Err("repository binding is unresolved");
    }
    Ok(format!("/api/repository/{}", urlencoding::encode(space)))
}

// The bar addresses exactly one repository endpoint, above. It mints its open
// link through the share control's transient command, and it reaches no
// invitation endpoint at all: listing, minting to a named root, and revoking
// are infrastructure the worker and CLI own. The one revocation surface a user
// gets is the account page's device list.

#[cfg(test)]
mod repository_endpoint_tests {
    use super::repository_endpoint;

    #[test]
    fn it_encodes_a_repository_did_as_one_path_segment() {
        assert_eq!(
            repository_endpoint("did:key:z6Mk/a").unwrap(),
            "/api/repository/did%3Akey%3Az6Mk%2Fa"
        );
    }

    #[test]
    fn it_rejects_empty_and_unresolved_repository_bindings() {
        for value in ["", "  ", "{id}", "did:key:{id}"] {
            assert!(repository_endpoint(value).is_err(), "{value:?}");
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FabBox {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FabState {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub dragging: bool,
}

pub enum FabIntent {
    DragStart,
    DragMove {
        x: f64,
        y: f64,
        state: FabState,
    },
    Resize {
        w: f64,
        h: f64,
        state: FabState,
    },
    Drop {
        x: f64,
        y: f64,
        state: FabState,
    },
    /// Expand the iframe to the full viewport so a modal dialog rendered
    /// inside the guest is not clipped to the small FAB box. Unlike
    /// `DragStart` it carries no drag semantics: the host leaves the resting
    /// position and size untouched, so a later `Resize` restores the FAB to
    /// exactly where it was when the modal closes.
    Overlay,
}

pub fn geometry_box(intent: &FabIntent, vw: f64, vh: f64) -> FabBox {
    match intent {
        FabIntent::DragStart => FabBox {
            left: 0.0,
            top: 0.0,
            width: vw,
            height: vh,
        },
        // During a drag the iframe stays pinned full-viewport (like DragStart);
        // the FAB element is translated *inside* the iframe to follow the
        // pointer. Moving the iframe itself per-frame would shift the pointer
        // coordinate frame under itself. `x`/`y` are unused here — the guest
        // applies them to the inner element, and Drop uses them for the final
        // shrunk box.
        FabIntent::DragMove {
            x: _,
            y: _,
            state: _,
        } => FabBox {
            left: 0.0,
            top: 0.0,
            width: vw,
            height: vh,
        },
        FabIntent::Resize { w, h, state } => FabBox {
            left: state.x,
            top: state.y,
            width: *w,
            height: *h,
        },
        FabIntent::Drop { x, y, state } => FabBox {
            left: *x,
            top: *y,
            width: state.w,
            height: state.h,
        },
        // Full-viewport, like DragStart, but with no drag semantics — used to
        // back a modal dialog so it renders unclipped. The resting box is
        // recovered from `FabState` by the following `Resize`, not from here.
        FabIntent::Overlay => FabBox {
            left: 0.0,
            top: 0.0,
            width: vw,
            height: vh,
        },
    }
}

/// The four fallback seats the FAB can restore on a subsequent page load.
/// A drop persists the nearest one: the vertical half of the viewport picks
/// top vs bottom, the horizontal half picks left vs right.
///
/// The resting position is expressed as two CSS classes on `<tonk-fab>` — a vertical
/// one (`fab-dock-top` / `fab-dock-bottom`) and a horizontal one
/// (`fab-dock-left` / `fab-dock-right`) — and the actual pixel placement + the
/// submenu open-direction live in the view's stylesheet (profile.yaml). This
/// enum owns only the persisted fallback seat; the live page keeps the exact
/// point selected by [`snap_to_nearest_edge`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dock {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// The viewport edge a released FAB settles against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

impl Edge {
    /// The public event value used by the reference FABB.
    pub fn symbol(self) -> &'static str {
        match self {
            Edge::Left => "left",
            Edge::Right => "right",
            Edge::Top => "top",
            Edge::Bottom => "bottom",
        }
    }
}

/// The safe resting inset on each viewport edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeInsets {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

/// The resting top-left position selected for a released FAB.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeSnap {
    pub edge: Edge,
    pub left: f64,
    pub top: f64,
}

/// Settle a released FAB against its nearest viewport edge while preserving
/// the free coordinate along that edge.
pub fn snap_to_nearest_edge(
    left: f64,
    top: f64,
    width: f64,
    height: f64,
    vw: f64,
    vh: f64,
    insets: EdgeInsets,
) -> EdgeSnap {
    let mut x = left.min(vw - width - insets.right).max(insets.left);
    let mut y = top.min(vh - height - insets.bottom).max(insets.top);
    let distances = [
        (Edge::Left, x),
        (Edge::Right, vw - (x + width)),
        (Edge::Top, y),
        (Edge::Bottom, vh - (y + height)),
    ];
    let edge = distances
        .into_iter()
        .reduce(|nearest, candidate| {
            if nearest.1 <= candidate.1 {
                nearest
            } else {
                candidate
            }
        })
        .map_or(Edge::Left, |(edge, _)| edge);

    match edge {
        Edge::Left => x = insets.left,
        Edge::Right => x = vw - width - insets.right,
        Edge::Top => y = insets.top,
        Edge::Bottom => y = vh - height - insets.bottom,
    }

    EdgeSnap {
        edge,
        left: x.max(insets.left),
        top: y.max(insets.top),
    }
}

/// Every dock axis class the view stylesheet defines, for clearing the element
/// before a fresh dock's classes are applied.
pub const DOCK_CLASSES: [&str; 4] = [
    "fab-dock-top",
    "fab-dock-bottom",
    "fab-dock-left",
    "fab-dock-right",
];

impl Dock {
    /// The two CSS classes the view stylesheet keys position + menu direction
    /// off: `[vertical, horizontal]`.
    pub fn css_classes(self) -> [&'static str; 2] {
        let vertical = match self {
            Dock::TopLeft | Dock::TopRight => "fab-dock-top",
            Dock::BottomLeft | Dock::BottomRight => "fab-dock-bottom",
        };
        let horizontal = match self {
            Dock::TopLeft | Dock::BottomLeft => "fab-dock-left",
            Dock::TopRight | Dock::BottomRight => "fab-dock-right",
        };
        [vertical, horizontal]
    }

    /// The entity symbol persisted for this dock (the `tonk:fab/dock` value).
    pub fn symbol(self) -> &'static str {
        match self {
            Dock::TopLeft => "tonk:top-left",
            Dock::TopRight => "tonk:top-right",
            Dock::BottomLeft => "tonk:bottom-left",
            Dock::BottomRight => "tonk:bottom-right",
        }
    }

    /// Parse a persisted dock symbol back to a `Dock`. Unknown / absent → `None`
    /// (the caller falls back to the default dock).
    pub fn from_symbol(s: &str) -> Option<Dock> {
        match s {
            "tonk:top-left" => Some(Dock::TopLeft),
            "tonk:top-right" => Some(Dock::TopRight),
            "tonk:bottom-left" => Some(Dock::BottomLeft),
            "tonk:bottom-right" => Some(Dock::BottomRight),
            _ => None,
        }
    }
}

/// Resolve the persisted dock from a `/query` result (a `Conclusion[]` JSON
/// value, the shape `window.tonk.query` yields).
///
/// A conclusion row is `{ this, fields: { dock, … } }`, so the projected
/// `dock` symbol lives under `fields` — reading it off the row directly
/// (an easy mistake that strands the dock at its default) finds nothing.
/// Falls back to a flat `row.dock` so a change in the projection shape
/// degrades to the default rather than a silent mismatch. `None` when the
/// result is empty, malformed, or names an unknown dock symbol.
pub fn dock_from_conclusions(rows: &Value) -> Option<Dock> {
    let first = rows.as_array()?.first()?;
    let symbol = first
        .get("fields")
        .and_then(|fields| fields.get("dock"))
        .or_else(|| first.get("dock"))
        .and_then(Value::as_str)?;
    Dock::from_symbol(symbol)
}

/// Resolve the persisted collapse from a `/query` result — the same row
/// shapes [`dock_from_conclusions`] reads. `None` when the result is
/// empty or malformed, which the caller treats as expanded.
pub fn collapsed_from_conclusions(rows: &Value) -> Option<bool> {
    let first = rows.as_array()?.first()?;
    first
        .get("fields")
        .and_then(|fields| fields.get("collapsed"))
        .or_else(|| first.get("collapsed"))
        .and_then(Value::as_bool)
}

/// Pick the fallback corner nearest a drop. The vertical half of the viewport (height
/// `vh`) picks top vs bottom and the horizontal half (width `vw`) picks left vs
/// right, keyed off the drag's anchor point `(center_x, center_y)` — the grab
/// handle's center, the same anchor `mirrored` reads. The exact midlines fall
/// to the bottom / right dock.
pub fn nearest_dock(center_x: f64, center_y: f64, vw: f64, vh: f64) -> Dock {
    match (center_x < vw / 2.0, center_y < vh / 2.0) {
        (true, true) => Dock::TopLeft,
        (false, true) => Dock::TopRight,
        (true, false) => Dock::BottomLeft,
        (false, false) => Dock::BottomRight,
    }
}

/// Whether the bar shows MIRRORED (right-anchored) at this horizontal center.
/// The exact midline mirrors, matching `nearest_dock`'s midline-to-right
/// choice, so the live drag preview always agrees with the eventual snap.
pub fn mirrored(center_x: f64, vw: f64) -> bool {
    center_x >= vw / 2.0
}

pub const FULL_BAR_WIDTH_PX: f64 = 396.0;
pub const COMPACT_CELL_PX: f64 = 44.0;
pub const COMPACT_SPACE_MIN_PX: f64 = 120.0;
pub const SPACE_CELL_PX: f64 = 216.0;
pub const SHARE_CELL_PX: f64 = 144.0;

/// The action partition for one resolved usable width. The caller subtracts
/// its safe-area-aware left and right float insets before asking for a layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarLayout {
    pub compact: bool,
    pub space_width_px: f64,
    pub show_share: bool,
    pub show_overflow: bool,
}

/// Partition the canonical action run according to what fully fits.
///
/// Exact fits stay in the wider layout. The overflow bookend appears only
/// when it holds something: with the mode row gone the overflow's one
/// resident is the share route, so a compact bar that still shows share
/// would open an EMPTY stack from it — and a dead cell is worse chrome than
/// an honest one. The space cell consumes the remaining room and may shrink
/// to zero when even the bookends do not fit.
pub fn bar_layout(usable_width_px: f64) -> BarLayout {
    let usable = usable_width_px.max(0.0);
    if usable >= FULL_BAR_WIDTH_PX {
        return BarLayout {
            compact: false,
            space_width_px: SPACE_CELL_PX,
            show_share: true,
            show_overflow: false,
        };
    }

    let show_share = usable >= COMPACT_CELL_PX + COMPACT_SPACE_MIN_PX + SHARE_CELL_PX;
    let reserved = COMPACT_CELL_PX
        + if show_share {
            SHARE_CELL_PX
        } else {
            COMPACT_CELL_PX
        };
    BarLayout {
        compact: true,
        space_width_px: (usable - reserved).clamp(0.0, SPACE_CELL_PX),
        show_share,
        show_overflow: !show_share,
    }
}

/// Lift a resting bottom edge above the visual viewport, leaving `gap_px`
/// between the bar and any software keyboard occluding the layout viewport.
pub fn keyboard_lift_px(
    resting_bottom: f64,
    visual_offset_top: f64,
    visual_height: f64,
    gap_px: f64,
) -> f64 {
    let occlusion = resting_bottom - (visual_offset_top + visual_height);
    if occlusion > 0.0 {
        occlusion + gap_px.max(0.0)
    } else {
        0.0
    }
}

/// Clamp a dragged bar's top-left corner so the bar stays fully inside the
/// viewport. The origin clamp runs LAST: a bar wider or taller than the
/// viewport pins to the left/top edge, keeping the grab handle reachable.
pub fn clamp_position(
    left: f64,
    top: f64,
    width: f64,
    height: f64,
    vw: f64,
    vh: f64,
) -> (f64, f64) {
    (left.min(vw - width).max(0.0), top.min(vh - height).max(0.0))
}

/// Build a `TransactRequest` JSON body for `window.tonk.transact(...)`.
///
/// Asserts the `tonk:fab/dock` concept on `state:fab` with the given dock as an
/// entity symbol. The JSON shape matches the `TransactRequest` serde derive in
/// `tonk-core/src/claim.rs`:
///
/// - `Claim` → `#[serde(tag="op", content="application")]` → `"op":"assert"`, `"application":{...}`
/// - `ConceptDescriptor` → `#[serde(tag="kind", content="concept")]` → `"kind":"transient"`, `"concept":{...}`
/// - `PredicateApplication` → `{ predicate, parameters }`
pub fn dock_claim_json(dock: Dock) -> Value {
    json!({
        "claims": [{
            "op": "assert",
            "application": {
                "predicate": {
                    "kind": "durable",
                    "concept": {
                        "description": "Persisted FAB dock (profile claim).",
                        "with": {
                            "dock": {
                                "the": "xyz.tonk.fab/dock",
                                "cardinality": "one",
                                "as": "Entity"
                            }
                        }
                    }
                },
                "parameters": {
                    "this": "state:fab",
                    "dock": dock.symbol()
                }
            }
        }]
    })
}

/// Build a `TransactRequest` JSON body persisting whether the bar is
/// collapsed, on the same `state:fab` entity the dock claim rides.
///
/// A concept of its own rather than a second field on the dock concept:
/// both are cardinality-one, and a two-field descriptor stops matching
/// whichever claim was written alone, stranding the older state.
pub fn collapsed_claim_json(collapsed: bool) -> Value {
    json!({
        "claims": [{
            "op": "assert",
            "application": {
                "predicate": {
                    "kind": "durable",
                    "concept": {
                        "description": "Persisted FAB collapse (profile claim).",
                        "with": {
                            "collapsed": {
                                "the": "xyz.tonk.fab/collapsed",
                                "cardinality": "one",
                                "as": "Boolean"
                            }
                        }
                    }
                },
                "parameters": {
                    "this": "state:fab",
                    "collapsed": collapsed
                }
            }
        }]
    })
}

/// Build a `TransactRequest` JSON body for the `member/promote` command.
///
/// Asserted once the page has minted the admin hop under the passkey:
/// `member` is the DID the membership is keyed on, `space` the space, and
/// `chain` the base58 hop `promoter-account -> member`. Routeless like
/// `pause_claim_json`: the worker reads `space` off the command.
pub fn promote_claim_json(space: &str, member: &str, chain: &str) -> Value {
    json!({
        "claims": [{
            "op": "assert",
            "application": {
                "predicate": {
                    "kind": "transient",
                    "concept": {
                        "description": "Promote a member of a space to admin.",
                        "with": {
                            "member": { "the": "xyz.tonk.promote/member", "cardinality": "one", "as": "Entity" },
                            "space": { "the": "xyz.tonk.promote/space", "cardinality": "one", "as": "Entity" },
                            "chain": { "the": "xyz.tonk.promote/chain", "cardinality": "one", "as": "Text" }
                        }
                    }
                },
                "parameters": {
                    "member": member,
                    "space": space,
                    "chain": chain
                }
            }
        }]
    })
}

/// Build a `TransactRequest` JSON body for the `tonk:pause-sync` command.
///
/// A transient command asserting the target `space` (the DID to pause) with a
/// per-click `time` so each dispatch is a distinct transient, plus the `marker`
/// (the command URI) that keeps the shape distinct from `tonk:invite`. `this`
/// is omitted so the worker mints it from `(descriptor, parameters)`. Dispatched
/// routeless via `window.tonk.transact`, so it lands on the FAB portal's own
/// `main@profile:tonk` context where the command lives; the worker's handler
/// reads `space` to flip that replica — nothing space-side is required.
pub fn pause_claim_json(command: &str, space: &str, time: f64) -> Value {
    json!({
        "claims": [{
            "op": "assert",
            "application": {
                "predicate": {
                    "kind": "transient",
                    "concept": {
                        "description": "Toggle auto-sync (pause ⇄ resume) for a space.",
                        "with": {
                            "time":   { "the": "dom.event/time-stamp", "as": "Float" },
                            "space":  { "the": "xyz.tonk.pause-sync/space", "as": "Entity" },
                            "marker": { "the": "dom.event.current-target.dataset/pause-sync", "as": "Entity" }
                        }
                    }
                },
                "parameters": {
                    "time": time,
                    "space": space,
                    "marker": command
                }
            }
        }]
    })
}

/// The inline `min-width` (px) to stamp on a bar segment when its dropdown
/// opens: the menu's natural (max-content) width when that EXCEEDS the
/// segment, so the rung widens — whitespace filling around its label — and
/// the menu (styled `width: 100%`) lands exactly as wide as the rung. `None`
/// when the segment is already at least as wide (the menu's `width: 100%`
/// alone matches them). Only ever widens; a menu narrower than its segment
/// never shrinks the bar.
pub fn menu_min_width(menu_natural: f64, segment: f64) -> Option<f64> {
    (menu_natural > segment).then(|| menu_natural.ceil())
}

/// The ratcheted `min-width` for a segment whose menu just opened: the
/// equalized target when the menu outmeasures the segment's current rendered
/// width, never below an already-stamped value (mid-transition the rect
/// under-reports, and a shrunken menu must not narrow the column). `None`
/// means leave any existing stamp untouched.
pub fn ratchet_min_width(menu_natural: f64, segment: f64, stamped: Option<f64>) -> Option<f64> {
    let target = menu_min_width(menu_natural, segment)?;
    Some(stamped.map_or(target, |prior| target.max(prior)))
}

/// The AUTHORITATIVE `min-width` for the one fonts-ready restamp: the menu's
/// fresh real-metrics width, ceiled, replacing any ratcheted stamp in BOTH
/// directions — measurements taken before the font landed used the fallback
/// face (typically wider than condensed Plex), and the never-shrink ratchet
/// cannot correct an over-wide stamp downward. `None` (an unrendered, empty
/// menu) leaves the existing stamp untouched.
pub fn corrected_min_width(menu_natural: f64) -> Option<f64> {
    (menu_natural > 0.0).then(|| menu_natural.ceil())
}

#[cfg(test)]
mod mirror {
    use super::*;

    #[test]
    fn a_center_left_of_the_midline_is_not_mirrored() {
        assert!(!mirrored(499.9, 1000.0));
    }

    #[test]
    fn a_center_right_of_the_midline_is_mirrored() {
        assert!(mirrored(500.1, 1000.0));
    }

    #[test]
    fn the_midline_mirrors_like_nearest_dock_falls_right() {
        // Consistent with `nearest_dock`, whose exact midline docks right.
        assert!(mirrored(500.0, 1000.0));
    }
}

#[cfg(test)]
mod compact {
    use super::*;

    #[test]
    fn the_fit_policy_partitions_every_boundary_width() {
        for (width, expected) in [
            (
                396.0,
                BarLayout {
                    compact: false,
                    space_width_px: 216.0,
                    show_share: true,
                    show_overflow: false,
                },
            ),
            (
                395.9,
                BarLayout {
                    compact: true,
                    space_width_px: 207.9,
                    show_share: true,
                    show_overflow: false,
                },
            ),
            (
                308.0,
                BarLayout {
                    compact: true,
                    space_width_px: 120.0,
                    show_share: true,
                    show_overflow: false,
                },
            ),
            (
                307.9,
                BarLayout {
                    compact: true,
                    space_width_px: 216.0,
                    show_share: false,
                    show_overflow: true,
                },
            ),
            (
                216.0,
                BarLayout {
                    compact: true,
                    space_width_px: 128.0,
                    show_share: false,
                    show_overflow: true,
                },
            ),
            (
                80.0,
                BarLayout {
                    compact: true,
                    space_width_px: 0.0,
                    show_share: false,
                    show_overflow: true,
                },
            ),
        ] {
            let actual = bar_layout(width);
            assert_eq!(actual.compact, expected.compact, "usable width {width}");
            assert_eq!(
                actual.show_share, expected.show_share,
                "usable width {width}"
            );
            assert_eq!(
                actual.show_overflow, expected.show_overflow,
                "usable width {width}"
            );
            assert!(
                (actual.space_width_px - expected.space_width_px).abs() < 0.001,
                "usable width {width}: expected space {}, got {}",
                expected.space_width_px,
                actual.space_width_px,
            );
        }
    }
}

#[cfg(test)]
mod keyboard_lift {
    use super::*;

    #[test]
    fn a_resting_bar_inside_the_visual_viewport_is_not_lifted() {
        assert_eq!(keyboard_lift_px(700.0, 0.0, 720.0, 8.0), 0.0);
    }

    #[test]
    fn an_occluded_resting_bar_clears_the_keyboard_by_the_gap() {
        assert_eq!(keyboard_lift_px(760.0, 0.0, 600.0, 8.0), 168.0);
    }

    #[test]
    fn repeated_measurements_use_the_same_resting_bottom() {
        let first = keyboard_lift_px(760.0, 20.0, 600.0, 8.0);
        let second = keyboard_lift_px(760.0, 20.0, 600.0, 8.0);
        assert_eq!(first, 148.0);
        assert_eq!(second, first);
    }
}

#[cfg(test)]
mod clamp {
    use super::*;

    #[test]
    fn an_inside_position_is_untouched() {
        assert_eq!(
            clamp_position(100.0, 50.0, 300.0, 36.0, 1000.0, 800.0),
            (100.0, 50.0)
        );
    }

    #[test]
    fn it_clamps_every_edge() {
        // Past the origin pins to 0.
        assert_eq!(
            clamp_position(-20.0, -5.0, 300.0, 36.0, 1000.0, 800.0),
            (0.0, 0.0)
        );
        // Right/bottom overflow pins to viewport minus the bar.
        assert_eq!(
            clamp_position(900.0, 790.0, 300.0, 36.0, 1000.0, 800.0),
            (700.0, 764.0)
        );
    }

    #[test]
    fn a_bar_wider_than_the_viewport_pins_to_the_origin() {
        // vw - width is negative; the origin wins (max runs last) so the
        // bar's left edge — and the circle cap on it — stays reachable.
        assert_eq!(
            clamp_position(50.0, 10.0, 500.0, 36.0, 400.0, 800.0),
            (0.0, 10.0)
        );
    }
}

#[cfg(test)]
mod corrected {
    use super::*;

    #[test]
    fn a_rendered_menu_restamps_to_its_ceiled_width() {
        assert_eq!(corrected_min_width(220.4), Some(221.0));
    }

    #[test]
    fn an_empty_menu_clears_nothing() {
        // Zero means the menu has no rendered rows yet — leave the current
        // stamp alone rather than collapsing the column.
        assert_eq!(corrected_min_width(0.0), None);
    }
}

#[cfg(test)]
mod dock {
    use super::*;

    // 1000x800 viewport: midlines at x=500, y=400.
    #[test]
    fn the_top_left_quadrant_docks_top_left() {
        assert_eq!(nearest_dock(10.0, 10.0, 1000.0, 800.0), Dock::TopLeft);
    }

    #[test]
    fn the_top_right_quadrant_docks_top_right() {
        assert_eq!(nearest_dock(900.0, 10.0, 1000.0, 800.0), Dock::TopRight);
    }

    #[test]
    fn the_bottom_left_quadrant_docks_bottom_left() {
        assert_eq!(nearest_dock(10.0, 700.0, 1000.0, 800.0), Dock::BottomLeft);
    }

    #[test]
    fn the_bottom_right_quadrant_docks_bottom_right() {
        assert_eq!(nearest_dock(900.0, 700.0, 1000.0, 800.0), Dock::BottomRight);
    }

    #[test]
    fn the_midlines_fall_to_the_bottom_right_dock() {
        assert_eq!(nearest_dock(500.0, 400.0, 1000.0, 800.0), Dock::BottomRight);
    }

    #[test]
    fn each_dock_has_a_vertical_and_horizontal_class() {
        assert_eq!(
            Dock::TopLeft.css_classes(),
            ["fab-dock-top", "fab-dock-left"]
        );
        assert_eq!(
            Dock::TopRight.css_classes(),
            ["fab-dock-top", "fab-dock-right"]
        );
        assert_eq!(
            Dock::BottomLeft.css_classes(),
            ["fab-dock-bottom", "fab-dock-left"]
        );
        assert_eq!(
            Dock::BottomRight.css_classes(),
            ["fab-dock-bottom", "fab-dock-right"]
        );
    }

    #[test]
    fn a_dock_round_trips_through_its_symbol() {
        for dock in [
            Dock::TopLeft,
            Dock::TopRight,
            Dock::BottomLeft,
            Dock::BottomRight,
        ] {
            assert_eq!(Dock::from_symbol(dock.symbol()), Some(dock));
        }
    }

    #[test]
    fn an_unknown_symbol_has_no_dock() {
        assert_eq!(Dock::from_symbol("tonk:middle"), None);
        assert_eq!(Dock::from_symbol(""), None);
    }
}

#[cfg(test)]
mod edge_snap {
    use super::*;

    const INSETS: EdgeInsets = EdgeInsets {
        top: 16.0,
        right: 16.0,
        bottom: 16.0,
        left: 16.0,
    };

    #[test]
    fn a_left_edge_drop_keeps_its_vertical_position() {
        assert_eq!(
            snap_to_nearest_edge(120.0, 300.0, 200.0, 36.0, 1000.0, 800.0, INSETS),
            EdgeSnap {
                edge: Edge::Left,
                left: 16.0,
                top: 300.0,
            }
        );
    }

    #[test]
    fn a_bottom_edge_drop_keeps_its_horizontal_position() {
        assert_eq!(
            snap_to_nearest_edge(420.0, 690.0, 200.0, 36.0, 1000.0, 800.0, INSETS),
            EdgeSnap {
                edge: Edge::Bottom,
                left: 420.0,
                top: 748.0,
            }
        );
    }

    #[test]
    fn a_release_is_clamped_inside_the_safe_insets_before_it_snaps() {
        assert_eq!(
            snap_to_nearest_edge(-40.0, -20.0, 200.0, 36.0, 1000.0, 800.0, INSETS),
            EdgeSnap {
                edge: Edge::Left,
                left: 16.0,
                top: 16.0,
            }
        );
    }
}

#[cfg(test)]
mod geometry {
    use super::*;

    #[test]
    fn dragstart_covers_full_viewport() {
        let b = geometry_box(&FabIntent::DragStart, 1000.0, 800.0);
        assert_eq!(
            b,
            FabBox {
                left: 0.0,
                top: 0.0,
                width: 1000.0,
                height: 800.0
            }
        );
    }

    #[test]
    fn overlay_covers_full_viewport() {
        let b = geometry_box(&FabIntent::Overlay, 1000.0, 800.0);
        assert_eq!(
            b,
            FabBox {
                left: 0.0,
                top: 0.0,
                width: 1000.0,
                height: 800.0
            }
        );
    }

    #[test]
    fn resize_keeps_position_changes_size() {
        let state = FabState {
            x: 100.0,
            y: 50.0,
            w: 320.0,
            h: 64.0,
            dragging: false,
        };
        let b = geometry_box(
            &FabIntent::Resize {
                w: 320.0,
                h: 64.0,
                state,
            },
            1000.0,
            800.0,
        );
        assert_eq!(
            b,
            FabBox {
                left: 100.0,
                top: 50.0,
                width: 320.0,
                height: 64.0
            }
        );
    }

    #[test]
    fn drop_moves_to_point_keeps_size() {
        let state = FabState {
            x: 100.0,
            y: 50.0,
            w: 320.0,
            h: 64.0,
            dragging: true,
        };
        let b = geometry_box(
            &FabIntent::Drop {
                x: 400.0,
                y: 600.0,
                state,
            },
            1000.0,
            800.0,
        );
        assert_eq!(
            b,
            FabBox {
                left: 400.0,
                top: 600.0,
                width: 320.0,
                height: 64.0
            }
        );
    }

    #[test]
    fn dragmove_keeps_full_viewport() {
        // During a drag the iframe stays pinned full-viewport so the pointer
        // coordinate frame never moves under itself; the FAB element is
        // translated *inside* the iframe instead. So DragMove ignores x/y for
        // the iframe box and returns the same full-viewport box as DragStart.
        let state = FabState {
            x: 100.0,
            y: 50.0,
            w: 320.0,
            h: 64.0,
            dragging: true,
        };
        let b = geometry_box(
            &FabIntent::DragMove {
                x: 200.0,
                y: 300.0,
                state,
            },
            1000.0,
            800.0,
        );
        assert_eq!(
            b,
            FabBox {
                left: 0.0,
                top: 0.0,
                width: 1000.0,
                height: 800.0
            }
        );
    }
}

#[cfg(test)]
mod persist {
    use super::*;

    #[test]
    fn claim_json_targets_the_fab_dock() {
        let v = dock_claim_json(Dock::BottomLeft);
        assert_eq!(v["claims"][0]["op"], "assert");
        let app = &v["claims"][0]["application"];
        assert_eq!(app["parameters"]["dock"], "tonk:bottom-left");
        // The dock is a durable profile choice: it must survive commits,
        // not evaporate as a transient at the timestep it's written.
        assert_eq!(app["predicate"]["kind"], "durable");
        assert_eq!(
            app["predicate"]["concept"]["with"]["dock"]["the"],
            "xyz.tonk.fab/dock"
        );
        assert_eq!(app["parameters"]["this"], "state:fab");
    }

    #[test]
    fn collapsed_claim_json_rides_the_fab_entity() {
        let v = collapsed_claim_json(true);
        assert_eq!(v["claims"][0]["op"], "assert");
        let app = &v["claims"][0]["application"];
        // Like the dock, a durable profile choice — not a transient.
        assert_eq!(app["predicate"]["kind"], "durable");
        assert_eq!(
            app["predicate"]["concept"]["with"]["collapsed"]["the"],
            "xyz.tonk.fab/collapsed"
        );
        assert_eq!(app["parameters"]["this"], "state:fab");
        assert_eq!(app["parameters"]["collapsed"], true);
    }

    #[test]
    fn collapsed_resolves_from_conclusions() {
        let rows = serde_json::json!([{ "this": "state:fab", "fields": { "collapsed": true } }]);
        assert_eq!(collapsed_from_conclusions(&rows), Some(true));
        let flat = serde_json::json!([{ "this": "state:fab", "collapsed": false }]);
        assert_eq!(collapsed_from_conclusions(&flat), Some(false));
        assert_eq!(collapsed_from_conclusions(&serde_json::json!([])), None);
        assert_eq!(collapsed_from_conclusions(&serde_json::json!({})), None);
    }

    #[test]
    fn pause_claim_carries_the_space_and_is_transient() {
        let v = pause_claim_json("tonk:pause-sync", "did:key:zSpace", 123.0);
        let app = &v["claims"][0]["application"];
        assert_eq!(v["claims"][0]["op"], "assert");
        // A command is a one-timestep transient, not a durable fact.
        assert_eq!(app["predicate"]["kind"], "transient");
        // The target space rides the command so the handler needn't read it
        // from the dispatch origin — this is what lets pause dispatch from the
        // profile branch and depend on nothing seeded per-space.
        assert_eq!(app["parameters"]["space"], "did:key:zSpace");
        assert_eq!(app["parameters"]["marker"], "tonk:pause-sync");
        assert_eq!(app["parameters"]["time"], 123.0);
        assert_eq!(
            app["predicate"]["concept"]["with"]["space"]["the"],
            "xyz.tonk.pause-sync/space"
        );
        // `this` is omitted so the worker mints it from (descriptor, params).
        assert!(app["parameters"].get("this").is_none());
    }

    #[test]
    fn reads_the_dock_from_a_conclusion_row() {
        // The exact `Conclusion[]` shape `window.tonk.query` returns: the
        // projected `dock` lives under `fields`, not on the row. Reading it
        // off the row directly is the regression that stranded restore at
        // its default even though the fact was persisted.
        let rows = json!([{
            "this": "state:fab",
            "fields": { "this": "state:fab", "dock": "tonk:bottom-left" }
        }]);
        assert_eq!(dock_from_conclusions(&rows), Some(Dock::BottomLeft));
    }

    #[test]
    fn reads_the_dock_from_a_flat_row() {
        // Fallback shape: a flat `row.dock` still resolves, so a projection
        // change degrades to a working read rather than a silent default.
        let rows = json!([{ "dock": "tonk:top-right" }]);
        assert_eq!(dock_from_conclusions(&rows), Some(Dock::TopRight));
    }

    #[test]
    fn empty_result_has_no_dock() {
        assert_eq!(dock_from_conclusions(&json!([])), None);
    }

    #[test]
    fn unknown_symbol_has_no_dock() {
        let rows = json!([{ "fields": { "dock": "tonk:middle" } }]);
        assert_eq!(dock_from_conclusions(&rows), None);
    }
}

#[cfg(test)]
mod menu {
    use super::*;

    #[test]
    fn a_wider_menu_widens_the_segment() {
        // Fractional natural widths round UP so the stamped min-width never
        // undershoots the menu by a subpixel.
        assert_eq!(menu_min_width(220.4, 120.0), Some(221.0));
    }

    #[test]
    fn a_narrower_or_equal_menu_leaves_the_segment_alone() {
        assert_eq!(menu_min_width(80.0, 120.0), None);
        assert_eq!(menu_min_width(120.0, 120.0), None);
    }
}

#[cfg(test)]
mod ratchet {
    use super::*;

    #[test]
    fn a_first_stamp_takes_the_equalized_target() {
        assert_eq!(ratchet_min_width(220.4, 120.0, None), Some(221.0));
    }

    #[test]
    fn a_prior_stamp_is_never_regressed() {
        // Mid-transition rect (120) under-reports a prior 260 stamp; a
        // shrunken menu (221 natural) must not narrow the column.
        assert_eq!(ratchet_min_width(220.4, 120.0, Some(260.0)), Some(260.0));
    }

    #[test]
    fn a_wider_menu_raises_a_prior_stamp() {
        assert_eq!(ratchet_min_width(300.0, 120.0, Some(260.0)), Some(300.0));
    }

    #[test]
    fn a_menu_narrower_than_the_segment_leaves_the_stamp_alone() {
        assert_eq!(ratchet_min_width(80.0, 120.0, Some(260.0)), None);
        assert_eq!(ratchet_min_width(80.0, 120.0, None), None);
    }
}

/// What the share control is doing, and therefore what it shows.
///
/// One click drives the whole cycle: `Idle` → `Copying` (mint in flight, the
/// clipboard already holding an unresolved promise) → `Copied` → back to
/// `Idle`. There is no state in which the control sits waiting for a *second*
/// click to copy — that was the old two-control flow, where minting revealed a
/// separate copy button that then stayed on the bar forever.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShareState {
    /// Resting: offers to share.
    Idle,
    /// A mint is in flight. The clipboard write is already pending on a
    /// promise this state is waiting to resolve.
    Copying,
    /// The mint was refused because the space has no shareable sync remote.
    /// The prompt offering to attach one is up; unlike `Copied`/`Failed` this
    /// does not revert on a timer, because the user is being asked a question.
    Blocked,
    /// The link is on the clipboard. Reverts to `Idle` after
    /// [`COPIED_LINGER_MS`].
    Copied,
    /// The mint or the clipboard write failed. Also reverts to `Idle`, so the
    /// control always returns to offering a retry rather than latching.
    Failed,
}

impl ShareState {
    /// The `data-share-state` attribute value. The view stylesheet keys its
    /// label/spinner swap off this, so the element owns the state and the
    /// stylesheet owns the look.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Copying => "copying",
            Self::Blocked => "blocked",
            Self::Copied => "copied",
            Self::Failed => "failed",
        }
    }

    /// Whether a click should start a new mint. A click while one is already
    /// in flight is dropped: the clipboard holds exactly one pending promise
    /// and a second mint would rotate the credential out from under it.
    pub fn accepts_click(self) -> bool {
        !matches!(self, Self::Copying)
    }

    /// Whether this state settles back to `Idle` on a timer.
    pub fn is_transient(self) -> bool {
        matches!(self, Self::Copied | Self::Failed)
    }
}

/// How long the "copied" (or "failed") confirmation stays up before the
/// control reverts to offering "share" again. Long enough to read, short
/// enough that the bar doesn't keep showing a stale result.
pub const COPIED_LINGER_MS: i32 = 2_000;

#[cfg(test)]
mod share {
    use super::*;

    #[test]
    fn it_offers_a_retry_from_every_settled_state() {
        // Only an in-flight mint refuses a click. Notably `Copied` accepts
        // one: the link rotates per mint, so a second share is a legitimate
        // ask, not a double-submit.
        assert!(ShareState::Idle.accepts_click());
        assert!(ShareState::Copied.accepts_click());
        assert!(ShareState::Failed.accepts_click());
        assert!(!ShareState::Copying.accepts_click());
    }

    #[test]
    fn it_settles_only_the_confirmation_states() {
        // Idle is already the resting state and Copying ends by resolving,
        // not by timing out — neither is on a revert timer.
        assert!(ShareState::Copied.is_transient());
        assert!(ShareState::Failed.is_transient());
        assert!(!ShareState::Idle.is_transient());
        assert!(!ShareState::Copying.is_transient());
    }
}

/// The `with` attribute for a space's content branch: `main@{did}`.
///
/// Each `ui-` child carries its own `with` and subscribes through it —
/// `resolve_with` reads the element's OWN attribute and never walks
/// ancestors, so this must be stamped per element, not inherited.
pub fn space_with(space_did: &str) -> String {
    format!("main@{space_did}")
}

/// The subscribe body for a repository's name.
///
/// An INLINE predicate over the raw `xyz.tonk.repo/name` attribute — it names
/// no concept, so nothing need be seeded on the space's branch and an old
/// `core.yaml` cannot break it. Mirrors `<ui-sync-status>`'s
/// `status_query_body`. `this` is bound to the repo subject by the caller.
pub fn repo_name_query_body(subject: &str) -> Result<String, String> {
    if subject.is_empty() {
        return Err("repo_name_query_body: empty subject".into());
    }
    Ok(json!({
        "predicate": { "with": { "name": {
            "the": "xyz.tonk.repo/name", "as": "Text", "cardinality": "one"
        } } },
        "terms": { "this": subject, "name": { "?": { "name": "name" } } }
    })
    .to_string())
}

#[cfg(test)]
mod space_name {
    use super::*;

    #[test]
    fn it_builds_a_with_string_for_a_space_did() {
        assert_eq!(space_with("did:key:z6Mk"), "main@did:key:z6Mk");
    }

    #[test]
    fn it_queries_the_repo_name_by_raw_attribute() {
        let body = repo_name_query_body("did:key:z6Mk").expect("query body builds");
        // The raw attribute URI — NOT a concept name. Nothing seeded is needed,
        // so an old core.yaml cannot break this read.
        assert!(body.contains("xyz.tonk.repo/name"));
        assert!(body.contains("did:key:z6Mk"));
        assert!(!body.contains("tonk:repository"));
    }

    #[test]
    fn it_rejects_an_empty_subject() {
        assert!(repo_name_query_body("").is_err());
    }
}

/// The subscribe body for a space's member roster.
///
/// ONE inline predicate carrying all three fields on the same entity, in
/// directory mode (`this` unbound), so each member returns as a row. Three
/// separate subscriptions would need client-side row-joining that no existing
/// element does.
///
/// All three are required fields: a member missing a synced name or role is
/// invisible. That matches the seeded view's behaviour, but it is now this
/// element's choice.
pub fn member_roster_query_body() -> String {
    json!({
        "predicate": { "with": {
            "member": { "the": "xyz.tonk.membership/member", "as": "Entity", "cardinality": "one" },
            "role":   { "the": "xyz.tonk.membership/role",   "as": "Entity", "cardinality": "one" },
            "name":   { "the": "xyz.tonk.membership/name",   "as": "Text", "cardinality": "one" }
        } },
        "terms": {
            "this":   { "?": { "name": "this" } },
            "member": { "?": { "name": "member" } },
            "role":   { "?": { "name": "role" } },
            "name":   { "?": { "name": "name" } }
        }
    })
    .to_string()
}

/// The one-shot query body for the signed-in member's own profile DID.
///
/// Reads the PROFILE branch's replica records by raw attribute: every
/// replica there carries `xyz.tonk.replica/profile`, the profile that owns
/// it, so any row answers. Directory mode (`this` unbound). Routeless from
/// the FAB, whose host mounts `with="main@profile:tonk"`.
pub fn self_did_query_body() -> String {
    json!({
        "predicate": { "with": {
            "profile": { "the": "xyz.tonk.replica/profile", "as": "Entity", "cardinality": "one" }
        } },
        "terms": {
            "this":    { "?": { "name": "this" } },
            "profile": { "?": { "name": "profile" } }
        }
    })
    .to_string()
}

/// The profile DID from a `Conclusion[]` answer to [`self_did_query_body`]:
/// the first row's `profile` field. `None` for an empty answer.
pub fn self_did_from_conclusions(rows: &Value) -> Option<String> {
    rows.as_array()?.iter().find_map(|row| {
        row.get("fields")
            .and_then(|fields| fields.get("profile"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    })
}

/// Whether a member holding `role` runs the space: founders and admins
/// may promote (and expel) other members; the worker refuses everyone
/// else, so the roster offers those controls only to them.
pub fn role_manages_members(role: &str) -> bool {
    role == "tonk:founder" || role == "tonk:admin"
}

#[cfg(test)]
mod self_did {
    use super::*;

    #[test]
    fn it_queries_the_replica_profile_by_raw_attribute() {
        let body = self_did_query_body();
        assert!(body.contains("xyz.tonk.replica/profile"));
        assert!(body.contains("\"this\":{\"?\""));
        assert!(!body.contains("tonk:profile"));
    }

    #[test]
    fn it_reads_the_profile_off_the_first_row() {
        let rows = json!([
            { "this": "r1", "fields": { "profile": "did:key:zMe" } },
            { "this": "r2", "fields": { "profile": "did:key:zMe" } }
        ]);
        assert_eq!(
            self_did_from_conclusions(&rows).as_deref(),
            Some("did:key:zMe")
        );
        assert_eq!(self_did_from_conclusions(&json!([])), None);
    }

    #[test]
    fn it_lets_founders_and_admins_manage_members() {
        assert!(role_manages_members("tonk:founder"));
        assert!(role_manages_members("tonk:admin"));
        assert!(!role_manages_members("tonk:member"));
        assert!(!role_manages_members(""));
    }
}

#[cfg(test)]
mod promote {
    use super::*;

    #[test]
    fn it_names_the_space_member_and_chain_on_the_promote_command() {
        let body = promote_claim_json("did:key:zSpace", "did:key:zMember", "3vQB7B6MrGQZaxCu");
        let claim = &body["claims"][0];
        assert_eq!(claim["op"], "assert");
        assert_eq!(claim["application"]["predicate"]["kind"], "transient");
        let parameters = &claim["application"]["parameters"];
        assert_eq!(parameters["space"], "did:key:zSpace");
        assert_eq!(parameters["member"], "did:key:zMember");
        assert_eq!(parameters["chain"], "3vQB7B6MrGQZaxCu");
        let with = &claim["application"]["predicate"]["concept"]["with"];
        assert_eq!(with["chain"]["the"], "xyz.tonk.promote/chain");
    }
}

#[cfg(test)]
mod member_roster {
    use super::*;

    #[test]
    fn it_queries_all_member_fields_in_one_directory_predicate() {
        let body = member_roster_query_body();
        assert!(body.contains("xyz.tonk.membership/name"));
        assert!(body.contains("xyz.tonk.membership/member"));
        assert!(body.contains("xyz.tonk.membership/role"));
        // Directory mode: `this` is an unbound variable, so every member row
        // comes back. A bound `this` would return one. `serde_json::Value`'s
        // `Display` is the COMPACT formatter (no spaces around `:`/`,`), so
        // the substring below has none either — a pretty-printed literal
        // (with spaces) never matches the actual body.
        assert!(body.contains("\"this\":{\"?\""));
        // No concept named — nothing seeded is consulted.
        assert!(!body.contains("tonk:member"));
    }
}

/// The subscribe body for the profile's space list.
///
/// Reads the PROFILE branch's account-level space directory by raw attribute.
/// Directory mode (`this` unbound), so every convergent space entry returns as
/// a row. `name` is optional for vintage entries that predate the mirror.
pub fn space_list_query_body() -> String {
    json!({
        "predicate": { "with": {
            "subject": { "the": "xyz.tonk.space/subject", "as": "Entity", "cardinality": "one" },
            "name":    { "the": "xyz.tonk.space/name",    "as": "Text",   "cardinality": "one", "optional": true },
            "status":  { "the": "xyz.tonk.space/status",  "as": "Entity", "cardinality": "one" }
        } },
        "terms": {
            "this":    { "?": { "name": "this" } },
            "subject": { "?": { "name": "subject" } },
            "name":    { "?": { "name": "name" } },
            "status":  { "?": { "name": "status" } }
        }
    })
    .to_string()
}

/// Replace a subscription's keyed snapshot while preserving delivery order.
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) fn reset_keyed_rows<T>(
    rows: &mut Vec<(String, T)>,
    conclusions: impl IntoIterator<Item = (String, T)>,
) {
    rows.clear();
    for (id, row) in conclusions {
        match rows.iter_mut().find(|(existing_id, _)| existing_id == &id) {
            Some(existing) => existing.1 = row,
            None => rows.push((id, row)),
        }
    }
}

#[cfg(test)]
mod space_list {
    use super::*;

    #[test]
    fn it_queries_the_account_space_directory_by_raw_attribute() {
        let body = space_list_query_body();
        assert!(body.contains("xyz.tonk.space/subject"));
        assert!(body.contains("xyz.tonk.space/name"));
        assert!(body.contains("xyz.tonk.space/status"));
        assert!(
            !body.contains("xyz.tonk.replica/"),
            "the account directory replaced per-device replica rows: {body}"
        );
        // Directory mode over every account space entry.
        assert!(body.contains("\"this\":{\"?\""));
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
        assert_eq!(
            parsed["predicate"]["with"]["name"]["optional"], true,
            "a vintage directory entry without a name must remain listable"
        );
    }

    #[test]
    fn a_reset_keeps_one_row_per_directory_entity() {
        let mut rows = vec![("stale".to_owned(), "stale")];

        reset_keyed_rows(
            &mut rows,
            [
                ("directory:space".to_owned(), "first"),
                ("directory:space".to_owned(), "latest"),
            ],
        );

        assert_eq!(rows, vec![("directory:space".to_owned(), "latest")]);
    }
}

/// The subscribe body for the signed-in member's profile display name.
///
/// An INLINE predicate over the raw `xyz.tonk.profile/display-name`
/// attribute — not the deleted `tonk:profile/name-view`, which was a
/// per-space-seeded template. Directory mode (`this` unbound): the profile
/// branch carries at most one such row (the member's own override), so no
/// subject needs binding, unlike [`repo_name_query_body`]'s repo-scoped
/// read. Absent until the user renames (the worker's `petname` fallback is
/// computed, not persisted), so an empty result is expected, not an error.
pub fn profile_name_query_body() -> String {
    json!({
        "predicate": { "with": { "name": {
            "the": "xyz.tonk.profile/display-name", "as": "Text", "cardinality": "one"
        } } },
        "terms": { "this": { "?": { "name": "this" } }, "name": { "?": { "name": "name" } } }
    })
    .to_string()
}

#[cfg(test)]
mod profile_name {
    use super::*;

    #[test]
    fn it_queries_the_profile_display_name_by_raw_attribute() {
        let body = profile_name_query_body();
        // The raw attribute URI — NOT the deleted `tonk:profile/name-view`.
        // Nothing seeded is needed, so an old core.yaml cannot break this.
        assert!(body.contains("xyz.tonk.profile/display-name"));
        assert!(!body.contains("tonk:profile/name"));
        // Directory mode: `this` is unbound (see `member_roster_query_body`'s
        // test for why the compact-JSON substring below has no spaces).
        assert!(body.contains("\"this\":{\"?\""));
    }
}

/// Build a `TransactRequest` body for `tonk/rename-repository`.
///
/// A transient carrying the target `space` and the new `value`. Dispatched
/// routeless via `window.tonk.transact`, so it lands on the FAB's own
/// `main@profile:tonk`; the worker's handler reads `space` to rename that
/// repository — nothing space-side is required. `this` is omitted so the
/// worker mints it from `(descriptor, parameters)`.
///
/// An empty `name` is omitted entirely: the extractor drops empty fields, so
/// a blank would store no fact and the command would never fire.
pub fn rename_repo_claim_json(space: &str, name: &str) -> Value {
    let mut parameters = json!({
        "space": space,
        "rename-repository": "tonk:repository"
    });
    if !name.is_empty() {
        parameters["value"] = json!(name);
    }
    json!({
        "claims": [{
            "op": "assert",
            "application": {
                "predicate": {
                    "kind": "transient",
                    "concept": {
                        "description": "Rename a space's repository from the FAB.",
                        "with": {
                            "value":            { "the": "dom.event.current-target/value", "as": "Text" },
                            "space":            { "the": "xyz.tonk.rename-repository/space", "as": "Entity" },
                            "rename-repository": { "the": "dom.event.current-target.dataset/rename-repository", "as": "Entity" }
                        }
                    }
                },
                "parameters": parameters
            }
        }]
    })
}

#[cfg(test)]
mod rename_repo {
    use super::*;

    #[test]
    fn it_inlines_the_rename_descriptor_and_names_its_target_space() {
        let claim = rename_repo_claim_json("did:key:z6Mk", "Renamed");
        let text = claim.to_string();
        // The descriptor rides WITH the claim — nothing seeded is consulted.
        assert!(text.contains("xyz.tonk.rename-repository/space"));
        assert!(text.contains("dom.event.current-target/value"));
        assert!(text.contains("did:key:z6Mk"));
        assert!(text.contains("Renamed"));
    }

    #[test]
    fn it_omits_an_empty_name_rather_than_sending_a_blank() {
        // The extractor drops empty fields; a blank would store no fact and the
        // handler would never fire. The descriptor's `with.value` mapping is
        // schema metadata and stays present regardless — what must be absent
        // is the `value` PARAMETER, the thing that actually becomes a fact.
        let claim = rename_repo_claim_json("did:key:z6Mk", "");
        assert!(
            claim["claims"][0]["application"]["parameters"]
                .get("value")
                .is_none()
        );
    }
}

/// The `space/create` claim shape, defined in `tonk-worker-api` so every
/// dispatcher builds the same transient — the FAB's wizard and the
/// browser tests that drive creation the way the app does. Re-exported
/// here because this module is where the FAB's other claim builders live.
pub use tonk_worker_api::create_space_claim_json;

#[cfg(test)]
mod create_space {
    use super::*;

    #[test]
    fn it_uses_the_declared_form_attribute_uris_for_create_space() {
        let claim = create_space_claim_json("Untitled");
        let text = claim.to_string();
        // Verbatim, kebab-cased as declared — the handler matches on these.
        // Every control is read at `/value`: the segment after the control
        // name is the JS property the browser's extractor would have read,
        // so a descriptive leaf resolves to `undefined` there and the
        // handler would never see the field here.
        assert!(text.contains("dom.event.current-target.elements.name/value"));
        let params = &claim["claims"][0]["application"]["parameters"];
        assert_eq!(params["name"], "Untitled");
    }

    /// The claim declares `name` and nothing else.
    ///
    /// Both dropped fields cost a whole create when they were declared
    /// and unbacked: a field the form does not carry fails to resolve
    /// against the event, and the command aborts before the handler runs
    /// — which is how `template` broke creation outright after the Hub
    /// lost its wizard.
    ///
    /// `remote` is gone for a second reason: the page supplying one made
    /// every create look like a deliberate choice of this server, so a
    /// space created before anyone registered was wired to a service that
    /// refuses to serve it. The worker resolves it from the account.
    #[test]
    fn it_declares_only_the_field_the_form_carries() {
        let claim = create_space_claim_json("Untitled");
        let with = &claim["claims"][0]["application"]["predicate"]["concept"]["with"];
        let declared: Vec<&String> = with.as_object().expect("a with block").keys().collect();
        assert_eq!(declared, vec!["name"], "only `name` is backed by a control");

        let params = &claim["claims"][0]["application"]["parameters"];
        assert!(params.get("remote").is_none(), "the worker resolves it");
        assert!(params.get("template").is_none(), "template seeding is gone");
    }
}

/// Build a `TransactRequest` JSON body for the `profile/rename` command.
///
/// Inlines the descriptor `profile.yaml` declares for `command!:
/// &profile/rename` — so renaming the signed-in member depends on nothing
/// seeded on the profile branch. `this` is omitted so the worker mints it
/// from `(descriptor, parameters)`.
///
/// An empty `name` is omitted entirely, mirroring [`rename_repo_claim_json`]:
/// the extractor drops empty fields, and `ProfileRename`'s `name` field is
/// required, so an absent fact means the command doesn't decode at all — the
/// same "commit a blank, nothing changes" behaviour `ProfileRenameHandler`
/// itself would otherwise have to special-case.
pub fn profile_rename_claim_json(name: &str) -> Value {
    let mut parameters = json!({ "marker": "tonk:profile" });
    if !name.is_empty() {
        parameters["name"] = json!(name);
    }
    json!({
        "claims": [{
            "op": "assert",
            "application": {
                "predicate": {
                    "kind": "transient",
                    "concept": {
                        "description": "Rename the signed-in member (set their display name).",
                        "with": {
                            "name":   { "the": "dom.event.current-target/value", "as": "Text" },
                            "marker": { "the": "dom.event.current-target.dataset/rename", "as": "Entity" }
                        }
                    }
                },
                "parameters": parameters
            }
        }]
    })
}

#[cfg(test)]
mod profile_rename {
    use super::*;

    #[test]
    fn it_inlines_the_rename_descriptor_and_marks_the_profile() {
        let claim = profile_rename_claim_json("Ada");
        let with = &claim["claims"][0]["application"]["predicate"]["concept"]["with"];
        // Assert the EXACT attribute, not a substring: `contains` on
        // "…dataset/rename" also matches "…dataset/rename-repository", so a
        // marker silently pointed at the repo-rename attribute would still
        // pass. That collision is not hypothetical — both commands once
        // derived `dataset/rename` and every space rename also renamed the
        // profile, because decoding matches on which attributes are present
        // and never on their values. `dialog-reactor`'s
        // `it_does_not_decode_a_repo_rename_as_a_profile_rename` pins the
        // invariant; this pins the claim this crate actually builds.
        assert_eq!(with["name"]["the"], "dom.event.current-target/value");
        assert_eq!(
            with["marker"]["the"],
            "dom.event.current-target.dataset/rename"
        );
        let params = &claim["claims"][0]["application"]["parameters"];
        assert_eq!(params["name"], "Ada");
        assert_eq!(params["marker"], "tonk:profile");
    }

    #[test]
    fn it_omits_an_empty_name_rather_than_sending_a_blank() {
        let claim = profile_rename_claim_json("");
        assert!(
            claim["claims"][0]["application"]["parameters"]
                .get("name")
                .is_none()
        );
    }
}

/// Build a `TransactRequest` JSON body for the `tonk:invite` command.
///
/// Inlines the descriptor (mirroring `core.yaml`'s `command!: &tonk/invite`)
/// plus a `space` field — mirroring [`pause_claim_json`]'s `space` — so the
/// handler mints for the named repository instead of reading the dispatch
/// origin, which is empty when `<tonk-share>` dispatches routeless from the
/// FAB's own profile-branch context. `this` is omitted so the worker mints
/// it from `(descriptor, parameters)`; `time` makes each click a distinct
/// transient so repeated Share clicks reliably re-fire the handler and
/// rotate the credential.
pub fn invite_claim_json(space: &str, time: f64) -> Value {
    json!({
        "claims": [{
            "op": "assert",
            "application": {
                "predicate": {
                    "kind": "transient",
                    "concept": {
                        "description": "Mint a repo invite — generates a membership keypair and delegation.",
                        "with": {
                            "time":   { "the": "dom.event/time-stamp", "as": "Float" },
                            "space":  { "the": "xyz.tonk.invite/space", "as": "Entity" },
                            "marker": { "the": "dom.event.current-target.dataset/invite", "as": "Entity" }
                        }
                    }
                },
                "parameters": {
                    "time": time,
                    "space": space,
                    "marker": "tonk:invite"
                }
            }
        }]
    })
}

/// The `tonk:enable-sync` claim the share control dispatches when a user
/// accepts the offer to turn sync on.
///
/// `share` adds the marker asking the worker to mint an invite once the
/// remote is attached. When false the marker is omitted from BOTH the
/// declared concept and the parameters — a declared field with no value makes
/// the assert incomplete, so the transient would commit and match nothing.
pub fn enable_sync_claim_json(space: &str, remote: &str, share: bool, time: f64) -> Value {
    let mut with = json!({
        "time":   { "the": "dom.event/time-stamp", "as": "Float" },
        "space":  { "the": "xyz.tonk.enable-sync/space", "as": "Entity" },
        "marker": { "the": "dom.event.current-target.dataset/enable-sync", "as": "Entity" }
    });
    let mut parameters = json!({
        "time": time,
        "space": space,
        "marker": "tonk:enable-sync"
    });
    // An empty remote is omitted rather than sent as `""`: the handler
    // then resolves where this account syncs from its own recorded
    // provider. The page used to derive `origin + /ucan/` for itself,
    // which a sealed guest cannot do until the portal bridge has told it
    // its own origin — a share before that arrived silently did nothing.
    if !remote.is_empty() {
        with["remote"] = json!({ "the": "xyz.tonk.enable-sync/remote", "as": "Text" });
        parameters["remote"] = json!(remote);
    }
    if share {
        with["share"] = json!({ "the": "xyz.tonk.enable-sync/share", "as": "Entity" });
        parameters["share"] = json!("tonk:share");
    }
    json!({
        "claims": [{
            "op": "assert",
            "application": {
                "predicate": {
                    "kind": "transient",
                    "concept": {
                        "description": "Attach a sync remote to a space, and share it.",
                        "with": with
                    }
                },
                "parameters": parameters
            }
        }]
    })
}

#[cfg(test)]
mod invite {
    use super::*;

    #[test]
    fn it_names_the_target_space_on_the_invite() {
        let claim = invite_claim_json("did:key:z6Mk", 1.0);
        assert!(claim.to_string().contains("xyz.tonk.invite/space"));
        assert!(claim.to_string().contains("did:key:z6Mk"));
        let app = &claim["claims"][0]["application"];
        assert_eq!(app["parameters"]["space"], "did:key:z6Mk");
        assert_eq!(app["parameters"]["marker"], "tonk:invite");
        assert_eq!(app["parameters"]["time"], 1.0);
        assert!(app["parameters"].get("this").is_none());
    }
}

/// The subscribe body for the FAB's minted invite link.
///
/// An INLINE predicate over the raw `xyz.tonk.credential/link` attribute —
/// not the rule-derived `tonk:agent-invite` view the seeded FAB used to
/// read: rules, like views, are frozen at whatever `core.yaml` seeded a
/// space with, so reading the raw attribute instead depends on nothing
/// seeded. `this` is bound to the space's own subject DID (the same entity
/// `InviteHandler` keys the credential by), mirroring
/// [`repo_name_query_body`].
pub fn invite_link_query_body(subject: &str) -> Result<String, String> {
    if subject.is_empty() {
        return Err("invite_link_query_body: empty subject".into());
    }
    Ok(json!({
        "predicate": { "with": { "link": {
            "the": "xyz.tonk.credential/link", "as": "Text", "cardinality": "one"
        } } },
        "terms": { "this": subject, "link": { "?": { "name": "link" } } }
    })
    .to_string())
}

#[cfg(test)]
mod invite_link {
    use super::*;

    #[test]
    fn it_reads_the_invite_link_not_the_rule_derived_agent_invite() {
        let body = invite_link_query_body("did:key:z6Mk").expect("query body builds");
        // `tonk:agent-invite` is rule-derived; rules are frozen like views.
        assert!(body.contains("xyz.tonk.credential/link"));
        assert!(!body.contains("agent-invite"));
        assert!(body.contains("did:key:z6Mk"));
    }

    #[test]
    fn it_rejects_an_empty_subject() {
        assert!(invite_link_query_body("").is_err());
    }
}

/// The subscribe body for a refused share.
///
/// An INLINE predicate over the raw `xyz.tonk.share/*` attributes, for the
/// same reason [`invite_link_query_body`] is inline: rules and views are
/// frozen at whatever `core.yaml` seeded a space with, so reading raw
/// attributes depends on nothing seeded and works on spaces that predate this
/// feature. `this` binds to the space's subject DID, the entity the worker
/// keys the refusal by.
/// Whether this profile's account is registered and served.
///
/// The bar shows "log in to share" or the copy row depending on this.
/// A live query rather than a `GET /api/account` probe: the answer
/// changes the moment registration completes, and a one-shot read taken
/// on connect is stale by then — the bar kept offering to log in to
/// someone who just had.
///
/// Subscribes to the REGISTRATION fact: an account that enrolled has one,
/// whatever happened after. Resolving at all is what tells the bar an
/// account exists — the old query required a provider, so a just-enrolled
/// account read as no account and the bar went on offering "log in to
/// share" to someone who had just registered.
///
/// Whether that account is SERVED is a separate fact
/// (`xyz.tonk.account/activated-at`), read by whoever needs it. There is no
/// status string to compare against in either case.
pub fn account_customer_query_body() -> String {
    json!({
        "predicate": { "with": {
            "registered_at": {
                "the": "xyz.tonk.account/registered-at", "as": "UnsignedInteger",
                "cardinality": "one"
            },
            "email": {
                "the": "xyz.tonk.account/customer-email", "as": "Text", "cardinality": "one"
            }
        } },
        "terms": {
            // `this` has to be bound, even when the subject is unknown:
            // an unbound one is a query error, not a wildcard, and the
            // whole subscription fails with `UnboundVariable`. It failed
            // silently — no frames ever arrived, so the bar fell back to
            // its defaults and went on offering "log in to share" to
            // someone who had just registered.
            //
            // A variable rather than a subject because the account's DID
            // is not known here; there is one customer row per profile,
            // so whatever binds is it.
            "this": { "?": { "name": "account" } },
            "registered_at": { "?": { "name": "registered_at" } },
            "email": { "?": { "name": "email" } }
        }
    })
    .to_string()
}

/// Whether this account is SERVED: the activation fact, whose presence is
/// the whole answer.
///
/// A separate subscription from the registration one because the two
/// resolve independently — an enrolled account has a registration row and
/// no activation row, and a query requiring both would resolve for neither
/// (the join-status lesson). No status string is compared in either.
pub fn account_active_query_body() -> String {
    json!({
        "predicate": { "with": {
            ACTIVATION_FIELD: {
                "the": "xyz.tonk.account/activated-at", "as": "UnsignedInteger",
                "cardinality": "one"
            }
        } },
        "terms": {
            "this": { "?": { "name": "account" } },
            ACTIVATION_FIELD: { "?": { "name": ACTIVATION_FIELD } },
        }
    })
    .to_string()
}

/// The frame field whose presence says the account activated.
///
/// Named once because the subscription that asks for it
/// ([`account_active_query_body`]) and the banner reader that looks for
/// it (`activation::apply`) must agree. They drifted once — the query
/// moved from carrying a `provider` to the bare activation timestamp and
/// the reader kept probing `provider` — so every activation frame read
/// as not-yet-active and the pending banner lingered for the life of the
/// page after the account was served.
pub const ACTIVATION_FIELD: &str = "activated_at";

/// The share control's single subscription: where this space's invite
/// has got to.
///
/// One row, replacing the pair of queries the control used to run (one
/// for the link, one for the refusal). `status` is what the control
/// reads; `url` is present only once granted, so it is `maybe:` and a
/// request in flight still resolves.
///
/// See `plan/share-intent.md`.
pub fn invite_state_query_body(subject: &str) -> Result<String, String> {
    if subject.is_empty() {
        return Err("invite_state_query_body: empty subject".into());
    }
    Ok(json!({
        "predicate": {
            "with": {
                "status": {
                    "the": "xyz.tonk.invite/status", "as": "Entity", "cardinality": "one"
                },
                // Optional is a FLAG on the attribute, not a separate
                // block. `maybe:` is the YAML notation's spelling, which
                // the analyzer translates into this; a hand-built wire
                // query that uses it declares nothing, and the field is
                // dropped from the projection without an error.
                //
                // That cost a whole share: the invite minted, the row
                // said `granted`, and the url the control settles the
                // clipboard write with was simply absent — so it waited
                // out its timeout and reported "could not copy".
                "url": {
                    "the": "xyz.tonk.invite/url", "as": "Text",
                    "cardinality": "one", "optional": true
                }
            }
        },
        "terms": {
            "this": subject,
            "status": { "?": { "name": "status" } },
            "url": { "?": { "name": "url" } }
        }
    })
    .to_string())
}

pub fn share_blocked_query_body(subject: &str) -> Result<String, String> {
    if subject.is_empty() {
        return Err("share_blocked_query_body: empty subject".into());
    }
    Ok(json!({
        "predicate": { "with": {
            "blocked": { "the": "xyz.tonk.share/blocked", "as": "Text", "cardinality": "one" },
            "detail":  { "the": "xyz.tonk.share/detail",  "as": "Text", "cardinality": "one" },
            "time":    { "the": "xyz.tonk.share/time",    "as": "Float", "cardinality": "one" }
        } },
        "terms": {
            "this": subject,
            "blocked": { "?": { "name": "blocked" } },
            "detail":  { "?": { "name": "detail" } },
            "time":    { "?": { "name": "time" } }
        }
    })
    .to_string())
}

/// This page's default UCAN access-service endpoint: `origin + /ucan/`.
///
/// A fallback, not the source of truth. Where a space syncs is the
/// account's own registration, recorded by the service in its receipt
/// and resolved worker-side; the create wizard passes no remote at all
/// for exactly that reason. This remains for the share path, which still
/// supplies one, and is the next derivation to remove.
///
/// Kept pure (origin in, URL out) so it is testable off-browser; the
/// How long a share click waits for a result before giving up.
///
/// Without this the control has no failure path at all for anything other
/// than an explicit refusal: a mint that never lands leaves the clipboard
/// write open and the button pinned on `copying`, which
/// [`ShareState::accepts_click`] refuses, so the button is dead for the rest
/// of the session. Generous, because the enable-sync path holds the write
/// across a network round-trip.
pub const SHARE_TIMEOUT_MS: i32 = 15_000;

#[cfg(test)]
mod enable_sync_claim {
    use super::*;

    #[test]
    fn it_names_the_space_remote_and_share_marker() {
        let claim = enable_sync_claim_json("did:key:z6Mk", "https://tonk.network/ucan/", true, 7.0);
        let app = &claim["claims"][0]["application"];
        assert_eq!(app["parameters"]["space"], "did:key:z6Mk");
        assert_eq!(app["parameters"]["remote"], "https://tonk.network/ucan/");
        assert_eq!(app["parameters"]["share"], "tonk:share");
        assert_eq!(app["parameters"]["marker"], "tonk:enable-sync");
        assert_eq!(app["parameters"]["time"], 7.0);
        assert_eq!(
            app["predicate"]["concept"]["description"],
            "Attach a sync remote to a space, and share it."
        );

        // The `with` declaration is what the worker actually matches on: a
        // typo here compiles and passes every assertion on `parameters`
        // above, then silently no-ops at runtime because the transient
        // commits and matches no handler. Pin every declared attribute name.
        let with = &app["predicate"]["concept"]["with"];
        assert_eq!(with["time"]["the"], "dom.event/time-stamp");
        assert_eq!(with["space"]["the"], "xyz.tonk.enable-sync/space");
        assert_eq!(with["remote"]["the"], "xyz.tonk.enable-sync/remote");
        assert_eq!(
            with["marker"]["the"],
            "dom.event.current-target.dataset/enable-sync"
        );
        assert_eq!(with["share"]["the"], "xyz.tonk.enable-sync/share");
    }

    #[test]
    fn it_omits_the_share_marker_when_not_sharing() {
        let claim = enable_sync_claim_json("did:key:z6Mk", "https://x.test/ucan/", false, 1.0);
        let app = &claim["claims"][0]["application"];
        assert!(app["parameters"].get("share").is_none());
        assert!(
            app["predicate"]["concept"]["with"].get("share").is_none(),
            "an omitted parameter must not be declared, or the assert is incomplete"
        );
    }
}

#[cfg(test)]
mod account_state_query {
    use super::*;

    /// The subject must be BOUND.
    ///
    /// An unbound `this` is a query error, not a wildcard: the whole
    /// subscription failed with `UnboundVariable` on every attempt, no
    /// frame ever arrived, and the bar silently fell back to its
    /// defaults — reporting stale sync and offering to log in to an
    /// account that was already active.
    #[test]
    fn it_binds_its_subject() {
        let body = account_customer_query_body();
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
        assert!(
            parsed["terms"]["this"].is_object(),
            "`this` must be bound, got {body}",
        );
        assert!(body.contains("xyz.tonk.account/registered-at"));
        assert!(body.contains("xyz.tonk.account/customer-email"));
    }
}

#[cfg(test)]
mod invite_state_query {
    use super::*;

    #[test]
    fn it_reads_the_status_and_an_optional_url() {
        let body = invite_state_query_body("did:key:z6Mk").expect("query body builds");
        // Raw attribute URIs, not a concept name: nothing seeded is
        // needed, so an old core.yaml cannot break the read.
        assert!(body.contains("xyz.tonk.invite/status"));
        assert!(body.contains("xyz.tonk.invite/url"));
        assert!(body.contains("did:key:z6Mk"));
        // `url` must be optional or a request in flight — which has a
        // status and no url yet — would never resolve, and the control
        // would sit on `share` through the whole mint.
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
        // Optional is a FLAG, not a block. `maybe:` is the YAML
        // notation's spelling; on the wire an optional attribute stays
        // in `with` and carries `"optional": true`. A query that used
        // `maybe:` declared nothing at all — the field was dropped from
        // the projection with no error, and the share control waited out
        // its timeout for a url that was sitting in the row.
        assert!(
            parsed["predicate"]["maybe"].is_null(),
            "there is no `maybe` block on the wire, got {body}",
        );
        assert_eq!(
            parsed["predicate"]["with"]["url"]["optional"], true,
            "url must be marked optional in place, got {body}",
        );
        assert!(
            parsed["predicate"]["with"]["status"]["optional"].is_null(),
            "status is required, got {body}",
        );
    }

    #[test]
    fn it_refuses_an_empty_subject() {
        assert!(invite_state_query_body("").is_err());
    }
}

#[cfg(test)]
mod share_blocked_query {
    use super::*;

    #[test]
    fn it_reads_the_raw_share_attributes() {
        let body = share_blocked_query_body("did:key:z6Mk").expect("query body builds");
        assert!(body.contains("xyz.tonk.share/blocked"));
        assert!(body.contains("xyz.tonk.share/detail"));
        assert!(body.contains("xyz.tonk.share/time"));
        assert!(body.contains("did:key:z6Mk"));
    }

    #[test]
    fn it_rejects_an_empty_subject() {
        assert!(share_blocked_query_body("").is_err());
    }

    /// The activation subscription asks for the field its reader checks.
    ///
    /// Two halves of one contract, held together only by this test. They
    /// came apart once: the query moved from carrying a `provider` to
    /// the bare activation timestamp, the banner reader kept probing
    /// `provider`, and because a field that is not requested is simply
    /// absent from every frame, activation was never noticed — the
    /// pending banner lingered for the life of the page after the
    /// account was served.
    #[test]
    fn it_reads_the_field_the_activation_subscription_asks_for() {
        let body: serde_json::Value =
            serde_json::from_str(&account_active_query_body()).expect("the query body is JSON");
        assert!(
            body["predicate"]["with"][ACTIVATION_FIELD].is_object(),
            "the subscription must request `{ACTIVATION_FIELD}`, which is what the \
             banner reader checks; got {}",
            body["predicate"]["with"],
        );
        assert!(
            body["terms"][ACTIVATION_FIELD].is_object(),
            "`{ACTIVATION_FIELD}` must be bound in `terms` or it never reaches a frame",
        );
    }
}

#[cfg(test)]
mod share_state_blocked {
    use super::*;

    #[test]
    fn it_accepts_a_click_and_does_not_time_out() {
        assert_eq!(ShareState::Blocked.as_str(), "blocked");
        // A refused share must be retryable straight away.
        assert!(ShareState::Blocked.accepts_click());
        // The dialog is up; nothing should quietly revert it.
        assert!(!ShareState::Blocked.is_transient());
    }
}

/// The `as` type variants the worker's query and transact bodies accept.
///
/// `String` is NOT one of them: the wire enum is dialog's, not Rust's, and
/// a body carrying `"as": "String"` is rejected outright — queries with
/// `invalid query body: data did not match any variant of untagged enum
/// Predicate`, claims with `unknown variant `String``. That failure is
/// total and silent from the UI's side: the subscription never delivers and
/// the command never fires, so a chip renders its fallback and a button
/// spins forever, exactly as if the data were merely absent.
///
/// Asserting a body *contains* an attribute URI does not catch this — the
/// body is well-formed JSON either way. Only the variant name matters.
#[cfg(test)]
const WIRE_TYPES: [&str; 6] = ["Text", "Entity", "Float", "Integer", "Bytes", "Boolean"];

#[cfg(test)]
mod wire_types {
    use super::*;

    /// Every `as` in every body this module builds must name a variant the
    /// worker accepts. Regression: all nine originally said `String`, which
    /// killed every FAB read and every FAB command at once.
    #[test]
    fn it_only_names_type_variants_the_worker_accepts() {
        let bodies: Vec<String> = vec![
            repo_name_query_body("did:key:zX").expect("repo name"),
            member_roster_query_body(),
            space_list_query_body(),
            profile_name_query_body(),
            invite_link_query_body("did:key:zX").expect("invite link"),
            rename_repo_claim_json("did:key:zX", "N").to_string(),
            create_space_claim_json("N").to_string(),
            profile_rename_claim_json("N").to_string(),
            invite_claim_json("did:key:zX", 1.0).to_string(),
            pause_claim_json("tonk:pause-sync", "did:key:zX", 1.0).to_string(),
            dock_claim_json(crate::logic::Dock::BottomRight).to_string(),
        ];

        for body in &bodies {
            for found in body.split("\"as\":").skip(1) {
                let variant = found
                    .trim_start()
                    .trim_start_matches('"')
                    .split('"')
                    .next()
                    .unwrap_or("");
                assert!(
                    WIRE_TYPES.contains(&variant),
                    "`as: {variant}` is not a wire type the worker accepts \
                     (expected one of {WIRE_TYPES:?}) in body: {body}"
                );
            }
        }
    }
}
