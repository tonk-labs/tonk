//! Carry browser client (Leptos SPA).
//!
//! This is the Leptos entrypoint. Routes, components, and the claim flow
//! will live in sibling modules as they land.

// Leptos components are PascalCase by convention.
#![allow(non_snake_case)]

use leptos::prelude::*;

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(|| view! { <App/> });
}

#[component]
fn App() -> impl IntoView {
    view! {
        <main>
            <h1>"Carry"</h1>
            <p>"Leptos scaffold ready. Phase 2b will port the claim flow here."</p>
        </main>
    }
}
