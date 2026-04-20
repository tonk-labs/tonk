//! Index route view: renders the first-run modal over an empty main area.
//!
//! Once `/api/repositories` exists, this route will be conditional — the
//! modal only appears when the repo list is empty. Today the route is
//! always the modal, since there's no list to gate on.

use leptos::prelude::*;

use crate::components::TonkFirstRunModal;

/// Index route. For now, always renders the first-run modal.
#[component]
pub fn TonkEmpty() -> impl IntoView {
    view! {
        <section class="empty">
            <TonkFirstRunModal />
        </section>
    }
}
