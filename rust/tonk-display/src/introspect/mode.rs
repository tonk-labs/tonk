//! The arm/observe/latch state machine.
//!
//! Pure logic, deliberately free of any DOM type so it runs under a
//! plain `cargo test`. The overlay feeds it pointer, key and click
//! facts and asks it two questions: which display to outline
//! ([`Machine::highlighted`]) and which one to fully observe
//! ([`Machine::observed`]).
//!
//! The interaction it encodes:
//!
//! - Hold Alt and move over a display: it outlines immediately.
//! - Keep resting there past [`DWELL_MS`]: observation switches on.
//! - Move to another display: the dwell restarts on the new one.
//! - Move off every display, or let Alt go: observation stops.
//! - Alt-click a display: observation latches and survives Alt going
//!   up. Alt-click the latched display again to let it go.
//!
//! Latched beats hovering on purpose. Once a display is pinned the
//! point is to move the pointer somewhere else — over the panel, over
//! a button to press — without the observation evaporating.

/// How long the pointer has to rest on one display, with Alt held,
/// before observation switches on. Short enough not to feel like a
/// wait, long enough that sweeping the pointer across a dense list
/// does not strobe every card on the way past.
pub const DWELL_MS: f64 = 300.0;

/// How long tracking survives the pointer leaving every display.
///
/// Without this, anything worth walking over to — the pin, the panel —
/// is unreachable, because the page between here and there is not a
/// display and reaching it reads as giving up. The grace is the width
/// of that gap in time rather than pixels.
pub const LEAVE_MS: f64 = 400.0;

/// Which display an observation is attached to — an index into the
/// overlay's table of `<tonk-display>` hosts it has seen this frame.
pub type TargetId = u32;

/// What the machine is doing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Not tracking anything.
    Idle,
    /// Alt is down over a display and the dwell timer is running.
    /// Outlined, not yet observed.
    Arming(TargetId),
    /// Observing because Alt is still held over this display.
    Observing(TargetId),
    /// Observing because the user pinned this display. Survives Alt
    /// going up and the pointer moving away.
    Latched(TargetId),
    /// The pointer left every display, but not long enough ago to
    /// mean it. Still painted; still the same target if the pointer
    /// comes back. The flag carries whether observation had started,
    /// so coming back does not restart the dwell.
    Leaving(TargetId, bool),
}

/// A fact the overlay hands the machine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Input {
    /// The pointer moved. `alt` is the live modifier state read off
    /// the event itself rather than remembered from a keydown — a
    /// frame that never had focus sees no keydown at all, but every
    /// mouse event it does get carries `altKey`.
    Pointer {
        /// Whether Alt was held as the event fired.
        alt: bool,
        /// The display under the pointer, if any.
        over: Option<TargetId>,
        /// `performance.now()` at the event.
        at: f64,
    },
    /// Time passed with no pointer event. Drives the dwell timer for
    /// a pointer resting perfectly still, which fires no `mousemove`.
    Tick {
        /// `performance.now()` at the tick.
        at: f64,
    },
    /// Alt came up, in a frame that actually received the `keyup`.
    AltReleased,
    /// Alt-click landed on a display: latch it, or release it if it
    /// is the one already latched.
    Toggle {
        /// The display that was alt-clicked.
        over: TargetId,
    },
    /// Everything off — Escape, focus loss, the overlay detaching.
    Clear,
}

/// The state machine.
#[derive(Debug, Clone)]
pub struct Machine {
    phase: Phase,
    /// When the current [`Phase::Arming`] began.
    armed_at: f64,
    /// When the current [`Phase::Leaving`] began.
    left_at: f64,
    /// Dwell threshold, injectable so tests need not sleep.
    dwell: f64,
    /// Leave grace, injectable for the same reason.
    leave: f64,
}

impl Default for Machine {
    fn default() -> Self {
        Self::with_timing(DWELL_MS, LEAVE_MS)
    }
}

impl Machine {
    /// A machine with custom thresholds, in milliseconds.
    pub fn with_timing(dwell: f64, leave: f64) -> Self {
        Self {
            phase: Phase::Idle,
            armed_at: 0.0,
            left_at: 0.0,
            dwell,
            leave,
        }
    }

    /// A machine with a custom dwell and the default leave grace.
    pub fn with_dwell(dwell: f64) -> Self {
        Self::with_timing(dwell, LEAVE_MS)
    }

    /// The current phase.
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// The display to outline: anything the machine is tracking,
    /// dwelling included.
    pub fn highlighted(&self) -> Option<TargetId> {
        match self.phase {
            Phase::Idle => None,
            Phase::Arming(target)
            | Phase::Observing(target)
            | Phase::Latched(target)
            | Phase::Leaving(target, _) => Some(target),
        }
    }

    /// The display to observe in full — slots painted, panels open.
    /// `None` while merely dwelling.
    pub fn observed(&self) -> Option<TargetId> {
        match self.phase {
            Phase::Observing(target) | Phase::Latched(target) | Phase::Leaving(target, true) => {
                Some(target)
            }
            Phase::Idle | Phase::Arming(_) | Phase::Leaving(_, false) => None,
        }
    }

    /// Whether the machine is tracking a display right now — which is
    /// the only window in which it claims a click. Outside it, an
    /// alt-click is the page's.
    pub fn is_tracking(&self) -> bool {
        self.highlighted().is_some()
    }

    /// Whether the current observation is pinned by an alt-click.
    pub fn is_latched(&self) -> bool {
        matches!(self.phase, Phase::Latched(_))
    }

    /// Feed the machine one fact.
    pub fn apply(&mut self, input: Input) {
        match input {
            Input::Clear => self.phase = Phase::Idle,
            Input::Toggle { over } => {
                self.phase = match self.phase {
                    Phase::Latched(current) if current == over => Phase::Idle,
                    _ => Phase::Latched(over),
                };
            }
            Input::AltReleased => {
                if !self.is_latched() {
                    self.phase = Phase::Idle;
                }
            }
            Input::Pointer { alt, over, at } => self.pointer(alt, over, at),
            Input::Tick { at } => self.settle(at),
        }
    }

    fn pointer(&mut self, alt: bool, over: Option<TargetId>, at: f64) {
        // A latched observation is deliberately deaf to hovering: the
        // user pinned it so they could go and point at something else.
        if self.is_latched() {
            return;
        }
        // Alt up is unambiguous — nothing is being inspected.
        if !alt {
            self.phase = Phase::Idle;
            return;
        }
        let Some(over) = over else {
            self.leave(at);
            return;
        };
        match self.phase {
            Phase::Arming(current) | Phase::Observing(current) if current == over => {
                self.settle(at)
            }
            // Back on the display it was leaving, inside the grace:
            // pick up exactly where it was rather than re-dwelling.
            Phase::Leaving(current, observing) if current == over => {
                self.phase = if observing {
                    Phase::Observing(current)
                } else {
                    Phase::Arming(current)
                };
            }
            _ => {
                self.phase = Phase::Arming(over);
                self.armed_at = at;
            }
        }
    }

    /// The pointer is on no display. Start the grace rather than
    /// giving up, so there is time to reach the overlay's own chrome.
    fn leave(&mut self, at: f64) {
        match self.phase {
            Phase::Arming(target) => {
                self.phase = Phase::Leaving(target, false);
                self.left_at = at;
            }
            Phase::Observing(target) => {
                self.phase = Phase::Leaving(target, true);
                self.left_at = at;
            }
            _ => {}
        }
    }

    /// Promote a dwell that has run long enough, and give up on a
    /// leave that has.
    fn settle(&mut self, at: f64) {
        match self.phase {
            Phase::Arming(target) if at - self.armed_at >= self.dwell => {
                self.phase = Phase::Observing(target);
            }
            Phase::Leaving(_, _) if at - self.left_at >= self.leave => {
                self.phase = Phase::Idle;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn over(machine: &mut Machine, target: TargetId, at: f64) {
        machine.apply(Input::Pointer {
            alt: true,
            over: Some(target),
            at,
        });
    }

    #[test]
    fn it_outlines_before_it_observes() {
        let mut machine = Machine::with_dwell(100.0);
        over(&mut machine, 7, 0.0);
        assert_eq!(machine.highlighted(), Some(7));
        assert_eq!(machine.observed(), None);
    }

    #[test]
    fn it_observes_once_the_pointer_has_rested_long_enough() {
        let mut machine = Machine::with_dwell(100.0);
        over(&mut machine, 7, 0.0);
        machine.apply(Input::Tick { at: 100.0 });
        assert_eq!(machine.observed(), Some(7));
    }

    #[test]
    fn it_restarts_the_dwell_on_a_different_display() {
        let mut machine = Machine::with_dwell(100.0);
        over(&mut machine, 7, 0.0);
        over(&mut machine, 8, 90.0);
        machine.apply(Input::Tick { at: 150.0 });
        assert_eq!(machine.observed(), None, "8 has only been hovered 60ms");
        machine.apply(Input::Tick { at: 190.0 });
        assert_eq!(machine.observed(), Some(8));
    }

    #[test]
    fn leaving_every_display_keeps_painting_for_the_grace() {
        let mut machine = Machine::with_timing(100.0, 400.0);
        over(&mut machine, 7, 0.0);
        machine.apply(Input::Tick { at: 100.0 });
        machine.apply(Input::Pointer {
            alt: true,
            over: None,
            at: 120.0,
        });
        assert_eq!(
            machine.observed(),
            Some(7),
            "the pin and the panel are off the display; reaching them cannot mean giving up"
        );
        machine.apply(Input::Tick { at: 400.0 });
        assert_eq!(machine.observed(), Some(7), "still inside the grace");
        machine.apply(Input::Tick { at: 600.0 });
        assert_eq!(machine.phase(), Phase::Idle, "grace expired");
    }

    #[test]
    fn coming_back_inside_the_grace_does_not_restart_the_dwell() {
        let mut machine = Machine::with_timing(100.0, 400.0);
        over(&mut machine, 7, 0.0);
        machine.apply(Input::Tick { at: 100.0 });
        machine.apply(Input::Pointer {
            alt: true,
            over: None,
            at: 120.0,
        });
        over(&mut machine, 7, 200.0);
        assert_eq!(
            machine.phase(),
            Phase::Observing(7),
            "it was observing when it left, so it is observing when it returns"
        );
    }

    #[test]
    fn a_dwell_interrupted_by_a_gap_resumes_as_a_dwell() {
        let mut machine = Machine::with_timing(100.0, 400.0);
        over(&mut machine, 7, 0.0);
        machine.apply(Input::Pointer {
            alt: true,
            over: None,
            at: 20.0,
        });
        assert_eq!(machine.observed(), None, "it had not started observing");
        over(&mut machine, 7, 40.0);
        assert_eq!(machine.phase(), Phase::Arming(7));
    }

    #[test]
    fn crossing_onto_another_display_inside_the_grace_starts_its_dwell() {
        let mut machine = Machine::with_timing(100.0, 400.0);
        over(&mut machine, 7, 0.0);
        machine.apply(Input::Tick { at: 100.0 });
        machine.apply(Input::Pointer {
            alt: true,
            over: None,
            at: 120.0,
        });
        over(&mut machine, 8, 140.0);
        assert_eq!(machine.phase(), Phase::Arming(8));
    }

    #[test]
    fn releasing_alt_during_the_grace_gives_up_at_once() {
        let mut machine = Machine::with_timing(100.0, 400.0);
        over(&mut machine, 7, 0.0);
        machine.apply(Input::Tick { at: 100.0 });
        machine.apply(Input::Pointer {
            alt: true,
            over: None,
            at: 120.0,
        });
        machine.apply(Input::AltReleased);
        assert_eq!(machine.phase(), Phase::Idle);
    }

    #[test]
    fn the_machine_claims_a_click_only_while_it_is_tracking() {
        let mut machine = Machine::with_dwell(100.0);
        assert!(!machine.is_tracking(), "an idle overlay owns no gesture");
        over(&mut machine, 7, 0.0);
        assert!(machine.is_tracking());
    }

    #[test]
    fn it_stops_observing_when_alt_comes_up_under_the_pointer() {
        let mut machine = Machine::with_dwell(100.0);
        over(&mut machine, 7, 0.0);
        machine.apply(Input::Tick { at: 100.0 });
        machine.apply(Input::Pointer {
            alt: false,
            over: Some(7),
            at: 120.0,
        });
        assert_eq!(machine.phase(), Phase::Idle);
    }

    #[test]
    fn it_stops_observing_on_an_alt_keyup() {
        let mut machine = Machine::with_dwell(100.0);
        over(&mut machine, 7, 0.0);
        machine.apply(Input::Tick { at: 100.0 });
        machine.apply(Input::AltReleased);
        assert_eq!(machine.phase(), Phase::Idle);
    }

    #[test]
    fn a_latched_observation_survives_alt_coming_up() {
        let mut machine = Machine::default();
        machine.apply(Input::Toggle { over: 7 });
        machine.apply(Input::AltReleased);
        assert_eq!(machine.observed(), Some(7));
    }

    #[test]
    fn a_latched_observation_ignores_hovering_elsewhere() {
        let mut machine = Machine::with_dwell(100.0);
        machine.apply(Input::Toggle { over: 7 });
        over(&mut machine, 8, 0.0);
        machine.apply(Input::Tick { at: 500.0 });
        assert_eq!(machine.observed(), Some(7));
    }

    #[test]
    fn alt_clicking_the_latched_display_releases_it() {
        let mut machine = Machine::default();
        machine.apply(Input::Toggle { over: 7 });
        machine.apply(Input::Toggle { over: 7 });
        assert_eq!(machine.observed(), None);
    }

    #[test]
    fn alt_clicking_another_display_moves_the_latch() {
        let mut machine = Machine::default();
        machine.apply(Input::Toggle { over: 7 });
        machine.apply(Input::Toggle { over: 8 });
        assert_eq!(machine.observed(), Some(8));
    }

    #[test]
    fn clear_releases_even_a_latch() {
        let mut machine = Machine::default();
        machine.apply(Input::Toggle { over: 7 });
        machine.apply(Input::Clear);
        assert_eq!(machine.observed(), None);
    }
}
