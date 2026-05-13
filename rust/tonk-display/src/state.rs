//! Reflect lifecycle phase onto the host as `data-state`.

use web_sys::Element;

/// The four states authors can target from CSS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Resolving concept / view / entity; no DOM yet.
    Loading,
    /// Entity rendered.
    Ready,
    /// Entity not found / stream emitted zero rows.
    Empty,
    /// Concept lookup, view lookup, or network failure.
    Error,
}

impl State {
    fn as_str(self) -> &'static str {
        match self {
            State::Loading => "loading",
            State::Ready => "ready",
            State::Empty => "empty",
            State::Error => "error",
        }
    }
}

/// Set `data-state` on `host` to the textual form of `state`.
pub fn set(host: &Element, state: State) {
    let _ = host.set_attribute("data-state", state.as_str());
}
