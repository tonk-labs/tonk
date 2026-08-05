# Account panel dashboard implementation plan

**Goal:** Replace the signed-in account interstitial and separate device screen with one legible, responsive dashboard using only data already available to the browser UI.
**Approach:** Keep the existing `<tonk-account>` state machine and account authority operations, but author one `success` panel containing status, passkey guidance, the device list, and a clearly separated sign-out section. Load devices automatically whenever that panel is shown, and isolate its typography and controls from global Tonk document styles. Email, account creation time, passkey backup metadata, and passkey-provider inference are explicitly deferred because they need new authenticated data contracts.
**Constraints:**
- Keep WebAuthn ceremonies in the top-document `<tonk-account>` surface.
- Preserve account creation, login, handoff, setup, revocation, deep-link, and local sign-out semantics.
- Use only the existing `AccountStatus` and `AccountDevice` data contracts in this slice.
- Preserve unrelated work and introduce no new dependencies.
- Use familiar account language; keep authority terminology out of ordinary UI copy while retaining precise confirmation for permanent removal.
- Maintain at least 44px interactive hit areas, reduced-motion behavior, and responsive mobile layout.

## File map

- `plan/account-panel-dashboard.md`: Durable scope, decisions, and verification state for this slice.
- `rust/tonk-ui/src/account.html`: Signed-in dashboard semantics and newcomer-facing copy.
- `rust/tonk-ui/src/account.rs`: Automatic device loading, device-row presentation, and confirmation/status copy.
- `rust/tonk-ui/src/account.css`: Scoped heading reset, dashboard grid, consistent spacing, device rows, responsive rules, and interaction states.

### Task 1: Author one signed-in account dashboard

**Files:**
- Modify: `rust/tonk-ui/src/account.html:account-success`
- Modify: `rust/tonk-ui/src/account.rs:show_success, load_devices, bind`
- Test: `rust/tonk-ui/src/account.rs:tests`

**Interfaces:**
- Consumes: existing `AccountStatus`, `AccountDevice`, `account_devices()`, `unlink_account()`, and revocation handlers.
- Produces: the existing `success` mode as the sole signed-in dashboard; no wire-format changes.

- [x] Add `it_authors_a_single_signed_in_dashboard` covering the visible `Account` title, passkey explanation, device list, sign-out explanation, absence of the `Manage devices` interstitial, and absence of technical grant/authority copy.
- [x] Run the dashboard test against the old UI and observe the expected assertion failure on `Your Tonk account` versus `Account`.
- [x] Move the device list and sign-out controls into `#account-success`, remove the separate devices panel and obsolete navigation buttons, and keep the status message as a transient notice.
- [x] Make `show_success` and revoke deep links reveal the dashboard and load devices automatically without changing account or revocation authority behavior.
- [x] Run the focused dashboard test and confirm success.

### Task 2: Make device management legible without authority jargon

**Files:**
- Modify: `rust/tonk-ui/src/account.rs:render_devices, revocation_status, begin_revoke, bind`
- Test: `rust/tonk-ui/src/account.rs:tests`

**Interfaces:**
- Consumes: unchanged `AccountDevice { name, status, created_at, this_device, ... }`.
- Produces: device rows labelled with `Added <localized date>`, `This device`, `Access removed`, and `Remove access`; existing `data-*` revocation hooks remain intact.

- [x] Extend the device-list regression test to require plain-language dates, state labels, current-device marker, removal action, and legacy relink guidance.
- [x] Run the focused device-list test and observe the expected failure on the missing `This device` label.
- [x] Render a semantic identity/meta group and plain-language status labels while retaining the existing target identifiers and safe revocation eligibility checks.
- [x] Replace sign-out, revocation confirmation, and completion messages with plain language that distinguishes reversible local sign-out from permanent device-access removal.
- [x] Run the focused account tests and confirm success.

### Task 3: Isolate and polish the account layout

**Files:**
- Modify: `rust/tonk-ui/src/account.css`
- Test: browser inspection of the mounted `/account` surface at desktop and mobile widths.

**Interfaces:**
- Consumes: the dashboard classes authored by Tasks 1 and 2 and existing Web Awesome colour tokens.
- Produces: a centered responsive surface with a consistent 8px-derived spacing rhythm and no leaked global highlighted-heading treatment.

- [x] Explicitly reset account headings (`display`, `background`, `color`, `padding`, and box-decoration behavior) so `styles.css` cannot apply the site-wide black highlight treatment.
- [x] Add masthead, section, passkey-note, device-row, current-device badge, sign-out, and responsive grid styles; retain explicit transition properties, `scale: 0.96`, pretty/balanced wrapping, antialiasing, and 44px hit areas.
- [x] Run formatting, focused tests, Wasm checking, and whitespace checks successfully (see verification evidence below).
- [x] Build and serve the current UI, then inspect a disposable mounted dashboard in isolated headless Chrome at desktop and mobile widths. Confirm the heading reset, row alignment, responsive stacking, text wrapping, and 44px minimum hit areas. The static server predictably returns 404 for `/.well-known/tonk`; no authenticated provider or real device data was used.

## Verification evidence

- Red/green: the new dashboard test initially failed on the old title; the extended device test initially failed on the missing current-device label. Both passed after implementation.
- `nix develop -c cargo test --target wasm32-unknown-unknown -p tonk-ui --lib account::tests -- --nocapture`: 12 passed, 0 failed.
- `cargo fmt --package tonk-ui -- --check`: passed.
- `cargo check -p tonk-ui --target wasm32-unknown-unknown`: passed.
- `nix develop -c cargo test -p tonk-ui --no-run`: passed, compiling the updated real-browser flow assertions; one pre-existing unused-helper warning remains.
- `nix develop -c build:web`: passed; the optional FlakeHub cache returned 401 and Nix built locally.
- `git diff --check`: passed.
- Browser QA: built CSS inspected at 1440px and the Chrome runner's 500px minimum width with a disposable fake device list. No horizontal overflow; headings had transparent backgrounds and zero padding; visible actions measured 44px, 44px, and 46px high. This was visual/layout verification, not a live authenticated revocation ceremony.

## Deferred slices

- Authenticated account summary exposing verified email and account creation time.
- Versioned local passkey metadata for application-recorded creation time and browser-reported backup/attachment hints.
- Device rename, last-used activity, or exact password-manager/provider claims.
