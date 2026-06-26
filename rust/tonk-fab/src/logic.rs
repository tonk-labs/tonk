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
    DragMove { x: f64, y: f64, state: FabState },
    Resize { w: f64, h: f64, state: FabState },
    Drop { x: f64, y: f64, state: FabState },
}

pub fn geometry_box(intent: &FabIntent, vw: f64, vh: f64) -> FabBox {
    match intent {
        FabIntent::DragStart => FabBox {
            left: 0.0,
            top: 0.0,
            width: vw,
            height: vh,
        },
        FabIntent::DragMove { x, y, state } => FabBox {
            left: *x,
            top: *y,
            width: state.w,
            height: state.h,
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
    }
}

/// Returns `true` when the FAB's top-left `y` is in the top half of the viewport
/// (height `vh`), meaning the submenu should open downward.
pub fn submenu_opens_down(y: f64, vh: f64) -> bool {
    y < vh / 2.0
}

pub const COLLAPSE_MS: u64 = 1000;

pub struct CollapseMachine {
    expanded: bool,
    since_leave: Option<u64>,
}

impl CollapseMachine {
    pub fn new() -> Self {
        Self {
            expanded: false,
            since_leave: None,
        }
    }

    pub fn expanded(&self) -> bool {
        self.expanded
    }

    pub fn on_enter(&mut self) {
        self.expanded = true;
        self.since_leave = None;
    }

    pub fn on_leave(&mut self) {
        self.since_leave = Some(0);
    }

    pub fn tick(&mut self, elapsed_ms: u64) {
        if let Some(acc) = self.since_leave.as_mut() {
            *acc += elapsed_ms;
            if *acc >= COLLAPSE_MS {
                self.expanded = false;
                self.since_leave = None;
            }
        }
    }
}

impl Default for CollapseMachine {
    fn default() -> Self {
        Self::new()
    }
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
/// - `ConceptDescriptor` → `#[serde(tag="kind", content="concept")]` → `"kind":"transient"`, `"concept":{...}`
/// - `PredicateApplication` → `{ predicate, parameters }`
pub fn position_claim_json(x: u32, y: u32) -> Value {
    json!({
        "claims": [{
            "op": "assert",
            "application": {
                "predicate": {
                    "kind": "transient",
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
mod collapse {
    use super::*;

    #[test]
    fn enter_expands_immediately() {
        let mut m = CollapseMachine::new();
        m.on_enter();
        assert!(m.expanded());
    }

    #[test]
    fn leave_then_timeout_collapses() {
        let mut m = CollapseMachine::new();
        m.on_enter();
        m.on_leave();
        assert!(m.expanded(), "still expanded right after leave");
        m.tick(1000);
        assert!(!m.expanded(), "collapsed after 1s");
    }

    #[test]
    fn reenter_cancels_collapse() {
        let mut m = CollapseMachine::new();
        m.on_enter();
        m.on_leave();
        m.tick(500);
        m.on_enter();
        m.tick(1000);
        assert!(m.expanded());
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
    fn dragmove_moves_to_point_keeps_size() {
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
                left: 200.0,
                top: 300.0,
                width: 320.0,
                height: 64.0
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
        // verify predicate shape matches claim.rs serde derive
        assert_eq!(app["predicate"]["kind"], "transient");
        assert!(app["predicate"]["concept"]["with"].is_object());
        assert_eq!(app["parameters"]["this"], "state:fab");
    }
}
