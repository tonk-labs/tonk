use leptos::prelude::*;
use leptos_router::hooks::use_location;

use crate::{components::ActiveSubject, did};

/// Sidebar content slotted into `<wa-page>`'s navigation regions.
/// Interactive items are `<wa-button>` with `href` so they render
/// as real anchors — navigation belongs to links, not buttons.
#[component]
pub fn TonkToolbar() -> impl IntoView {
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

    // Current path drives the active-space indicator. Reading
    // `pathname` here keeps the toolbar reactive to route changes
    // without needing a dedicated context.
    let location = use_location();
    let active_space = Signal::derive(move || {
        location
            .pathname
            .get()
            .strip_prefix("/space/")
            .map(str::to_string)
    });
    let is_active = move |name: &'static str| {
        let active = active_space;
        Signal::derive(move || active.get().as_deref() == Some(name))
    };
    let home_active = is_active("home");
    let scratch_active = is_active("scratch");

    view! {
        <div slot="navigation-header" class="sidebar-section sidebar-section--flush">
            <wa-button
                class="sidebar-space"
                class:is-active=move || home_active.get()
                href="/space/home"
                aria-label="Open home space"
            >
                <tonk-sigil
                    class="sidebar-sigil"
                    value=move || sigil_value.get()
                ></tonk-sigil>
            </wa-button>
            <wa-button
                class="sidebar-space"
                class:is-active=move || scratch_active.get()
                href="/space/scratch"
                aria-label="Open scratch space"
            >
                <tonk-sigil class="sidebar-sigil">"scratch"</tonk-sigil>
            </wa-button>
        </div>
        <div slot="navigation" class="sidebar-section">
            <wa-button
                href="/space/new"
                appearance="plain"
                variant="neutral"
                aria-label="Add space"
            >
                <wa-icon name="plus"></wa-icon>
            </wa-button>
        </div>
        <div slot="navigation-footer" class="sidebar-section">
            <wa-button
                href="/help"
                appearance="plain"
                variant="neutral"
                aria-label="Help"
            >
                <wa-icon name="circle-question"></wa-icon>
            </wa-button>
            <wa-button
                appearance="plain"
                variant="neutral"
                aria-label="Toggle theme"
            >
                <wa-icon name="moon"></wa-icon>
            </wa-button>
            <wa-button
                class="sidebar-profile"
                href="/profile"
                appearance="plain"
                variant="neutral"
                aria-label="Profile"
            >
                <wa-avatar label="User" class="sidebar-avatar"></wa-avatar>
            </wa-button>
        </div>
    }
}
