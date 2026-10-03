//! The pick/observe/pin state machine.
//!
//! Pure logic, deliberately free of any DOM type so it runs under a
//! plain `cargo test`. The overlay feeds it pointer and click facts and
//! asks it three questions: whether it is picking
//! ([`Machine::is_picking`]), which display to outline
//! ([`Machine::highlighted`]) and which one to fully observe
//! ([`Machine::observed`]).
//!
//! The interaction it encodes:
//!
//! - The `inspect` command starts picking. Nothing happens before that:
//!   the overlay takes no gesture from a page nobody is inspecting.
//! - While picking, the display under the pointer outlines at once.
//! - Rest there past [`DWELL_MS`] and observation switches on.
//! - Move to another display and the dwell restarts there.
//! - Move off every display and, after [`LEAVE_MS`], the outline goes —
//!   but picking does not, so the next display you reach outlines.
//! - Choose a display and observation pins to it. Choosing ends the
//!   pick, the way choosing an element ends a browser's element picker.
//! - Choose the pinned display again to release it, or clear everything.
//!
//! Pinned beats hovering on purpose. Once a display is pinned the point
//! is to move the pointer somewhere else — over the inspector, over a
//! button to press — without the observation evaporating.
//!
//! Picking replaced holding Alt. A held modifier is a mode you have to
//! keep pressing, read off a key event a frame that never had focus
//! does not receive; a command is a mode you enter once, from anywhere
//! the palette is.

/// How long the pointer has to rest on one display while picking before
/// observation switches on. Short enough not to feel like a wait, long
/// enough that sweeping the pointer across a dense list does not strobe
/// every card on the way past.
pub const DWELL_MS: f64 = 300.0;

/// How long tracking survives the pointer leaving every display.
///
/// Without this, anything worth walking over to — the pin, the
/// inspector — is unreachable, because the page between here and there
/// is not a display and reaching it reads as giving up. The grace is
/// the width of that gap in time rather than pixels.
pub const LEAVE_MS: f64 = 400.0;

/// Which display an observation is attached to — an index into the
/// overlay's table of `<tonk-display>` hosts it has seen this frame.
pub type TargetId = u32;

/// What the machine is doing with the pointer right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Not tracking anything.
    Idle,
    /// Over a display and the dwell timer is running. Outlined, not yet
    /// observed.
    Arming(TargetId),
    /// Observing the display under the pointer.
    Observing(TargetId),
    /// Observing a chosen display. Survives the pointer moving away.
    Pinned(TargetId),
    /// The pointer left every display, but not long enough ago to mean
    /// it. Still painted; still the same target if the pointer comes
    /// back. The flag carries whether observation had started, so
    /// coming back does not restart the dwell.
    Leaving(TargetId, bool),
}

/// A fact the overlay hands the machine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Input {
    /// Start picking: the `inspect` command arrived.
    Pick,
    /// The pointer moved.
    Pointer {
        /// The display under the pointer, if any.
        over: Option<TargetId>,
        /// `performance.now()` at the event.
        at: f64,
    },
    /// Time passed with no pointer event. Drives the dwell timer for a
    /// pointer resting perfectly still, which fires no `mousemove`.
    Tick {
        /// `performance.now()` at the tick.
        at: f64,
    },
    /// A display was chosen: pin it, or release it if it is the one
    /// already pinned.
    Choose {
        /// The display that was chosen.
        over: TargetId,
    },
    /// Everything off — Escape, the close button, the overlay detaching.
    Clear,
}

/// The state machine.
#[derive(Debug, Clone)]
pub struct Machine {
    phase: Phase,
    /// Whether the overlay is accepting the pointer at all.
    picking: bool,
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
            picking: false,
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

    /// Whether the overlay is picking — every display is a candidate and
    /// the pointer is being followed.
    pub fn is_picking(&self) -> bool {
        self.picking
    }

    /// The display to outline: anything the machine is tracking,
    /// dwelling included.
    pub fn highlighted(&self) -> Option<TargetId> {
        match self.phase {
            Phase::Idle => None,
            Phase::Arming(target)
            | Phase::Observing(target)
            | Phase::Pinned(target)
            | Phase::Leaving(target, _) => Some(target),
        }
    }

    /// The display to observe in full — slots painted, inspector open.
    /// `None` while merely dwelling.
    pub fn observed(&self) -> Option<TargetId> {
        match self.phase {
            Phase::Observing(target) | Phase::Pinned(target) | Phase::Leaving(target, true) => {
                Some(target)
            }
            Phase::Idle | Phase::Arming(_) | Phase::Leaving(_, false) => None,
        }
    }

    /// Whether the current observation is pinned.
    pub fn is_pinned(&self) -> bool {
        matches!(self.phase, Phase::Pinned(_))
    }

    /// Whether there is anything to draw: a pick in progress, or a
    /// display being tracked.
    pub fn is_active(&self) -> bool {
        self.picking || self.highlighted().is_some()
    }

    /// Feed the machine one fact.
    pub fn apply(&mut self, input: Input) {
        match input {
            Input::Clear => {
                self.phase = Phase::Idle;
                self.picking = false;
            }
            Input::Pick => self.picking = true,
            Input::Choose { over } => {
                self.phase = match self.phase {
                    Phase::Pinned(current) if current == over => Phase::Idle,
                    _ => Phase::Pinned(over),
                };
                // Choosing ends the pick, as choosing an element ends a
                // browser's element picker. Releasing does too: the
                // reader is done with this one.
                self.picking = false;
            }
            Input::Pointer { over, at } => self.pointer(over, at),
            Input::Tick { at } => self.settle(at),
        }
    }

    fn pointer(&mut self, over: Option<TargetId>, at: f64) {
        // A pinned observation is deliberately deaf to hovering: it was
        // chosen so the reader could go and point at something else.
        if self.is_pinned() || !self.picking {
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

    /// The pointer is on no display. Start the grace rather than giving
    /// up, so there is time to reach the overlay's own chrome.
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

    /// Promote a dwell that has run long enough, and give up on a leave
    /// that has. Giving up drops the outline, not the pick.
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

    fn picking(dwell: f64, leave: f64) -> Machine {
        let mut machine = Machine::with_timing(dwell, leave);
        machine.apply(Input::Pick);
        machine
    }

    fn over(machine: &mut Machine, target: TargetId, at: f64) {
        machine.apply(Input::Pointer {
            over: Some(target),
            at,
        });
    }

    fn off(machine: &mut Machine, at: f64) {
        machine.apply(Input::Pointer { over: None, at });
    }

    #[test]
    fn nothing_happens_before_inspect_is_asked_for() {
        let mut machine = Machine::with_dwell(100.0);
        over(&mut machine, 7, 0.0);
        machine.apply(Input::Tick { at: 500.0 });
        assert_eq!(machine.phase(), Phase::Idle, "hovering is not inspecting");
        assert!(!machine.is_active(), "the overlay takes no gesture unasked");
    }

    #[test]
    fn it_outlines_before_it_observes() {
        let mut machine = picking(100.0, 400.0);
        over(&mut machine, 7, 0.0);
        assert_eq!(machine.highlighted(), Some(7));
        assert_eq!(machine.observed(), None);
    }

    #[test]
    fn it_observes_once_the_pointer_has_rested_long_enough() {
        let mut machine = picking(100.0, 400.0);
        over(&mut machine, 7, 0.0);
        machine.apply(Input::Tick { at: 100.0 });
        assert_eq!(machine.observed(), Some(7));
    }

    #[test]
    fn it_restarts_the_dwell_on_a_different_display() {
        let mut machine = picking(100.0, 400.0);
        over(&mut machine, 7, 0.0);
        over(&mut machine, 8, 90.0);
        machine.apply(Input::Tick { at: 150.0 });
        assert_eq!(machine.observed(), None, "8 has only been hovered 60ms");
        machine.apply(Input::Tick { at: 190.0 });
        assert_eq!(machine.observed(), Some(8));
    }

    #[test]
    fn leaving_every_display_keeps_painting_for_the_grace() {
        let mut machine = picking(100.0, 400.0);
        over(&mut machine, 7, 0.0);
        machine.apply(Input::Tick { at: 100.0 });
        off(&mut machine, 120.0);
        assert_eq!(
            machine.observed(),
            Some(7),
            "the inspector is off the display; reaching it cannot mean giving up"
        );
        machine.apply(Input::Tick { at: 400.0 });
        assert_eq!(machine.observed(), Some(7), "still inside the grace");
        machine.apply(Input::Tick { at: 600.0 });
        assert_eq!(machine.phase(), Phase::Idle, "grace expired");
    }

    #[test]
    fn an_expired_grace_drops_the_outline_but_not_the_pick() {
        let mut machine = picking(100.0, 400.0);
        over(&mut machine, 7, 0.0);
        off(&mut machine, 20.0);
        machine.apply(Input::Tick { at: 600.0 });
        assert!(
            machine.is_picking(),
            "the next display reached should outline"
        );
        over(&mut machine, 8, 700.0);
        assert_eq!(machine.highlighted(), Some(8));
    }

    #[test]
    fn coming_back_inside_the_grace_does_not_restart_the_dwell() {
        let mut machine = picking(100.0, 400.0);
        over(&mut machine, 7, 0.0);
        machine.apply(Input::Tick { at: 100.0 });
        off(&mut machine, 120.0);
        over(&mut machine, 7, 200.0);
        assert_eq!(
            machine.phase(),
            Phase::Observing(7),
            "it was observing when it left, so it is observing when it returns"
        );
    }

    #[test]
    fn a_dwell_interrupted_by_a_gap_resumes_as_a_dwell() {
        let mut machine = picking(100.0, 400.0);
        over(&mut machine, 7, 0.0);
        off(&mut machine, 20.0);
        assert_eq!(machine.observed(), None, "it had not started observing");
        over(&mut machine, 7, 40.0);
        assert_eq!(machine.phase(), Phase::Arming(7));
    }

    #[test]
    fn crossing_onto_another_display_inside_the_grace_starts_its_dwell() {
        let mut machine = picking(100.0, 400.0);
        over(&mut machine, 7, 0.0);
        machine.apply(Input::Tick { at: 100.0 });
        off(&mut machine, 120.0);
        over(&mut machine, 8, 140.0);
        assert_eq!(machine.phase(), Phase::Arming(8));
    }

    #[test]
    fn choosing_a_display_pins_it_and_ends_the_pick() {
        let mut machine = picking(100.0, 400.0);
        over(&mut machine, 7, 0.0);
        machine.apply(Input::Choose { over: 7 });
        assert_eq!(machine.observed(), Some(7));
        assert!(machine.is_pinned());
        assert!(
            !machine.is_picking(),
            "as choosing an element ends a browser's element picker"
        );
    }

    #[test]
    fn a_display_can_be_chosen_without_ever_hovering_it() {
        // From the inspector's list of what is on the page.
        let mut machine = picking(100.0, 400.0);
        machine.apply(Input::Choose { over: 3 });
        assert_eq!(machine.observed(), Some(3));
    }

    #[test]
    fn a_pinned_observation_ignores_hovering_elsewhere() {
        let mut machine = picking(100.0, 400.0);
        machine.apply(Input::Choose { over: 7 });
        over(&mut machine, 8, 0.0);
        machine.apply(Input::Tick { at: 500.0 });
        assert_eq!(machine.observed(), Some(7));
    }

    #[test]
    fn choosing_the_pinned_display_releases_it() {
        let mut machine = picking(100.0, 400.0);
        machine.apply(Input::Choose { over: 7 });
        machine.apply(Input::Choose { over: 7 });
        assert_eq!(machine.observed(), None);
        assert!(
            !machine.is_active(),
            "released and not picking: nothing to draw"
        );
    }

    #[test]
    fn choosing_another_display_moves_the_pin() {
        let mut machine = picking(100.0, 400.0);
        machine.apply(Input::Choose { over: 7 });
        machine.apply(Input::Choose { over: 8 });
        assert_eq!(machine.observed(), Some(8));
    }

    #[test]
    fn inspecting_again_while_pinned_starts_a_new_pick() {
        let mut machine = picking(100.0, 400.0);
        machine.apply(Input::Choose { over: 7 });
        machine.apply(Input::Pick);
        assert!(machine.is_picking());
        assert_eq!(
            machine.observed(),
            Some(7),
            "the pin holds until another is chosen"
        );
    }

    #[test]
    fn clear_releases_even_a_pin_and_ends_the_pick() {
        let mut machine = picking(100.0, 400.0);
        machine.apply(Input::Choose { over: 7 });
        machine.apply(Input::Pick);
        machine.apply(Input::Clear);
        assert_eq!(machine.observed(), None);
        assert!(!machine.is_picking());
    }
}
