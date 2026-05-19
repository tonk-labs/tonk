//! `/vaults` — manage local-disk vaults backed by the File System
//! Access API.
//!
//! Renders the per-browser-profile registry, paints a tri-state
//! status indicator per entry, and exposes an "Open existing vault
//! on disk" CTA that walks the user through `showDirectoryPicker`,
//! a display-name prompt, and registration with the worker.
//!
//! Recovery affordances (Reconnect / Re-locate / Sync now) are
//! scaffolded as placeholders — landing them next once the open
//! flow is shaken out end-to-end.

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

    // Reactive list of vaults. Re-fetched after open / reconnect /
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

    let on_open_click = move |_| {
        spawn_local(async move {
            match open_existing_vault().await {
                Ok(Some(())) => reload(),
                Ok(None) => {
                    // User cancelled the picker — leave the list as-is.
                }
                Err(e) => {
                    log!("open_existing_vault: {e}");
                    entries.set(Err(format!("{e}")));
                }
            }
        });
    };

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
                <wa-button variant="brand" on:click=on_open_click>
                    "Open existing vault on disk"
                </wa-button>
            </header>

            { move || match entries.get() {
                Ok(list) if list.is_empty() => view! {
                    <wa-callout variant="neutral">
                        <wa-icon slot="icon" name="circle-info"></wa-icon>
                        "No vaults registered on this device yet. "
                        "Click \"Open existing vault on disk\" to add one."
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

/// Drive the open-existing flow: pick a directory, ask for a
/// display name, write the entry to the registry, and hand the
/// handle to the worker. Returns `Ok(None)` if the user cancels
/// the picker.
async fn open_existing_vault() -> Result<Option<()>, TonkUiError> {
    let Some(handle) = fs_access::show_directory_picker().await? else {
        return Ok(None);
    };

    let display_name = prompt_display_name(handle.name())?;
    let Some(display_name) = display_name else {
        return Ok(None);
    };

    let vault_id = random_vault_id()?;

    let registry = VaultRegistry::open()
        .await
        .map_err(|e| TonkUiError::other(format!("opening registry: {e}")))?;
    let entry = VaultEntry {
        id: vault_id.clone(),
        display_name,
        handle: handle.clone(),
        subject_did: None,
        last_synced_at: None,
        last_known_status: VaultStatus::Yellow,
    };
    registry
        .put(&entry)
        .await
        .map_err(|e| TonkUiError::other(format!("saving registry entry: {e}")))?;

    // Best effort: ask for permission so the boot-scan reload paints
    // green immediately on the freshly registered row. Failure here
    // just leaves the status at yellow until the user re-clicks
    // Reconnect.
    if let Ok(PermissionState::Granted) = fs_access::request_readwrite_permission(&handle).await {
        let _ = registry.update_status(&vault_id, VaultStatus::Green).await;
        fs_access::register_handle_with_worker(&vault_id, &handle)?;
    }
    Ok(Some(()))
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

fn prompt_display_name(default: String) -> Result<Option<String>, TonkUiError> {
    let window = web_sys::window().ok_or_else(|| TonkUiError::other("window unavailable"))?;
    // `window.prompt(message, default)` — the `_with_message_and_default` overload.
    let result = window
        .prompt_with_message_and_default("Name this vault", &default)
        .map_err(|_| TonkUiError::other("prompt() rejected"))?;
    Ok(result.and_then(|s| {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }))
}

fn random_vault_id() -> Result<String, TonkUiError> {
    let window = web_sys::window().ok_or_else(|| TonkUiError::other("window unavailable"))?;
    let crypto = window
        .crypto()
        .map_err(|_| TonkUiError::other("crypto API unavailable"))?;
    Ok(crypto.random_uuid())
}
