# Browser multi-account support implementation plan

**Goal:** Let several accounts be signed in on one browser concurrently, with an account switcher, where each account sees only its own spaces and the hidden profile/account repositories (display name, future preferences) follow the active account. Remove the confusion where account A's spaces remain visible and syncable after signing into account B.

**Approach:** Profile-per-account. Everything that should swap per account is already scoped to the worker profile: the replica index that is the space list, the `tonk-local-root-v1` record, the `tonk-account-provider-v1` attachment, the trusted-base marker, the hidden account-repository mount and its `AccountKeys` hiding, the `tonk-space-root-v1/*` escrow prefixes, the profile display name, and the UCAN certificate store. So instead of adding an account dimension to any of those, give each account its own profile and make switching repoint the existing active-profile pointer (`tonk-active-profile-v1` on the fixed registry profile, `rust/tonk-worker/src/device.rs`). The dormant `device::rotate` machinery becomes the seed of "add account". Isolation then falls out of existing structure rather than being patched leak by leak: the certificate store, restore inventory, and space list can never cross accounts because they never leave their profile.

A switcher menu needs to know about profiles it has not opened, so a small roster record lives beside the pointer on the registry profile and is maintained by the worker at the moments it already has the facts in hand (boot, link, unlink, rename, switch).

**Scope:** Browser only — `rust/tonk-worker`, `rust/tonk-worker-api`, `rust/tonk-ui`. No CLI changes and no account-service changes in this pass (follow-ups listed at the end).

**Verified constraints that shape the design:**

- A page reload does not restart the service worker. `rust/tonk-ui/assets/service_worker.js` memoizes the wasm worker (`tonkServiceWorkerResolves`), and `TonkServiceWorker::new` (`rust/tonk-worker/src/worker.rs:1676`) runs once per SW process. Switching must therefore rebuild `TonkState` in place — swap the value inside the `Arc<RwLock<TonkState>>` returned by `api_router_with_state` — and also write the pointer so a genuine SW restart boots the same profile.
- `validate_grant` (`rust/tonk-worker/src/router/identity.rs:44-59`) requires the root delegation's audience to equal the current profile DID. A ceremony run against profile A can never be persisted into profile B, so "add account" must rotate to the fresh profile first and then run the unchanged sign-in ceremony there.
- Device DID equals profile DID, and the account service enforces one active attachment per device DID globally (`devices_one_active_did`). Distinct profiles are distinct devices, so concurrent attachments from one browser do not collide, and switching needs no server interaction at all.
- `main@profile:tonk` (`rust/tonk-host/src/location.rs`) routes to whatever profile the worker opened; the name is a compatibility literal. The Hub directory view, the FAB space switcher, and every sealed profile view follow the active profile with zero changes.
- `Profile::open` is open-or-create, so an activation endpoint must validate the requested name against the roster before opening — an unvalidated name would silently mint a garbage key.
- Existing installs must grandfather in without migration: the current (possibly mixed) profile becomes one roster entry, bound to whatever account it is attached to, or listed as a local workspace if signed out. Local (never-signed-in) workspaces remain first-class switcher entries.

## Profile roster

New credential site on the registry profile, sibling of `ACTIVE_PROFILE_SITE` in `rust/tonk-worker/src/device.rs`:

```rust
const PROFILE_ROSTER_SITE: &str = "tonk-profile-roster-v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RosterEntry {
    pub profile_name: String,
    pub root_did: Option<String>,   // None => local workspace
    pub provider: Option<String>,
    pub email: Option<String>,      // best-effort, may lag
    pub display_name: Option<String>,
    pub last_active_at: u64,        // unix seconds
}
```

Stored as JSON-serialized `Vec<RosterEntry>`. Chosen over deriving the menu by opening every profile at render time, which would cost key-material load, IDB open, and credential reads per profile per render; the roster is one credential load, and every event that changes an entry already runs in the worker. Registry-profile credential writes are performed directly against storage exactly as `Registry::rotate` already does, so they work regardless of which profile is active — that is what makes the endpoint ordering below safe.

Staleness policy: the active profile's entry is refreshed inline by `GET /api/profiles` from live state (`resolve_display_name`, `local_root`, `provider` are cheap per-profile reads). Inactive entries are as-of their profile's last activation; a display name renamed on another device converges the next time that account's profile is activated. Documented and accepted.

`Registry` (currently a private struct) becomes pub(crate) with a handle stored on `TonkState` (`registry: device::Registry`) — the worker constructs `Registry::device()`, router tests construct the existing `scratch()` pattern so they neither collide with each other nor touch the real profile store. New methods: `set_active`, `read_roster`, `upsert_roster`, `remove_from_roster`.

Grandfathering is the boot-time upsert: on every worker boot the active profile's entry is written best-effort (inside the existing detached catch-up task in `TonkServiceWorker::new`), so existing installs self-populate. `rotate` has zero callers today, so no dormant pre-existing profiles need back-filling.

## Worker endpoints

New module `rust/tonk-worker/src/router/profiles.rs`, wired in `router.rs` next to the `/api/profile` block. DTOs in `rust/tonk-worker-api/src/profiles.rs` (`ProfileRosterEntry` camelCase mirror plus `active: bool`; `ProfilesResponse { active, profiles }`; `ActivateProfileRequest { profile }`), exported from lib.rs with the crate's serde round-trip tests. Pattern to copy: `rust/tonk-worker-api/src/account.rs`.

- `GET /api/profiles` — roster plus which entry is active, active entry refreshed from live state before returning.
- `POST /api/profiles/activate` — validate the name is `REGISTRY_PROFILE` or a roster member, then:
  1. build the replacement `TonkState` for the target profile without holding the state write lock, via a factored-out `boot_state` (see below);
  2. `registry.set_active` — only after the target opened successfully, so a failed open never repoints;
  3. `*state.write().await = new_state`;
  4. upsert the entry's `last_active_at` and respond with `ProfilesResponse`.
  After the swap, spawn the same detached `ensure_account_state` + `restore_spaces` catch-up the boot path runs. `restore_spaces`'s process-global in-flight guard needs no change. The calling tab reloads itself after the response.
- `POST /api/profiles/add` — rotate to a fresh profile via `registry.rotate` and swap with the same machinery. Abandoned-add reuse guard: if the current profile has no persisted local root (`identity::load_record` returns `None`) and no user-space replicas (`profile_name::real_space_keys` empty), it is already a fresh landing pad — return it unchanged instead of minting another orphan key.
- `DELETE /api/profiles/{name}` (polish) — roster-only removal: refuses the active profile with `Conflict`, removes the entry, leaves the `{name}.profile` database and any space databases untouched. No server-side detach.

Boot refactor in `rust/tonk-worker/src/worker.rs`: extract `boot_state(...) -> Result<TonkState, _>` from `TonkServiceWorker::new` steps 2–4 (open active profile, `session::open`, `Reactor::new` + `TonkState` construction, `bootstrap_profile`) so boot and activation share one path.

Roster maintenance hooks: small best-effort `upsert_roster` calls in `link`, `unlink`, and `establish_repository` (`rust/tonk-worker/src/router/account.rs`) and in the `rename_display_name` path (`rust/tonk-worker/src/router/account_state.rs`). Email is captured best-effort during `link` via the existing devices/summary fetch.

## Login, add-account, and sign-out rules

Worker-enforced rule for persisting a ceremony's root (`persist_root`, `rust/tonk-worker/src/router/identity.rs`):

- No persisted local root — link in place. Covers the grandfathered first sign-in of an existing local profile and every fresh profile minted by add-account.
- Same root as the persisted record — idempotent re-sign-in in place. `root_needs_persist` (`rust/tonk-ui/src/account.rs:664`) already skips the save; `persist_root`'s equal-record early return keeps it idempotent.
- Different root — `Conflict`, with error text directing to add-account. This restores the guard that commit 697c86cb5 relaxed, narrowing the relaxation to same-root record updates. It lands in the same change as the UI add-account flow, which replaces the relaxation's only purpose (switching accounts by re-login), so there is no intermediate regression.

Sign-out stays per-profile and non-destructive: `unlink` continues to zero the provider record and keep root, data, and certificates. Its roster upsert clears the entry's account fields so the row renders as a local workspace; the persisted local-root record stays on the profile, so "sign back in" still short-circuits in place. The existing confirm copy ("your spots stay here") remains accurate. With a switcher, switching becomes the common path and sign-out the rare one.

If a "sign back in" ceremony nevertheless returns a different root — the passkey assertion is a discoverable-credential picker with no `allowCredentials`, so the browser chooses — the worker's `Conflict` surfaces and the UI error offers the add-account action.

## Switcher UI

Account panel only in this pass (no FAB changes), in the existing `<tonk-account>` custom-element style (`rust/tonk-ui/src/account.rs`, `account.html`, `account.css`). Account routes already bypass the sealed portal because WebAuthn must run top-level.

- `account.html`: a profiles section inside `#account-success` above sign-out, with `#account-profile-list` and an `#account-add-profile` "Add account" button. A compact list on `#account-choice` shows the other roster entries plus a "Use a different account" affordance.
- `account.rs`: `render_profiles(host, &ProfilesResponse)` in the DOM-building style of `render_devices`; rows carry `data-activate="{profile_name}"`, the active row is marked and inert. Click delegation like the existing `data-revoke` listener: `api::activate_profile(name)` then `location.reload()`. "Add account" / "Use a different account": `api::add_account_profile()` then reload — the fresh profile lands on the normal Choice flow and the entire existing create/link ceremony code runs there unchanged.
- Which-account-am-I: the active roster entry's email/display name renders in the success masthead (the panel already shows `#account-email-value`); the active marker in the list covers the rest.
- `rust/tonk-ui/src/api.rs` wrappers in the shape of `unlink_account`: `list_profiles`, `activate_profile`, `add_account_profile`, and later `remove_profile`.

## Cross-profile residue

- **Shared space storage.** A space's IndexedDB/OPFS name is exactly its routing key, with no profile prefix, so two profiles replicating one space share its storage. The space-removal flow's best-effort `delete_space_storage` (`rust/tonk-worker/src/router/repository.rs`) run in one profile would break another profile's replica. Proportional guard now: before deleting storage, skip the deletion (still removing the replica rows) whenever the roster has more than one entry, with a log line. The failure mode is leaked storage, never data loss — blocks are re-fetchable for sync-enabled spaces. A precise guard that consults the other profiles' indexes is deferred.
- **`tonk:auto-sync:{repo}` localStorage** is keyed by routing key only and therefore shared across profiles. Accepted: per-account space sets are disjoint except deliberately shared spaces, where a shared pause preference is defensible; namespacing would orphan existing preferences.
- **sessionStorage pending intent** is per-tab and strays are already discarded by `load_status`; the add-account reload arrives with no `next`. No change.
- **Multi-tab.** Other open tabs keep talking to the swapped worker and render the new profile's data until reloaded. The switching tab reloads itself; broadcasting a reload to sibling clients is a listed follow-up.

## Delivery order

Three stacked changes, smallest first:

1. **Worker roster and switch plumbing.** `device.rs` roster (`RosterEntry`, registry methods, pub(crate) `Registry` with the handle on `TonkState`); `boot_state` extraction and boot-time roster upsert; `rust/tonk-worker-api/src/profiles.rs` DTOs; `GET /api/profiles` and `POST /api/profiles/activate`; roster maintenance hooks in link/unlink/establish/rename; the coarse `delete_space_storage` guard.
2. **Add-account flow and switcher UI.** `POST /api/profiles/add` with the reuse guard; `persist_root` tightening (replacing `it_replaces_the_local_root_after_signing_out`); `api.rs` wrappers; `account.html`/`account.rs`/`account.css` switcher rendering, activate/add handlers, and the Choice-panel routing.
3. **Polish (optional).** `DELETE /api/profiles/{name}` plus a "Remove from this browser" row action; roster email backfill on boot; sibling-tab reload broadcast if cheap.

## Tests

Named `it_does_x`; worker and UI suites are wasm-gated (darwin needs Chrome plus a major-matched chromedriver).

Worker:
- device.rs — `it_reads_an_empty_roster_before_any_entry_is_written`, `it_upserts_a_roster_entry_by_profile_name`.
- profiles.rs — `it_lists_the_active_profile_with_its_account_state` (via the existing `attach_test_account` helper), `it_refuses_to_activate_a_profile_the_roster_does_not_name`, `it_serves_the_other_profiles_spaces_after_activation` (create a space, add a profile, the swapped state lists none, switching back restores it), `it_repoints_the_active_pointer_only_after_the_target_profile_opens`, `it_rotates_to_a_fresh_profile_for_add_account`, `it_reuses_an_unattached_empty_profile_instead_of_rotating_again`, and later `it_refuses_to_remove_the_active_profile_from_the_roster`.
- account.rs — `it_marks_the_profile_local_in_the_roster_after_unlink`.
- identity.rs — `it_rejects_a_different_root_on_a_previously_linked_profile`.

UI:
- account.rs — extend `it_authors_a_single_signed_in_dashboard` selectors with `#account-profile-list` and `#account-add-profile`; `it_renders_local_and_account_roster_rows`; `it_marks_the_active_profile_row_inert`; `it_offers_a_different_account_from_the_choice_panel_when_a_root_is_persisted`.
- account_flow.rs integration, with a second virtual authenticator — `it_adds_a_second_account_and_switches_between_disjoint_space_lists`: sign up A, create a space, add account, sign up B, assert B's hub lacks A's space, switch back to A, assert it returns.

## Verification

- `cargo clippy --workspace --all-targets --all-features` and `cargo fmt --all --check` (the repo lint gate; `profiles.rs` must compile natively via the wasm_compat pattern).
- The wasm worker and UI test suites per the repo testing setup.
- Manual: serve the UI; sign in as A and create a space; add account and sign up B; the switcher shows both entries and each hub shows only its own spaces; the hidden display name follows each account; sign out B and its row turns local; switch to A and its spaces and name return; hard-restart the service worker (close all tabs or update) and it boots into the profile the pointer names.

## Deferred follow-ups

- Server-side detach on sign-out or remove-from-browser. The browser has no detach path at all today (the CLI queues a `SignedDetachIntent`); the stale active attachment row on the account service is pre-existing behavior, made harmless for switching by per-profile device DIDs.
- The precise shared-space storage guard (per-profile index consultation before `delete_space_storage`), and profile-data deletion on remove-from-browser (the `{name}.profile` database and orphaned space databases stay).
- Sibling-tab refresh broadcast after a switch.
- FAB switcher entry point; per-profile namespacing of `tonk:auto-sync:*`; any preferences UI.
- CLI multi-account: grow `AccountSessionState` to multiple accounts with an active pointer, associate spot entries with an account, and filter listing/reconciliation by the active account — leaning on the already-account-keyed `tonk-space-root-v2/{subject}/{root}` prefixes.
