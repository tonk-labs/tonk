use leptos::prelude::*;

use crate::components::Status;

/// Top navigation bar for the workspace.
#[component]
pub fn TopBar() -> impl IntoView {
    let status = use_context::<Signal<Status, LocalStorage>>().expect("Missing status");

    view! {
        <header
            class="topbar"
            class:visible=move || status.get() == Status::Authorized
        >
            <div class="space-info">
                <h1>"Space Title"</h1>
            </div>
        </header>
    }
}
