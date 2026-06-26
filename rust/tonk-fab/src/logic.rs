//! Pure geometry logic for the FAB element.
//!
//! No DOM imports — compiles and tests on the native target.

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
    Resize { w: f64, h: f64, state: FabState },
    Drop { x: f64, y: f64, state: FabState },
}

pub fn geometry_box(intent: &FabIntent, vw: f64, vh: f64) -> FabBox {
    match intent {
        FabIntent::DragStart => FabBox { left: 0.0, top: 0.0, width: vw, height: vh },
        FabIntent::Resize { w, h, state } => {
            FabBox { left: state.x, top: state.y, width: *w, height: *h }
        }
        FabIntent::Drop { x, y, state } => {
            FabBox { left: *x, top: *y, width: state.w, height: state.h }
        }
    }
}

#[cfg(test)]
mod geometry {
    use super::*;

    #[test]
    fn dragstart_covers_full_viewport() {
        let b = geometry_box(&FabIntent::DragStart, 1000.0, 800.0);
        assert_eq!(b, FabBox { left: 0.0, top: 0.0, width: 1000.0, height: 800.0 });
    }

    #[test]
    fn resize_keeps_position_changes_size() {
        let state = FabState { x: 100.0, y: 50.0, w: 320.0, h: 64.0, dragging: false };
        let b = geometry_box(&FabIntent::Resize { w: 320.0, h: 64.0, state }, 1000.0, 800.0);
        assert_eq!(b, FabBox { left: 100.0, top: 50.0, width: 320.0, height: 64.0 });
    }

    #[test]
    fn drop_moves_to_point_keeps_size() {
        let state = FabState { x: 100.0, y: 50.0, w: 320.0, h: 64.0, dragging: true };
        let b = geometry_box(&FabIntent::Drop { x: 400.0, y: 600.0, state }, 1000.0, 800.0);
        assert_eq!(b, FabBox { left: 400.0, top: 600.0, width: 320.0, height: 64.0 });
    }
}
