use leptos::prelude::*;

use crate::{
    components::{ActiveSubject, Status},
    did,
};

/// Top navigation toolbar with app controls and user menu.
#[component]
pub fn TonkToolbar() -> impl IntoView {
    let status = use_context::<Signal<Status, LocalStorage>>().expect("Missing status");

    // The sidebar sigil derives from the current space's subject
    // DID's public-key bytes, the same derivation used by the
    // remote tiles — so a remote pointing at this same peer
    // renders the matching sigil. Before the repo loads the
    // signal is `None` and the sigil renders in its zero state.
    let active_subject =
        use_context::<ActiveSubject>().expect("ActiveSubject context provided by TonkShell");
    let sigil_value = Signal::derive(move || {
        active_subject.get().as_deref().and_then(|did| {
            did::did_key_prefix(did).map(|bytes| {
                let n = u32::from_be_bytes(bytes);
                format!("0x{n:08x}")
            })
        })
    });

    view! {
        <section
            class="toolbar"
            class:visible=move || status.get() == Status::Ready
        >
            <tonk-sigil class="toolbar-sigil" value=move || sigil_value.get()></tonk-sigil>
            <wa-button class="toolbar-add" appearance="plain" variant="neutral" aria-label="Add space">
                <wa-icon name="plus" variant="regular"></wa-icon>
            </wa-button>
            <div class="spacer"></div>
            <wa-button class="toolbar-icon" appearance="plain" variant="neutral" aria-label="Help">
                <wa-icon name="circle-question" variant="regular"></wa-icon>
            </wa-button>
            <wa-button class="toolbar-icon" appearance="plain" variant="neutral" aria-label="Theme">
                <wa-icon name="moon" variant="regular"></wa-icon>
            </wa-button>
            <wa-avatar class="toolbar-avatar" label="User"></wa-avatar>
        </section>
    }
}
