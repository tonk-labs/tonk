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
            <wa-button
                class="sidebar-space sidebar-space--add"
                href="/space/new"
                aria-label="Add space"
            >
                <svg
                    class="sidebar-add-glyph"
                    viewBox="0 0 128 128"
                    xmlns="http://www.w3.org/2000/svg"
                    aria-hidden="true"
                >
                    <circle cx="64" cy="64" r="56" class="sidebar-add-disc" />
                    <path d="M64 24 V104 M24 64 H104" class="sidebar-add-cross" />
                </svg>
            </wa-button>
        </div>
        <div slot="navigation-footer" class="sidebar-section sidebar-section--flush">
            <wa-button
                class="sidebar-space sidebar-space--full sidebar-space--profile"
                href="/profile"
                aria-label="Profile"
            >
                <wa-avatar class="sidebar-avatar" label="User"></wa-avatar>
            </wa-button>
        </div>
    }
}
