//! Holding and rewinding a display's entity frames.
//!
//! A live view is hard to read precisely because it is live: the thing
//! you were looking at is replaced by the next frame before you have
//! finished looking at it. The recorder gives the stream a hold and a
//! position — stop applying what arrives, and step back through what
//! already did.
//!
//! What makes rewinding cheap is that a frame is the whole state, not a
//! delta. `<tonk-display>` folds each arrival into one conclusion per
//! subject and hands that to the renderer, which reconciles the DOM
//! against it. So replaying frame *i* is just handing the renderer
//! frame *i* again: no inverse operations, no snapshots to reconstruct.
//! The renderer patches in place either way, and going backwards costs
//! exactly what going forwards did.
//!
//! Recording runs only while a display is being observed. That keeps
//! it free the rest of the time, at the price of not being able to
//! rewind past the moment the hood opened — the recorder is seeded
//! with the frame already on screen, so position 0 is always "what I
//! was looking at when I started".
//!
//! Pure: `T` is the frame type, so this tests natively against
//! anything.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

/// How many frames are kept. A frame is a whole folded state, and a
/// directory frame carries every instance, so this is a memory budget
/// as much as a history depth.
pub const DEPTH: usize = 40;

/// Where the display is in its history, for the panel to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Timeline {
    /// How many frames are recorded.
    pub frames: usize,
    /// Which one is on screen, or `None` when the display is live and
    /// showing whatever arrives.
    pub position: Option<usize>,
    /// How many frames arrived and were recorded but not applied,
    /// because the display is held. Zero when live.
    pub held: usize,
}

impl Timeline {
    /// Whether arrivals are being held rather than applied.
    pub fn is_held(self) -> bool {
        self.position.is_some()
    }

    /// The index on screen: the pinned one, or the newest.
    pub fn current(self) -> Option<usize> {
        match self.position {
            Some(position) => Some(position),
            None => self.frames.checked_sub(1),
        }
    }

    /// How the panel spells it: `live · 6 frames` / `frame 3 of 6 · 2 held`.
    pub fn label(self) -> String {
        match (self.position, self.frames) {
            (None, 0) => "live · nothing recorded yet".to_owned(),
            (None, frames) => format!("live · {frames} frame(s)"),
            (Some(position), frames) => {
                let mut out = format!("held · frame {} of {frames}", position + 1);
                if self.held > 0 {
                    out.push_str(&format!(" · {} waiting", self.held));
                }
                out
            }
        }
    }
}

/// What the display should do with a frame that just arrived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrival {
    /// Render it, as normal.
    Apply,
    /// Recorded, but do not render: the display is held.
    Hold,
}

/// A display's frame history and its position in it.
#[derive(Debug, Clone)]
pub struct Recorder<T> {
    frames: VecDeque<T>,
    position: Option<usize>,
    held: usize,
    depth: usize,
    /// True while a seek is re-applying a recorded frame, so the
    /// replay does not record itself.
    replaying: bool,
}

impl<T: Clone> Default for Recorder<T> {
    fn default() -> Self {
        Self::with_depth(DEPTH)
    }
}

impl<T: Clone> Recorder<T> {
    /// An empty recorder with a custom depth, for tests.
    pub fn with_depth(depth: usize) -> Self {
        Self {
            frames: VecDeque::new(),
            position: None,
            held: 0,
            depth: depth.max(1),
            replaying: false,
        }
    }

    /// Seed with the frame already on screen, so position 0 is what
    /// the observer was looking at when they opened the hood.
    pub fn seed(&mut self, frame: T) {
        if self.frames.is_empty() {
            self.frames.push_back(frame);
        }
    }

    /// Record an arriving frame and say whether to render it.
    ///
    /// A frame that arrives while held is still recorded — that is the
    /// point of holding, to be able to step forward into what you
    /// missed — but it does not move the position.
    pub fn arrived(&mut self, frame: T) -> Arrival {
        if self.replaying {
            return Arrival::Apply;
        }
        self.frames.push_back(frame);
        while self.frames.len() > self.depth {
            self.frames.pop_front();
            // The pinned frame slid out from under the window; hold
            // the oldest still there rather than silently jumping.
            if let Some(position) = self.position.as_mut() {
                *position = position.saturating_sub(1);
            }
        }
        if self.position.is_some() {
            self.held += 1;
            Arrival::Hold
        } else {
            Arrival::Apply
        }
    }

    /// Pin the display to `position`, or release it to live.
    ///
    /// Returns the frame to render, if it moved. Releasing to live
    /// renders the newest frame, which is how the display catches up
    /// with everything that arrived while it was held.
    pub fn seek(&mut self, position: Option<usize>) -> Option<T> {
        let target = match position {
            Some(position) => Some(position.min(self.frames.len().saturating_sub(1))),
            None => None,
        };
        self.position = target;
        if target.is_none() {
            self.held = 0;
        }
        let index = target.or_else(|| self.frames.len().checked_sub(1))?;
        self.frames.get(index).cloned()
    }

    /// Hold the display where it is.
    pub fn hold(&mut self) -> Option<T> {
        if self.position.is_some() {
            return None;
        }
        self.seek(Some(self.frames.len().saturating_sub(1)))
    }

    /// Step by `delta` frames, holding if live. Clamped to the ends.
    pub fn step(&mut self, delta: i32) -> Option<T> {
        let current = self.timeline().current()? as i64;
        let next = (current + delta as i64).clamp(0, self.frames.len() as i64 - 1);
        self.seek(Some(next as usize))
    }

    /// What the panel draws.
    pub fn timeline(&self) -> Timeline {
        Timeline {
            frames: self.frames.len(),
            position: self.position,
            held: self.held,
        }
    }

    /// Run `apply` with replay suppression on, so re-rendering a
    /// recorded frame does not record it again.
    pub fn replaying(&mut self, on: bool) {
        self.replaying = on;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recorder() -> Recorder<&'static str> {
        Recorder::with_depth(4)
    }

    #[test]
    fn a_live_recorder_applies_what_arrives() {
        let mut recorder = recorder();
        assert_eq!(recorder.arrived("a"), Arrival::Apply);
        assert_eq!(recorder.timeline().frames, 1);
        assert!(!recorder.timeline().is_held());
    }

    #[test]
    fn holding_pins_the_newest_frame() {
        let mut recorder = recorder();
        recorder.arrived("a");
        recorder.arrived("b");
        assert_eq!(recorder.hold(), Some("b"));
        assert_eq!(recorder.timeline().position, Some(1));
    }

    #[test]
    fn a_frame_arriving_while_held_is_recorded_but_not_applied() {
        let mut recorder = recorder();
        recorder.arrived("a");
        recorder.hold();
        assert_eq!(recorder.arrived("b"), Arrival::Hold);
        let timeline = recorder.timeline();
        assert_eq!(timeline.frames, 2, "it is still recorded");
        assert_eq!(timeline.position, Some(0), "the view did not move");
        assert_eq!(timeline.held, 1);
    }

    #[test]
    fn going_live_catches_up_to_the_newest_frame() {
        let mut recorder = recorder();
        recorder.arrived("a");
        recorder.hold();
        recorder.arrived("b");
        recorder.arrived("c");
        assert_eq!(recorder.seek(None), Some("c"));
        assert_eq!(recorder.timeline().held, 0);
        assert!(!recorder.timeline().is_held());
    }

    #[test]
    fn stepping_back_from_live_holds_where_it_lands() {
        let mut recorder = recorder();
        recorder.arrived("a");
        recorder.arrived("b");
        assert_eq!(recorder.step(-1), Some("a"));
        assert!(recorder.timeline().is_held());
    }

    #[test]
    fn stepping_is_clamped_to_both_ends() {
        let mut recorder = recorder();
        recorder.arrived("a");
        recorder.arrived("b");
        assert_eq!(recorder.step(-10), Some("a"));
        assert_eq!(recorder.step(10), Some("b"));
    }

    #[test]
    fn a_seek_past_the_end_lands_on_the_last_frame() {
        let mut recorder = recorder();
        recorder.arrived("a");
        assert_eq!(recorder.seek(Some(99)), Some("a"));
        assert_eq!(recorder.timeline().position, Some(0));
    }

    #[test]
    fn the_window_drops_the_oldest_and_keeps_the_pin_on_its_frame() {
        let mut recorder = recorder();
        for frame in ["a", "b", "c", "d"] {
            recorder.arrived(frame);
        }
        recorder.seek(Some(1));
        assert_eq!(recorder.timeline().position, Some(1), "holding b");
        recorder.arrived("e");
        // `a` fell out of the window, so `b` is now index 0.
        assert_eq!(recorder.timeline().frames, 4);
        assert_eq!(recorder.timeline().position, Some(0));
        assert_eq!(recorder.seek(Some(0)), Some("b"), "still the same frame");
    }

    #[test]
    fn seeding_only_takes_the_first_frame_offered() {
        let mut recorder = recorder();
        recorder.seed("on screen");
        recorder.seed("ignored");
        assert_eq!(recorder.timeline().frames, 1);
        assert_eq!(recorder.seek(Some(0)), Some("on screen"));
    }

    #[test]
    fn a_replayed_frame_is_not_recorded_again() {
        let mut recorder = recorder();
        recorder.arrived("a");
        recorder.replaying(true);
        assert_eq!(recorder.arrived("a"), Arrival::Apply);
        recorder.replaying(false);
        assert_eq!(recorder.timeline().frames, 1);
    }

    #[test]
    fn it_spells_its_position_for_the_panel() {
        let mut recorder = recorder();
        assert_eq!(recorder.timeline().label(), "live · nothing recorded yet");
        recorder.arrived("a");
        recorder.arrived("b");
        assert_eq!(recorder.timeline().label(), "live · 2 frame(s)");
        recorder.seek(Some(0));
        recorder.arrived("c");
        assert_eq!(
            recorder.timeline().label(),
            "held · frame 1 of 3 · 1 waiting"
        );
    }
}
