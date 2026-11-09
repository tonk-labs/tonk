#![warn(missing_docs)]
//! The Tonk worker: what a site's kernel runs, and the native worker the CLI
//! and tests run.
//!
//! The worker is designed around an [`axum::Router`]. The mental model of
//! authoring routes is the same as though they were being authored in a server
//! context; a kernel answers its host's requests with it.
//!
//! To extend the worker with support for a new route, add one to the `router/`
//! directory and then include it in the router configuration in `router.rs`.

pub mod axum;
pub use axum::*;

mod router;
pub use router::*;

pub mod state_lock;
pub use state_lock::StateLock;

mod credential;

mod onboarding;

mod error;
pub use error::*;

mod worker;
pub use worker::*;

pub mod device;
pub mod session;

pub use device::{Standing, set_standing};
#[cfg(target_arch = "wasm32")]
pub use device::{app_origin, set_app_origin};

mod r#async;
pub use r#async::*;

pub use dialog_reactor as reactor;
pub use dialog_reactor::*;

#[cfg(any(test, feature = "helpers"))]
pub mod helpers;
