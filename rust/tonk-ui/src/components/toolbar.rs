use leptos::prelude::*;

use crate::components::Status;

/// Top navigation toolbar with app controls and user menu.
#[component]
pub fn TonkToolbar() -> impl IntoView {
    let status = use_context::<Signal<Status, LocalStorage>>().expect("Missing status");
    // TODO(cdata): This is all placeholder for now
    view! {
        <section
            class="toolbar"
            class:visible=move || status.get() == Status::Authorized
        >
            <img src="/images/tonk-logo.svg" />
            <img src="/images/circle-plus.svg" width="36"/>
            <div class="spacer"></div>
            <img src="/images/question-mark-circle.svg" width="30"/>
            <img src="/images/moon.svg" width="30"/>
            <img src="/images/dummy-avatar.png" width="40"/>
        </section>
    }
}
