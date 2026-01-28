use crate::components::TopBar;
use leptos::prelude::*;

/// Main workspace area for displaying Tonks.
#[component]
#[allow(clippy::unused_unit)]
pub fn TonkSpace() -> impl IntoView {
    view! {
        <section class="space">
            <TopBar></TopBar>
        </section>
    }
}
