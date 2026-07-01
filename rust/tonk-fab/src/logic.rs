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

/// The two spots the FAB is allowed to rest in. It docks against the left edge
/// only, so the sole choice is vertical: hug the top-left or the bottom-left
/// corner.
///
/// The resting spot is expressed as a CSS class on `<tonk-fab>`
/// (`fab-dock-top` / `fab-dock-bottom`) and the actual pixel placement + the
/// submenu open-direction live in the view's stylesheet (profile.yaml). This
/// enum is only the small decision Rust still owns — which of the two docks a
/// drop lands in — plus its persisted symbol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dock {
    TopLeft,
    BottomLeft,
}

impl Dock {
    /// The CSS class the view stylesheet keys position + menu direction off.
    pub fn css_class(self) -> &'static str {
        match self {
            Dock::TopLeft => "fab-dock-top",
            Dock::BottomLeft => "fab-dock-bottom",
        }
    }

    /// The entity symbol persisted for this dock (the `tonk:fab/dock` value).
    pub fn symbol(self) -> &'static str {
        match self {
            Dock::TopLeft => "tonk:top-left",
            Dock::BottomLeft => "tonk:bottom-left",
        }
    }

    /// Parse a persisted dock symbol back to a `Dock`. Unknown / absent → `None`
    /// (the caller falls back to the default dock).
    pub fn from_symbol(s: &str) -> Option<Dock> {
        match s {
            "tonk:top-left" => Some(Dock::TopLeft),
            "tonk:bottom-left" => Some(Dock::BottomLeft),
            _ => None,
        }
    }
}

/// Pick the dock nearest a drop. Both docks sit on the left edge, so only the
/// vertical position decides: a FAB whose center `center_y` is in the top half
/// of a viewport of height `vh` docks top-left, otherwise bottom-left. The
/// exact midline falls to the bottom dock.
pub fn nearest_dock(center_y: f64, vh: f64) -> Dock {
    if center_y < vh / 2.0 {
        Dock::TopLeft
    } else {
        Dock::BottomLeft
    }
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
                    "kind": "transient",
                    "concept": {
                        "description": "Persisted FAB dock (profile-meta claim).",
                        "with": {
                            "dock": {
                                "the": "xyz.tonk.fab/dock",
                                "cardinality": "one",
                                "as": "entity"
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

#[cfg(test)]
mod dock {
    use super::*;

    #[test]
    fn top_half_docks_top_left() {
        assert_eq!(nearest_dock(10.0, 800.0), Dock::TopLeft);
    }

    #[test]
    fn bottom_half_docks_bottom_left() {
        assert_eq!(nearest_dock(700.0, 800.0), Dock::BottomLeft);
    }

    #[test]
    fn the_midline_falls_to_the_bottom_dock() {
        assert_eq!(nearest_dock(400.0, 800.0), Dock::BottomLeft);
    }

    #[test]
    fn each_dock_has_its_css_class() {
        assert_eq!(Dock::TopLeft.css_class(), "fab-dock-top");
        assert_eq!(Dock::BottomLeft.css_class(), "fab-dock-bottom");
    }

    #[test]
    fn a_dock_round_trips_through_its_symbol() {
        for dock in [Dock::TopLeft, Dock::BottomLeft] {
            assert_eq!(Dock::from_symbol(dock.symbol()), Some(dock));
        }
    }

    #[test]
    fn an_unknown_symbol_has_no_dock() {
        assert_eq!(Dock::from_symbol("tonk:top-right"), None);
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
        // verify predicate shape matches claim.rs serde derive
        assert_eq!(app["predicate"]["kind"], "transient");
        assert_eq!(
            app["predicate"]["concept"]["with"]["dock"]["the"],
            "xyz.tonk.fab/dock"
        );
        assert_eq!(app["parameters"]["this"], "state:fab");
    }
}
