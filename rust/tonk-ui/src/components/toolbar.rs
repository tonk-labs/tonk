use js_sys::{decode_uri_component, encode_uri_component};
use leptos::prelude::*;
use leptos_router::hooks::use_location;

use crate::{components::ProfileResource, did};

/// Render a DID as the 8-hex-digit sigil value consumed by
/// `<tonk-sigil value=...>`. Returns `None` when the DID isn't a
/// `did:key` we can decode.
fn did_to_sigil(did: &str) -> Option<String> {
    did::did_key_prefix(did).map(|bytes| {
        let n = u32::from_be_bytes(bytes);
        format!("0x{n:08x}")
    })
}

/// Sidebar content slotted into `<wa-page>`'s navigation regions.
///
/// Space tiles and the profile footer are populated from
/// `GET /api/profile`, which returns the profile itself plus the
/// set of replicas this profile owns (each with its routing key,
/// display label, and subject DID for the sigil). Tiles route by the
/// key and label themselves by the label. Until the request resolves we
/// render nothing in the tile list — an empty strip is preferable
/// to a flash of stale hardcoded names. The `+` tile is always
/// rendered so navigation to "create a space" is available even
/// before the fetch lands.
#[component]
pub fn TonkToolbar() -> impl IntoView {
    // Profile data lives on the shell as a shared resource —
    // the shell refetches it in response to `/api/profile`
    // broadcasts, so the sidebar picks up new tiles as soon as
    // any write lands.
    let profile_resource =
        use_context::<ProfileResource>().expect("ProfileResource provided by TonkShell");

    // Current path drives the active-space indicator. Reading
    // `pathname` here keeps the toolbar reactive to route changes
    // without needing a dedicated context. The browser keeps the
    // pathname percent-encoded, so decode the first `/space/`
    // segment, then run it through `parse_space` to extract the
    // routing key — the tile compares against the key, not the label.
    let location = use_location();
    let active_space = Signal::derive(move || {
        let path = location.pathname.get();
        let raw = path.strip_prefix("/space/")?;
        // Only the first path segment encodes the space ref.
        let segment = raw.split('/').next().unwrap_or(raw);
        let decoded = decode_uri_component(segment)
            .ok()
            .and_then(|s| s.as_string())
            .unwrap_or_else(|| segment.to_string());
        crate::components::route::parse_space(&decoded).map(|space_ref| space_ref.name)
    });
    let profile_active = Signal::derive(move || location.pathname.get() == "/profile");
    let vaults_active = Signal::derive(move || location.pathname.get() == "/vaults");

    // Turn the space list into a stable-sorted list of tiles, carrying
    // the routing key (the URL the tile links by) and the subject DID
    // (drives the sigil). The membership index no longer carries a
    // display name (it lives in each space's own `tonk/repository`), so
    // the tile's aria-label and sort key fall back to the routing key.
    // The visible tile is a sigil, not text, so this is invisible in the
    // common case; surfacing the per-repo name here is a follow-up.
    let tiles = Signal::derive_local(move || {
        let info = profile_resource.get().and_then(|r| r.ok()).flatten()?;
        let mut spaces: Vec<(String, String, String)> = info
            .space
            .into_iter()
            .map(|entry| (entry.key.clone(), entry.key, entry.subject.to_string()))
            .collect();
        spaces.sort_by(|a, b| a.1.cmp(&b.1));
        Some(spaces)
    });

    // Profile footer's sigil is derived from the profile's own
    // DID — it's a property of the profile, not of whichever
    // space is currently active. `None` while the fetch is in
    // flight; the `<tonk-sigil>` element handles that by falling
    // back to its empty state.
    let profile_sigil = Signal::derive_local(move || {
        let info = profile_resource.get().and_then(|r| r.ok()).flatten()?;
        did_to_sigil(info.profile.subject.as_ref())
    });

    view! {
        // Standard wa-page navigation pattern: rail-style column
        // on desktop, drawer-on-hamburger on mobile. wa-page owns
        // the toggle and drawer below `mobile-breakpoint`.
        //
        // The wrapper `<div slot="navigation">` is load-bearing:
        // on desktop, the navigation slot is a CSS grid row (the
        // `1fr` middle track of `auto 1fr auto`), so multiple
        // slotted items get auto-placed into separate grid rows
        // that overflow the column. Wrapping into one container
        // gives us a single grid item that can lay its children
        // out as a flex column. The profile tile uses
        // `navigation-footer` so wa-page pins it to the bottom
        // row of the navigation grid.
        <div slot="navigation" class="sidebar-rail">
            { move || tiles.get().map(|spaces| {
                spaces
                    .into_iter()
                    .map(|(key, label, did)| {
                        // Route by the routing key (the repository's
                        // identity); encode it so URL-significant
                        // characters don't break the route. The
                        // aria-label uses the human-readable label.
                        let encoded = encode_uri_component(&key)
                            .as_string()
                            .unwrap_or_else(|| key.clone());
                        let href = format!("/space/{encoded}");
                        let aria = format!("Open {label} space");
                        let key_for_active = key.clone();
                        let is_active = Signal::derive(move || {
                            active_space.get().as_deref() == Some(key_for_active.as_str())
                        });
                        let sigil = did_to_sigil(&did);
                        view! {
                            <wa-button
                                class="sidebar-space"
                                class:is-active=move || is_active.get()
                                href=href
                                aria-label=aria
                            >
                                <tonk-sigil
                                    class="sidebar-sigil"
                                    value=sigil
                                ></tonk-sigil>
                            </wa-button>
                        }
                    })
                    .collect_view()
            }) }
            <wa-button
                class="sidebar-space sidebar-space--add"
                aria-label="Add space"
                href="/"
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

        <wa-button
            slot="navigation-footer"
            class="sidebar-space sidebar-space--vaults"
            class:is-active=move || vaults_active.get()
            href="/vaults"
            aria-label="Vaults on disk"
        >
            <wa-icon name="hard-drive" label="Vaults on disk"></wa-icon>
        </wa-button>
        <wa-button
            slot="navigation-footer"
            class="sidebar-space sidebar-space--profile"
            class:is-active=move || profile_active.get()
            href="/profile"
            aria-label="Profile"
        >
            <tonk-sigil
                class="sidebar-sigil"
                value=move || profile_sigil.get()
            ></tonk-sigil>
        </wa-button>
    }
}
