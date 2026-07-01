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

/// Returns `true` when the FAB's top-left `y` is in the top half of the viewport
/// (height `vh`), meaning the submenu should open downward.
pub fn submenu_opens_down(y: f64, vh: f64) -> bool {
    y < vh / 2.0
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

/// Clamp the drop position so the circle stays fully on-screen.
///
/// `x`, `y` are the desired top-left of the circle (viewport coords).
/// `vw`, `vh` are the viewport width/height.
/// `w`, `h` are the circle's width/height.
/// Returns the clamped `(x, y)`.
pub fn clamp_position(x: f64, y: f64, vw: f64, vh: f64, w: f64, h: f64) -> (f64, f64) {
    (
        x.clamp(0.0, (vw - w).max(0.0)),
        y.clamp(0.0, (vh - h).max(0.0)),
    )
}

/// Build a `TransactRequest` JSON body for `window.tonk.transact(...)`.
///
/// Asserts the `tonk:fab/position` concept on `state:fab` with the given
/// x/y pixel values. The JSON shape matches the `TransactRequest` serde
/// derive in `tonk-core/src/claim.rs`:
///
/// - `Claim` → `#[serde(tag="op", content="application")]` → `"op":"assert"`, `"application":{...}`
/// - `ConceptDescriptor` → `#[serde(tag="kind", content="concept")]` → `"kind":"durable"`, `"concept":{...}`
/// - `PredicateApplication` → `{ predicate, parameters }`
///
/// `durable` (not `transient`): the position must survive the reactor's commit
/// so a later load can read it back. A transient concept is stripped before the
/// durable commit, so it would never persist across a reload.
pub fn position_claim_json(x: u32, y: u32) -> Value {
    json!({
        "claims": [{
            "op": "assert",
            "application": {
                "predicate": {
                    "kind": "durable",
                    "concept": {
                        "description": "Persisted FAB position (profile-meta claim).",
                        "with": {
                            "x": {
                                "the": "xyz.tonk.fab/x",
                                "cardinality": "one",
                                "as": "UnsignedInteger"
                            },
                            "y": {
                                "the": "xyz.tonk.fab/y",
                                "cardinality": "one",
                                "as": "UnsignedInteger"
                            }
                        }
                    }
                },
                "parameters": {
                    "this": "state:fab",
                    "x": x,
                    "y": y
                }
            }
        }]
    })
}

/// Build a `TransactRequest` JSON body asserting the FAB's EXPANSION state on
/// `state:fab`: whether the bar is collapsed to the circle, and which
/// disclosure sections (`account` / `share`) are shown. Written back on a
/// telescope/disclosure change so the next load restores the bar's shape.
///
/// Independent of the position claim (each field is cardinality-one), so
/// asserting expansion here does not touch the persisted `x`/`y` — a drop
/// persists position, a toggle persists expansion, neither disturbs the other.
///
/// `durable` (not `transient`), like the position claim: the expansion must
/// survive the reactor's commit to be read back on the next load.
pub fn expansion_claim_json(collapsed: bool, account: bool, share: bool) -> Value {
    json!({
        "claims": [{
            "op": "assert",
            "application": {
                "predicate": {
                    "kind": "durable",
                    "concept": {
                        "description": "Persisted FAB expansion (profile-meta claim).",
                        "with": {
                            "collapsed": {
                                "the": "xyz.tonk.fab/collapsed",
                                "cardinality": "one",
                                "as": "Boolean"
                            },
                            "account": {
                                "the": "xyz.tonk.fab/account",
                                "cardinality": "one",
                                "as": "Boolean"
                            },
                            "share": {
                                "the": "xyz.tonk.fab/share",
                                "cardinality": "one",
                                "as": "Boolean"
                            }
                        }
                    }
                },
                "parameters": {
                    "this": "state:fab",
                    "collapsed": collapsed,
                    "account": account,
                    "share": share
                }
            }
        }]
    })
}

#[cfg(test)]
mod submenu {
    use super::*;
    #[test]
    fn opens_down_in_top_half() {
        assert!(submenu_opens_down(10.0, 800.0));
        assert!(!submenu_opens_down(700.0, 800.0));
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
    fn clamp_keeps_circle_on_screen() {
        assert_eq!(
            clamp_position(-30.0, 5.0, 1000.0, 800.0, 64.0, 64.0),
            (0.0, 5.0)
        );
        assert_eq!(
            clamp_position(2000.0, 5.0, 1000.0, 800.0, 64.0, 64.0),
            (936.0, 5.0)
        );
    }

    #[test]
    fn claim_json_targets_fab_position() {
        let v = position_claim_json(120, 240);
        assert_eq!(v["claims"][0]["op"], "assert");
        let app = &v["claims"][0]["application"];
        assert_eq!(app["parameters"]["x"], 120);
        assert_eq!(app["parameters"]["y"], 240);
        // Durable so the position survives the commit and restores on reload.
        assert_eq!(app["predicate"]["kind"], "durable");
        assert!(app["predicate"]["concept"]["with"].is_object());
        assert_eq!(app["parameters"]["this"], "state:fab");
    }

    #[test]
    fn expansion_claim_json_targets_fab_state() {
        let v = expansion_claim_json(true, false, true);
        assert_eq!(v["claims"][0]["op"], "assert");
        let app = &v["claims"][0]["application"];
        assert_eq!(app["parameters"]["collapsed"], true);
        assert_eq!(app["parameters"]["account"], false);
        assert_eq!(app["parameters"]["share"], true);
        assert_eq!(app["predicate"]["kind"], "durable");
        assert_eq!(app["parameters"]["this"], "state:fab");
    }
}
