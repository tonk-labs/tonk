//! User interface components for the Tonk application.
//!
//! This crate provides the web-based UI for Tonk, built using the Leptos framework.
//! It compiles to Wasm and runs in the browser.

use leptos::{logging::log, prelude::*};
use tonk_worker::AuthorizeRequest;

use crate::api;

mod auth;
use auth::*;

mod launcher;
use launcher::*;

mod toolbar;
use toolbar::*;

mod space;
use space::*;

/// The root UI component for the Tonk application.
///
/// This component serves as the main entry point for the Tonk user interface,
/// rendering the primary application view.
#[component]
pub fn TonkShell() -> impl IntoView {
    log!("Tonk shell initializing...");

    let authorize_action =
        Action::new_local(|request: &AuthorizeRequest| api::authorize(request.clone()));
    let authorization = Signal::derive_local(move || match authorize_action.value().get() {
        Some(Ok(response)) => Some(response),
        _ => None,
    });

    provide_context(authorize_action);
    provide_context(authorization);

    view! {
        <TonkAuth></TonkAuth>
        <TonkLauncher></TonkLauncher>
    }
}
