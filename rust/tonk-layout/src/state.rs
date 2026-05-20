//! Reflect lifecycle phase onto the host as `data-state` so
//! stylesheets can react to loading / ready / empty / error.
//!
//! The [`State`] enum and its `as_str` mapping are
//! target-independent so they can be unit-tested natively; the
//! `set` DOM function lives behind a `wasm32` cfg gate.

#[cfg(target_arch = "wasm32")]
use web_sys::Element;

/// The four states authors can target from CSS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Subscriptions opening; no strip rendered yet.
    Loading,
    /// Strip rendered.
    Ready,
    /// Workspace resolved but has zero columns.
    Empty,
    /// Query / network failure.
    Error,
}

impl State {
    /// The `data-state` attribute value for this state. Tests pin
    /// the mapping so CSS authors can rely on these strings.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            State::Loading => "loading",
            State::Ready => "ready",
            State::Empty => "empty",
            State::Error => "error",
        }
    }
}

/// Set `data-state` on `host`. Idempotent — safe to call
/// repeatedly with the same state.
#[cfg(target_arch = "wasm32")]
pub fn set(host: &Element, state: State) {
    let _ = host.set_attribute("data-state", state.as_str());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_maps_states_to_data_state_attribute_values() {
        // CSS rules (e.g. `tonk-layout[data-state="loading"]`)
        // rely on these exact strings — a typo here silently
        // breaks every loading/error skin downstream.
        assert_eq!(State::Loading.as_str(), "loading");
        assert_eq!(State::Ready.as_str(), "ready");
        assert_eq!(State::Empty.as_str(), "empty");
        assert_eq!(State::Error.as_str(), "error");
    }
}
