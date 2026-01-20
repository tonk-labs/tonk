use crate::components::{TonkSpace, TonkToolbar};
use leptos::prelude::*;

/// Main launcher view that combines the toolbar and workspace.
#[component]
pub fn TonkLauncher() -> impl IntoView {
    view! {
        <section class="launcher">
            <TonkToolbar></TonkToolbar>
            <TonkSpace></TonkSpace>
        </section>
    }
}
