//! `/vaults` — manage local-disk vaults backed by the File System
//! Access API.
//!
//! Renders the per-browser-profile registry, paints a tri-state
//! status indicator per entry, and exposes per-row Reconnect /
//! Sync / Forget actions. The page is purely a management view —
//! vaults are *created* by the join flow ("I have this on disk"
//! on `/join?access=…`), which is where the subject DID is known
//! and the registry entry can be wired to a profile space. An
//! "import vault from disk" entry point that creates a new space
//! from a picked directory is a separate piece of work.

use leptos::{logging::log, prelude::*, task::spawn_local};
use std::collections::HashMap;
use web_sys::{FileSystemDirectoryHandle, PermissionState};

use crate::{
    api,
    components::ProfileResource,
    error::TonkUiError,
    fs_access,
    vaults::{VaultEntry, VaultRegistry, VaultRegistryError, VaultStatus},
};

/// Top-level `/vaults` view.
#[component]
pub fn TonkVaults() -> impl IntoView {
    let profile_resource =
        use_context::<ProfileResource>().expect("ProfileResource provided by TonkShell");

    // Reactive list of vaults. Re-fetched after reconnect / sync /
    // delete so the UI converges without bespoke per-row signals.
    let entries: RwSignal<Result<Vec<VaultEntry>, String>, LocalStorage> =
        RwSignal::new_local(Ok(Vec::new()));
    let refreshing = RwSignal::new(false);
    // Last-sync error per vault, surfaced inline on the row that
    // produced it. Keyed by vault id.
    let sync_errors: RwSignal<HashMap<String, String>, LocalStorage> =
        RwSignal::new_local(HashMap::new());

    let reload = move || {
        refreshing.set(true);
        spawn_local(async move {
            match load_with_status().await {
                Ok(list) => entries.set(Ok(list)),
                Err(e) => entries.set(Err(format!("{e}"))),
            }
            refreshing.set(false);
        });
    };

    // Boot scan on mount. Permission-only — without a user gesture
    // it can't escape "yellow", but it can detect "red" (handle
    // resolves to nothing) and paint accordingly.
    reload();

    // Reverse-map of subject DID → local space name from the
    // shared profile resource. Drives the per-row "Sync" button:
    // a vault entry with a `subject_did` matching a space is
    // ready to wire as that space's FS upstream. Recomputed when
    // the profile resource updates so a newly-joined space starts
    // offering Sync without a manual reload.
    let space_by_did: Signal<HashMap<String, String>, LocalStorage> =
        Signal::derive_local(move || {
            let Some(Ok(Some(info))) = profile_resource.get() else {
                return HashMap::new();
            };
            info.space
                .into_iter()
                .map(|(name, did)| (did.to_string(), name))
                .collect()
        });

    view! {
        <section class="tonk-vaults">
            <header>
                <h1>"Vaults on this device"</h1>
            </header>

            { move || match entries.get() {
                Ok(list) if list.is_empty() => view! {
                    <wa-callout variant="neutral">
                        <wa-icon slot="icon" name="circle-info"></wa-icon>
                        "No vaults registered on this device. Open an invite "
                        "link and pick \"I have this on disk\" to add one."
                    </wa-callout>
                }.into_any(),
                Ok(list) => view! {
                    <ul class="vaults-list">
                        { list.into_iter().map(|entry| {
                            let matched_space = entry
                                .subject_did
                                .as_deref()
                                .and_then(|did| space_by_did.get().get(did).cloned());
                            render_row(entry, matched_space, sync_errors, reload)
                        }).collect_view() }
                    </ul>
                }.into_any(),
                Err(message) => view! {
                    <wa-callout variant="danger">
                        <wa-icon slot="icon" name="circle-exclamation"></wa-icon>
                        { message }
                    </wa-callout>
                }.into_any(),
            } }

            <Show when=move || refreshing.get()>
                <wa-spinner></wa-spinner>
            </Show>
        </section>
    }
}

/// Render a single registered vault. Action buttons key off
/// `last_known_status` so the visible affordance matches what's
/// actually possible without the user gesturing again.
///
/// `matched_space` is the local name of the profile space whose
/// subject DID matches this vault's `subject_did`, when both are
/// known. Drives the "Sync" button: without a match there's no
/// repo to wire as upstream, so we hide the action rather than
/// guess.
fn render_row(
    entry: VaultEntry,
    matched_space: Option<String>,
    sync_errors: RwSignal<HashMap<String, String>, LocalStorage>,
    reload: impl Fn() + Copy + 'static,
) -> impl IntoView {
    let (status_label, status_variant) = match entry.last_known_status {
        VaultStatus::Green => ("Ready", "success"),
        VaultStatus::Yellow => ("Tap to reconnect", "warning"),
        VaultStatus::Red => ("Vault not found", "danger"),
    };
    let display_name = entry.display_name.clone();
    let entry_id = entry.id.clone();
    let id_for_reconnect = entry.id.clone();
    let handle_for_reconnect = entry.handle.clone();
    let id_for_sync = entry.id.clone();
    let handle_for_sync = entry.handle.clone();
    let id_for_error_read = entry.id.clone();

    let on_reconnect = move |_| {
        let id = id_for_reconnect.clone();
        let handle = handle_for_reconnect.clone();
        spawn_local(async move {
            match reconnect_vault(&id, &handle).await {
                Ok(()) => reload(),
                Err(e) => log!("reconnect failed: {e}"),
            }
        });
    };

    let on_sync = {
        let matched_space = matched_space.clone();
        move |_| {
            let Some(space) = matched_space.clone() else {
                return;
            };
            let id = id_for_sync.clone();
            let handle = handle_for_sync.clone();
            sync_errors.update(|errors| {
                errors.remove(&id);
            });
            spawn_local(async move {
                match sync_vault(&space, &id, &handle).await {
                    Ok(()) => {
                        sync_errors.update(|errors| {
                            errors.remove(&id);
                        });
                        reload();
                    }
                    Err(e) => {
                        log!("sync_vault({id}, {space}): {e}");
                        sync_errors.update(|errors| {
                            errors.insert(id.clone(), format!("{e}"));
                        });
                    }
                }
            });
        }
    };

    let id_for_remove = entry_id.clone();
    let on_remove = move |_| {
        let id = id_for_remove.clone();
        spawn_local(async move {
            match remove_vault(&id).await {
                Ok(()) => reload(),
                Err(e) => log!("remove failed: {e}"),
            }
        });
    };

    let sync_button = match (entry.last_known_status, matched_space.clone()) {
        (VaultStatus::Green, Some(space)) => view! {
            <wa-button size="small" variant="brand" on:click=on_sync>
                { format!("Sync to {space}") }
            </wa-button>
        }
        .into_any(),
        // Subject DID matches a known space but permission isn't
        // green yet — Reconnect runs first, surfaced via the
        // status-block button below.
        _ => view! { <span></span> }.into_any(),
    };

    view! {
        <li class="vaults-list__row" data-vault-id=entry_id>
            <div class="vaults-list__name">{ display_name }</div>
            <wa-badge variant=status_variant>{ status_label }</wa-badge>
            <div class="vaults-list__actions">
                { match entry.last_known_status {
                    VaultStatus::Yellow => view! {
                        <wa-button size="small" on:click=on_reconnect>"Reconnect"</wa-button>
                    }.into_any(),
                    _ => view! { <span></span> }.into_any(),
                } }
                { sync_button }
                <wa-button size="small" appearance="plain" on:click=on_remove>
                    "Forget"
                </wa-button>
            </div>
            { move || sync_errors
                .get()
                .get(&id_for_error_read)
                .cloned()
                .map(|message| view! {
                    <wa-callout variant="danger">
                        <wa-icon slot="icon" name="circle-exclamation"></wa-icon>
                        { message }
                    </wa-callout>
                })
            }
        </li>
    }
}

/// Re-register the handle with the worker, wire the FS upstream
/// for `(space, "main")`, and pull. Also updates the registry
/// entry's `last_synced_at` on success.
async fn sync_vault(
    space: &str,
    vault_id: &str,
    handle: &FileSystemDirectoryHandle,
) -> Result<(), TonkUiError> {
    // Browsers reset FS-Access permission per session; re-request
    // here so a fresh page load can sync without a separate
    // Reconnect click. If the user already granted in this
    // session, this is a no-op.
    match fs_access::request_readwrite_permission(handle).await {
        Ok(PermissionState::Granted) => {}
        Ok(_) => return Err(TonkUiError::other("Permission was not granted")),
        Err(e) => return Err(e),
    }
    fs_access::register_handle_with_worker(vault_id, handle)?;
    api::set_fs_upstream(space, "main", vault_id).await?;
    api::pull(space, "main").await?;

    let registry = VaultRegistry::open()
        .await
        .map_err(|e| TonkUiError::other(format!("opening registry: {e}")))?;
    let now_ms = js_sys::Date::now();
    if let Err(e) = registry.touch_last_synced(vault_id, now_ms).await {
        log!("touch_last_synced('{vault_id}'): {e}");
    }
    Ok(())
}

/// Open the registry, list every entry, then probe each handle's
/// readwrite permission so the boot scan paints the right colour.
/// Permission probe never prompts, so the worst it can do is paint
/// yellow.
async fn load_with_status() -> Result<Vec<VaultEntry>, VaultRegistryError> {
    let registry = VaultRegistry::open().await?;
    let mut list = registry.list().await?;
    for entry in list.iter_mut() {
        let probed = match fs_access::query_readwrite_permission(&entry.handle).await {
            Ok(PermissionState::Granted) => VaultStatus::Green,
            Ok(PermissionState::Prompt) => VaultStatus::Yellow,
            Ok(PermissionState::Denied) | Ok(_) => VaultStatus::Red,
            Err(e) => {
                log!("queryPermission failed for vault '{}': {e}", entry.id);
                VaultStatus::Red
            }
        };
        if probed != entry.last_known_status {
            entry.last_known_status = probed;
            // Best-effort write-back; if it fails, log and keep
            // serving the in-memory view.
            if let Err(e) = registry.update_status(&entry.id, probed).await {
                log!("update_status('{}') failed: {e}", entry.id);
            }
        }
    }
    Ok(list)
}

/// Yellow → green reconnect: re-prompt for permission, update the
/// registry, hand the handle back to the worker on success.
async fn reconnect_vault(
    id: &str,
    handle: &FileSystemDirectoryHandle,
) -> Result<(), TonkUiError> {
    let state = fs_access::request_readwrite_permission(handle).await?;
    let registry = VaultRegistry::open()
        .await
        .map_err(|e| TonkUiError::other(format!("opening registry: {e}")))?;
    let next = match state {
        PermissionState::Granted => VaultStatus::Green,
        PermissionState::Prompt => VaultStatus::Yellow,
        _ => VaultStatus::Red,
    };
    registry
        .update_status(id, next)
        .await
        .map_err(|e| TonkUiError::other(format!("update_status: {e}")))?;
    if next == VaultStatus::Green {
        fs_access::register_handle_with_worker(id, handle)?;
    }
    Ok(())
}

/// Drop a vault from the registry and tell the worker to forget
/// its handle. Best-effort: registry removal is the source of
/// truth, the worker-side unregister is a hint.
async fn remove_vault(id: &str) -> Result<(), TonkUiError> {
    let registry = VaultRegistry::open()
        .await
        .map_err(|e| TonkUiError::other(format!("opening registry: {e}")))?;
    registry
        .remove(id)
        .await
        .map_err(|e| TonkUiError::other(format!("remove: {e}")))?;
    let _ = fs_access::unregister_handle_with_worker(id);
    Ok(())
}
