# Hub account switcher and settings implementation plan

**Goal:** Restore the FABB Hub account switcher, replace the current `/account` shortcut with an in-Hub settings surface backed by real account/device data, and correct removal copy so it describes device-local behavior.

**Approach:** Add one light-DOM `<ui-hub-account>` custom element to the Hub bar. It owns the account menu and fixed settings overlay, loads the existing profile/account/device APIs through the sealed-guest fetch bridge, and keeps all passkey-backed or destructive operations in the existing top-document `/account` surface. The settings rail contains only Account and Devices; Usage, the usage banner, and the prototype's account-level Syncing pane stay absent until their backing product contracts exist.

**Constraints:**

- Use the existing `GET /api/profiles`, `POST /api/profiles/activate`, and `POST /api/profiles/add` contracts. Do not add another account/profile model or change worker switching semantics.
- Use the existing `GET /api/account/summary`, `GET /api/account/devices`, and `POST /api/account/display-name` contracts for settings data.
- Keep WebAuthn in the top document. Adding passkeys, revoking device access, signing out, and deleting an account navigate to `/account`; the sealed Hub guest must not attempt those ceremonies itself.
- After activating an existing profile, navigate the top page to `/`; after adding a profile, navigate to `/account` so the normal sign-in/create flow runs in the fresh profile.
- The settings overlay must not invent account-level sync-server, latency, or inactivity controls. Auto-sync is currently a per-space behavior.
- Do not add Usage UI, upgrade copy, billing calls, metering requests, or a usage warning banner.
- Space removal remains local to this device. It must not imply removal from the account, other devices, peers, or offline replicas.
- Preserve the current Hub space creation, listing, copy-link, removal, mode-switch, mobile, and reduced-motion behavior.
- Introduce no new third-party dependencies.

## File map

- `plan/hub-account-settings.md`: Durable scope, ordering, and verification contract for this slice.
- `rust/tonk-host/src/http.rs`: Host-relative JSON GET transport for sealed guests; existing POST transport remains the mutation path.
- `rust/tonk-host/src/lib.rs`: Narrow public exports for the host-relative JSON transport used by Hub chrome.
- `rust/tonk-workspace/src/ui_hub_account.html`: Static, semantic account-menu and Account/Devices settings markup.
- `rust/tonk-workspace/src/ui_hub_account.rs`: `<ui-hub-account>` lifecycle, typed API calls, rendering, menu/dialog behavior, profile switching, display-name commit, and navigation handoffs.
- `rust/tonk-workspace/src/lib.rs`: Module registration for `<ui-hub-account>`.
- `rust/tonk-core/assets/library/profile.yaml`: Hub bar integration, account-menu/settings styling, responsive behavior, and corrected space-removal copy.
- `rust/tonk-worker/tests/standard_library.rs`: Profile-library contract regression for the device-local removal wording.
- `rust/tonk-ui/src/account_flow.rs`: Real-browser Hub-frame helpers and account-switcher/settings regression coverage.

### Task 1: Add the Hub account menu on the existing profile roster

**Files:**

- Modify: `rust/tonk-host/src/http.rs:get_json, post_json`
- Modify: `rust/tonk-host/src/lib.rs`
- Create: `rust/tonk-workspace/src/ui_hub_account.html`
- Create: `rust/tonk-workspace/src/ui_hub_account.rs`
- Modify: `rust/tonk-workspace/src/lib.rs:register`
- Modify: `rust/tonk-core/assets/library/profile.yaml:view/directory! Hub bar markup and account-stack CSS`
- Test: `rust/tonk-workspace/src/ui_hub_account.rs:tests`

**Interfaces:**

- Consumes: `ProfilesResponse`, `ProfileRosterEntry`, and `ActivateProfileRequest` from `tonk-worker-api`.
- Calls: `GET /api/profiles`, `POST /api/profiles/activate`, and `POST /api/profiles/add` as bare host-relative URLs so the opaque guest's fetch bridge can relay them.
- Produces: `<ui-hub-account>` with `[data-account-trigger]`, `[data-account-menu]`, `[data-profile]`, `[data-add-profile]`, and an inline `[data-account-error]` status.
- Label rule: display name, then verified/cached email, then profile storage name; the active row is marked `aria-current="true"` and is not actionable.

- [x] Add a browser-DOM unit test with a synthetic `ProfilesResponse` requiring the active label, current row, switch buttons for inactive rows, local-workspace fallback, and Add account row; run it before defining the element and observe the missing custom element/menu failure.
- [x] Add a browser-DOM interaction test requiring `aria-expanded` to follow open/closed state, outside pointer and Escape to close the menu, and focus to return to the trigger.
- [x] Add `get_json` beside the existing host-relative `post_json`; both wait for service-worker readiness, pass bare `/api/...` strings to `window.fetch`, return response text only for 2xx, and retain status/body details in `ErrorDetail` on failure. Re-export only these two JSON helpers from `tonk-host`.
- [x] Implement the light-DOM account element and render roster rows from the typed response. Keep event closures and pending asynchronous state owned by the element so disconnect/reconnect does not stack listeners.
- [x] On a profile-row click, disable switch actions, POST the exact `profileName`, and call `tonk_host::navigate_to("/")` only after a successful response. On failure, keep the current page/account and show the returned error in the menu.
- [x] On Add account, POST `/api/profiles/add` and call `tonk_host::navigate_to("/account")` only after success. Do not open the sign-in ceremony inside the guest.
- [x] Replace the Hub's `account▸` link with `<ui-hub-account>` and restore the 216px account stack geometry from the wireframe, including the 144px narrow-phone rung swap and 44px touch targets.
- [x] Run `cargo fmt --package tonk-host --package tonk-workspace -- --check` and `nix develop . -c cargo test --target wasm32-unknown-unknown -p tonk-workspace ui_hub_account -- --nocapture`; expect the new DOM and interaction tests to pass.

### Task 2: Rework settings as a truthful Account and Devices overlay

**Files:**

- Modify: `rust/tonk-workspace/src/ui_hub_account.html`
- Modify: `rust/tonk-workspace/src/ui_hub_account.rs`
- Modify: `rust/tonk-core/assets/library/profile.yaml:Hub settings CSS`
- Test: `rust/tonk-workspace/src/ui_hub_account.rs:tests`

**Interfaces:**

- Consumes: the active `ProfileRosterEntry`, `AccountSummary { email, passkey }`, and `Vec<AccountDevice>`.
- Calls: `GET /api/account/summary` and `GET /api/account/devices` when settings opens; `tonk_host::set_account_display_name` when a non-blank edited name commits.
- Produces: a fixed `role="dialog" aria-modal="true" aria-labelledby="hub-settings-title"` with Account and Devices rail tabs, independent loading/error states, and top-page navigation handoffs.
- Navigation handoffs: passkey/account management goes to `/account`; a revocable device goes to `/account?revoke=<encoded DID>&attachment=<encoded attachment ID>`.
- Local-profile state: when the active roster entry has no provider, show `Not signed in on this profile`, suppress account/device requests, and offer `create account or log in` via `/account` rather than presenting network-unavailable copy.

- [x] Extend the browser-DOM test to require exactly two settings tabs (`account`, `devices`), labelled account/passkey facts, device rows, and the complete absence of Usage, upgrade, banner, metering, and account-level Syncing controls; run it against Task 1 and observe the missing settings failure.
- [x] Add a rendering test covering populated account summary/passkey facts, legacy `passkey: null`, provider-unreachable `email: null`, a provider-free local profile, current/active/revoked device states, and safe query encoding for the revoke deep link.
- [x] Add an interaction test covering open, rail switching, scrim close, close-button close, Escape close, Tab/Shift-Tab containment, restoration of the opening button's focus, and closure of the account menu before the settings dialog opens.
- [x] Author the settings shell in `ui_hub_account.html`. Account shows editable display name, account email, passkey-created date, creation browser/OS, and a `manage passkeys and account` action. Devices shows name, added date, current/revoked state, and `remove access` only when the existing device DTO proves the top-level account page can act on it.
- [x] For an attached account, load summary and devices concurrently on open. A failure in one pane must leave the other usable and render an explicit `Unavailable`/inline error instead of clearing already-known roster data. For a local profile, make no account/device request and render the sign-in handoff.
- [x] Commit a display-name edit on Enter or blur, reject blank input by restoring the last confirmed name, show a busy/error state during the write, and repaint both the settings field and Hub account trigger from the authoritative response.
- [x] Route all passkey, revocation, sign-out, and deletion work to `/account`; do not duplicate confirmations or authority code in the Hub component.
- [x] Style the settings surface from the FABB geometry: chrome stays visible, the spaces stack steps behind a scrim, the panel is a solid readable surface, rail selection is explicit, focus is visible, controls meet 44px on touch layouts, and the panel becomes a single-column rail/body stack at the existing 640px breakpoint. While open, mark the logo, space stack, and sibling Hub controls inert; remove inert state on every close/disconnect path.
- [x] Run `cargo fmt --package tonk-workspace -- --check` and `nix develop . -c cargo test --target wasm32-unknown-unknown -p tonk-workspace ui_hub_account -- --nocapture`; expect all data, failure-state, and keyboard tests to pass.

### Task 3: Correct Hub copy at the actual deletion boundary

**Files:**

- Modify: `rust/tonk-core/assets/library/profile.yaml:space/remove confirmation`
- Modify: `rust/tonk-worker/tests/standard_library.rs`

**Interfaces:**

- Consumes: unchanged `space/remove` command and device-local `RemoveSpaceHandler` behavior.
- Produces this confirmation: `Remove {name} from this device? A synced space can be rejoined with an invite link; a local-only space is gone for good. Removing it does not delete other members' copies.`

- [x] Add a profile-library contract test requiring the device-local sentence and rejecting the current `from this account, on every device` claim; run it before the copy edit and observe the old-copy failure.
- [x] Replace only the confirmation body. Keep the heading, cancel path, submit label, command binding, and local/synced permanence distinction unchanged.
- [x] Run `cargo test -p tonk-worker --test standard_library`; expect the library to parse/analyze/lower and the copy contract to pass.

### Task 4: Prove account switching and settings through the sealed Hub

**Files:**

- Modify: `rust/tonk-ui/src/account_flow.rs:iframe helpers, it_adds_a_second_account_and_switches_between_disjoint_space_lists`

**Interfaces:**

- Consumes: the real `<tonk-site>` opaque iframe, account service, service worker, profile roster, and existing virtual WebAuthn authenticator.
- Produces: end-to-end evidence that the Hub uses the existing account boundary and reloads the whole product after a switch.

- [x] Add a WebDriver helper that waits for `tonk-site > iframe`, enters that frame, locates Hub chrome, and always returns to the top browsing context before visiting `/account` or reading top-document UI.
- [x] Change the existing multi-account flow so the final switch back happens from `[data-account-menu]`, not `#account-profile-list`; before switching, assert the second account's Hub omits `First Garden`, and after the top-page navigation assert the first account's Hub lists it.
- [x] In the same signed-in flow, open Hub settings and assert the real email/passkey summary and current device appear, Usage/banner/Syncing controls do not, and the settings close action restores focus to its trigger.
- [x] Exercise a display-name edit in Hub settings and assert the returned name repaints the Hub account trigger and is present after reopening settings. Do not invoke account deletion or device revocation in this test.
- [x] Run `nix develop . -c cargo test -p tonk-ui --features integration-tests it_adds_a_second_account_and_switches_between_disjoint_space_lists -- --test-threads=1 --nocapture`; expect the browser to complete both account ceremonies and restore the first account's disjoint space list through the Hub switcher.

### Task 5: Verify the complete Hub slice

- [x] Run `cargo fmt --all -- --check` and `git diff --check`.
- [x] Run `cargo test -p tonk-worker --test standard_library`.
- [x] Run `nix develop . -c cargo test --target wasm32-unknown-unknown -p tonk-workspace --lib`.
- [x] Run `nix develop . -c cargo check -p tonk-host -p tonk-workspace --target wasm32-unknown-unknown`.
- [x] Run `nix develop . -c build:web`.
- [x] Run `nix develop . -c cargo test -p tonk-ui --features integration-tests it_adds_a_second_account_and_switches_between_disjoint_space_lists -- --test-threads=1 --nocapture` after the final build.
- [x] Inspect the built Hub in isolated Chrome at 1440px and 390px widths in light and dark modes. Verify account-stack anchoring, settings scrim/panel layering, rail collapse, long display-name/email/device wrapping, visible focus, 44px touch targets, no horizontal overflow, and no new console errors.
- [x] Re-read the final diff and rendered copy to confirm there is no Usage/banner/metering UI, no fabricated global Syncing setting, no WebAuthn call from the sealed guest, and no statement that local space removal deletes another replica.
