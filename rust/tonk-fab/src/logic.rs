//! Pure geometry logic for the FAB element.
//!
//! No DOM imports — compiles and tests on the native target.

use serde_json::{Value, json};

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

/// The four corners the FAB is allowed to rest in. A drop snaps to the nearest
/// one: the vertical half of the viewport picks top vs bottom, the horizontal
/// half picks left vs right.
///
/// The resting spot is expressed as two CSS classes on `<tonk-fab>` — a vertical
/// one (`fab-dock-top` / `fab-dock-bottom`) and a horizontal one
/// (`fab-dock-left` / `fab-dock-right`) — and the actual pixel placement + the
/// submenu open-direction live in the view's stylesheet (profile.yaml). This
/// enum is only the small decision Rust still owns — which corner a drop lands
/// in — plus its persisted symbol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dock {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
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

/// Pick the corner nearest a drop. The vertical half of the viewport (height
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

/// The telescope animation duration, in milliseconds — each tile's
/// `max-width` transition (wireframe `--dur: .4s`).
pub const TELESCOPE_MS: u64 = 400;

/// Milliseconds of stagger between consecutive tiles as the bar telescopes
/// open/closed (wireframe `CP_STAG`). The tiles animate in sequence rather
/// than together, so the bar reads as unfolding rather than snapping.
pub const TELESCOPE_STAGGER_MS: u64 = 70;

/// The `transition-delay` (ms) for tile `i` of `n` in the telescope.
///
/// Expanding runs inner-to-outer (`i * stagger`), collapsing runs
/// outer-to-inner (`(n - 1 - i) * stagger`), so in both directions the tile
/// nearest the anchoring circle leads and the far edge trails — the bar looks
/// like it grows from / retracts into the circle. Mirrors the wireframe's
/// `(collapsed ? nTiles - 1 - i : i) * CP_STAG`.
pub fn telescope_delay_ms(index: usize, count: usize, collapsing: bool) -> u64 {
    let step = if collapsing {
        count.saturating_sub(1).saturating_sub(index)
    } else {
        index
    };
    step as u64 * TELESCOPE_STAGGER_MS
}

/// How long the whole telescope takes to settle: the last tile's start delay
/// plus one transition duration, with a small cushion. Used to schedule the
/// post-animation `settled` state that unclamps `max-width` so content can
/// reflow freely.
pub fn telescope_settle_ms(count: usize) -> u64 {
    let last = count.saturating_sub(1) as u64 * TELESCOPE_STAGGER_MS;
    last + TELESCOPE_MS + 160
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
mod telescope {
    use super::*;

    #[test]
    fn expanding_leads_from_the_inner_tile() {
        // Inner-to-outer: tile 0 starts first, later tiles trail.
        assert_eq!(telescope_delay_ms(0, 3, false), 0);
        assert_eq!(telescope_delay_ms(1, 3, false), TELESCOPE_STAGGER_MS);
        assert_eq!(telescope_delay_ms(2, 3, false), 2 * TELESCOPE_STAGGER_MS);
    }

    #[test]
    fn collapsing_leads_from_the_outer_tile() {
        // Outer-to-inner: the far tile (index 2) starts first, tile 0 trails —
        // so it still reads as retracting toward the circle.
        assert_eq!(telescope_delay_ms(2, 3, true), 0);
        assert_eq!(telescope_delay_ms(1, 3, true), TELESCOPE_STAGGER_MS);
        assert_eq!(telescope_delay_ms(0, 3, true), 2 * TELESCOPE_STAGGER_MS);
    }

    #[test]
    fn settle_covers_the_last_tile_plus_a_duration() {
        // 3 tiles: last starts at 2*stagger, runs one duration, + cushion.
        assert_eq!(
            telescope_settle_ms(3),
            2 * TELESCOPE_STAGGER_MS + TELESCOPE_MS + 160
        );
        // Degenerate: a single tile has no stagger.
        assert_eq!(telescope_settle_ms(1), TELESCOPE_MS + 160);
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
            "the": "xyz.tonk.repo/name", "as": "String", "cardinality": "one"
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
        "rename": "tonk:repository"
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
                            "value":  { "the": "dom.event.current-target/value", "as": "String" },
                            "space":  { "the": "xyz.tonk.rename-repository/space", "as": "Entity" },
                            "rename": { "the": "dom.event.current-target.dataset/rename", "as": "Entity" }
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
