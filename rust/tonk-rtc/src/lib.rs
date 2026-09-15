#![warn(missing_docs)]

//! A WebRTC data channel between the `tonk` CLI and a browser tab.
//!
//! This is the transport half of a proof of concept. The CLI creates an
//! offer, a browser page answers it, and the two get a bidirectional
//! [`RTCDataChannel`]. What rides on the channel is the caller's
//! business; today that is chat lines, and the intent is that it
//! eventually becomes dialog's remote effects so the CLI can serve as a
//! sync remote for the browser.
//!
//! # The three pieces
//!
//! - [`signal`] — the session descriptions the peers exchange, and how
//!   they are encoded. Target-agnostic; both halves agree on it.
//! - [`peer`] — the native peer. Offers, then talks.
//! - [`loopback`] — a same-machine signalling channel, modelled on the
//!   CLI's account-authorization callback.
//!
//! # What the browser half looks like
//!
//! It is not in this crate. The browser uses its own
//! [`RTCPeerConnection`], driven from `rust/tonk-ui/assets/rtc.mjs`.
//!
//! One constraint worth knowing before this grows into real sync:
//! **`RTCPeerConnection` is `[Exposed=Window]`** — it does not exist in
//! a service worker. Tonk's replica and sync engine live in the service
//! worker (`tonk-worker`), so a WebRTC sync transport cannot simply be
//! dropped in beside the S3 one. The peer connection has to live in the
//! page and every invocation has to cross a page↔worker `MessagePort`
//! (the shape `tonk-worker/src/router/bridge.rs` already implements for
//! the sealed iframe). That hop, not the transport, is the hard part.
//!
//! [`RTCDataChannel`]: https://developer.mozilla.org/en-US/docs/Web/API/RTCDataChannel
//! [`RTCPeerConnection`]: https://developer.mozilla.org/en-US/docs/Web/API/RTCPeerConnection

/// Choosing which page carries an operation, and failing it over.
/// Target-agnostic: this is the policy `tonk-worker` needs, with no
/// worker, no `postMessage` and no WebRTC in it.
pub mod dispatch;
pub mod signal;

#[cfg(not(target_arch = "wasm32"))]
pub mod dial;
#[cfg(not(target_arch = "wasm32"))]
pub mod loopback;
#[cfg(not(target_arch = "wasm32"))]
pub mod peer;

pub use signal::{Description, Role};

#[cfg(not(target_arch = "wasm32"))]
pub use dial::{Address, Candidate, Listener};
#[cfg(not(target_arch = "wasm32"))]
pub use loopback::{Loopback, SignalError};
#[cfg(not(target_arch = "wasm32"))]
pub use peer::{Offering, PeerError, Session};
