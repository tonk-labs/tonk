//! Reflect lifecycle phase onto the host as `data-state`.
//!
//! The [`State`] enum and its `as_str` mapping are target-independent
//! so they can be unit-tested natively. The `set` DOM function lives
//! behind a `wasm32` cfg gate.

#[cfg(target_arch = "wasm32")]
use web_sys::Element;

/// The four states authors can target from CSS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Ready/Empty/Error wired in once subscriptions land.
pub enum State {
    /// Subscriptions opening, no frame yet.
    Loading,
    /// Strip rendered.
    Ready,
    /// Workspace has zero columns (or doesn't exist yet).
    Empty,
    /// Query / network failure.
    Error,
}

impl State {
    /// The `data-state` attribute value for this state. CSS rules
    /// in `styles.css` rely on these exact strings — a typo here
    /// silently breaks every skin downstream.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            State::Loading => "loading",
            State::Ready => "ready",
            State::Empty => "empty",
            State::Error => "error",
        }
    }
}

/// Set `data-state` on `host`. Idempotent — safe to call repeatedly
/// with the same state.
#[cfg(target_arch = "wasm32")]
pub fn set(host: &Element, state: State) {
    let _ = host.set_attribute("data-state", state.as_str());
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn it_maps_states_to_data_state_attribute_values() {
        assert_eq!(State::Loading.as_str(), "loading");
        assert_eq!(State::Ready.as_str(), "ready");
        assert_eq!(State::Empty.as_str(), "empty");
        assert_eq!(State::Error.as_str(), "error");
    }
}
