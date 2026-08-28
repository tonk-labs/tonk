# Top-document settings convergence implementation plan

**Goal:** Replace the current editorial `/account` page with a canonical
`/settings` surface built from the FABB Account/Devices panel system, and bring
every create, login, handoff, loading, error, and confirmation state into the
same visual grammar without changing account lifecycle or authority semantics.

**Approach:** Make `/settings` and `/settings/link` the canonical browser
routes while retaining `/account` and `/account/link` as compatibility
redirects that preserve their query string. Keep `<tonk-account>` as the
trusted top-document controller because its WebAuthn ceremonies cannot move
into the sealed Hub guest. Rebuild its signed-in state as the production Hub's
attached Account/Devices settings panel, rebuild its intermediary states as
compact FABB ceremony clusters, and replace native browser confirms/alerts
with the reference confirmation cluster. Preserve the existing element IDs
and API calls where practical so the change remains presentational except for
display-name parity described below.

**Approved references:**

- `/Users/jackdouglas/tonk/gooey/fabb/hub.html`: Account/Devices rail and body,
  label/value rows, underlined action dialect, device rows, and confirmation
  cluster.
- `/Users/jackdouglas/tonk/gooey/fabb/onboard.html`: intermediary ceremony
  header, editable row, solid action row, narrator block, waiting/error motion,
  and bare-word exit.
- `rust/tonk-workspace/src/ui_hub_account.html` and
  `rust/tonk-core/assets/library/profile.yaml`: the production refinement of
  the Hub panel, including 576px desktop geometry, 432px intermediate geometry,
  compact top tabs, theme signal, focus treatment, and real data wording.

The standalone references supply component grammar, not product behavior. Do
not copy `hub.html`'s old left-aligned 810px shell, its prototype-only
"no spaces available" state, or `onboard.html`'s simulated code/display-name
steps. Use the current production Hub palette rather than introducing the
prototype's aubergine token values on `/settings` alone; browser coverage must
keep the Hub and settings tokens equal.

**Constraints:**

- WebAuthn, passkey verification, account deletion, and authoritative device
  revocation remain in top-document `rust/tonk-ui/src/account.rs`.
- `/settings` and `/settings/link` are canonical UI routes. Existing
  `/account` URLs remain compatibility redirects, but all generated in-product
  navigation and user-facing retry instructions use `/settings`. The
  `/api/account/*` contract names do not change.
- Preserve the current create-account, login, activation, add-passkey, CLI
  callback, revoke deep-link, sign-out, owned-space deletion, account deletion,
  profile switching, and return-to-`next` behavior.
- A provider-free local profile retains its local spaces and profile identity.
  Signed out, local, accountless, and empty are not interchangeable states.
- Treat profile add/switch behavior as a stable dependency. Retain its public
  selectors and hooks while changing only their presentation.
- Preserve the current exact deletion scope and safety gates: fresh server
  plan, matching account email, explicit acknowledgement, passkey verification
  for account deletion, and no passkey for one owned-space deletion.
- Use one ink palette. Destructive actions gain friction through wording,
  arming, and confirmation structure, not red styling.
- Chrome words are lowercase and bottom-right seated; user-provided names and
  email addresses retain their casing.
- Controls have at least 44px hit areas. Focus remains visible in both themes.
  Interactive motion is interruptible, disabled under reduced motion, and uses
  explicit transition properties rather than `transition: all`.
- No new API model, worker route, account-service route, dependency, or Hub
  lifecycle listener.

## State-to-surface contract

| Existing state | FABB surface after this change |
| --- | --- |
| `success` | Standalone attached settings view: Account/Devices rail plus panel body. |
| `choice` | Ceremony cluster with create and login action blocks, followed by other-browser account blocks. |
| `create` | Ceremony cluster with email editor row, current explanatory copy, solid create action, and bare back action. |
| `link` | Ceremony cluster with explanatory copy, solid passkey login action, and bare back action. |
| `handoff` | Permission-style CLI access request with a plain-language device-to-account relationship, optional technical DID disclosure, approval action, warning narrator, and cancel action. |
| busy status | The initiating solid row stays present, becomes inert, and carries the current status with the reference waiting treatment. |
| recoverable error | An inline narrator/error block receives focus or `aria-live` output and flashes an ink wash; the current mode stays mounted. |
| sign-out/revoke/delete confirmation | FABB modal cluster with scrim, heading, scope/body, cancel and solid confirm run, focus trap, Escape, and focus restoration. |

## File map

- `plan/account-panel-convergence.md`: durable scope, state mapping, delivery
  order, and verification contract.
- `rust/tonk-ui/src/account.html`: top-document settings panes, ceremony
  clusters, profile roster, and one reusable confirmation dialog.
- `rust/tonk-ui/src/account.css`: production-Hub token aliases, settings and
  ceremony geometry, responsive layout, focus, motion, and confirmation styles.
- `rust/tonk-ui/src/account.rs`: pane/cluster state, roster/device/summary
  rendering, display-name commit, custom confirmation state, focus management,
  and existing account actions.
- `rust/tonk-ui/src/api.rs`: top-document wrapper for the existing
  `POST /api/account/display-name` worker contract.
- `rust/tonk-ui/src/account_flow.rs`: real-browser lifecycle, custom-dialog,
  token-parity, geometry, and responsive regressions.
- `rust/tonk-ui/src/bin/ui.rs`: canonical `/settings` route recognition and
  compatibility redirects from the old `/account` paths.

### Task 1: Establish `/settings` and replace the signed-in dashboard with the attached Account/Devices panel

**Files:**

- Modify: `rust/tonk-ui/src/account.html:#account-success`
- Modify: `rust/tonk-ui/src/account.css`
- Modify: `rust/tonk-ui/src/account.rs:set_mode, show_success, render_summary, render_devices, render_profiles, bind`
- Modify: `rust/tonk-ui/src/api.rs`
- Modify: `rust/tonk-ui/src/bin/ui.rs`
- Test: `rust/tonk-ui/src/account.rs:tests`
- Test: `rust/tonk-ui/src/account_flow.rs`

**Interfaces:**

- Requests to `/settings` and `/settings/link` mount `<tonk-account>`.
  `/account` and `/account/link` redirect to their canonical equivalents while
  preserving query parameters used by `next`, revoke, add-account, and CLI
  callback flows. API routes remain under `/api/account/*`.
- `#account-success` contains a two-item `role=tablist` and two
  `role=tabpanel` children. Account is initially selected; Devices is selected
  for a `?revoke=` deep link.
- Desktop uses `144px + 432px` rail/body geometry inside a centered 576px
  shell. At 607px it uses `108px + 324px`; at 463px the rail becomes two
  full-width top tabs and the body has no horizontal overflow.
- Account pane rows are: editable display name, verified account email,
  passkey creation device/date, passkey explanation, activation status/action,
  accounts on this browser, add another passkey, sign out on this device, and
  permanent deletion entry. These retain existing IDs/data hooks.
- Browser-profile switch actions are soft icon-only chevrons with an
  account-specific accessible name and a 44px hit area. They have no filled
  cell, divider, persistent underline, or visible `switch` label.
- Devices pane contains the existing authoritative device list. Each row uses
  the Hub device-row grammar, preserves `data-revoke`, marks the current device,
  and does not infer last-seen data the API does not provide.
- Add `api::set_account_display_name(name: &str) -> Result<String,
  TonkUiError>` over the existing `POST /api/account/display-name` response.
  Enter and blur commit a trimmed nonblank value; success updates the active
  profile label, while failure restores the last confirmed value and displays
  an inline error.
- A bare `Back to Tonk`, or `Back` when `next` is present, remains available
  beneath the panel using the reference ghost-link grammar.

- [x] Add route coverage proving `/settings` and `/settings/link` mount the
      controller, old `/account` URLs reach the equivalent canonical route
      without losing query parameters, and generated UI navigation no longer
      points at the old route. Run it against the current router; expect
      failure because only `/account` is recognised.
- [x] Replace `it_authors_a_single_signed_in_dashboard` with a behavioral DOM
      test that selects both tabs, verifies their `aria-selected`, `tabindex`,
      and `hidden` states, and confirms every current account/device action is
      still reachable. Extend the existing real-account browser flow to change
      the name from the top-document panel and require the authoritative value
      after reload. Run the focused tests; expect failure on the missing
      rail/panes and top-document display-name path.
- [x] Re-author the signed-in markup and CSS using the Hub row/section/tab
      grammar. Remove the 760px white card, editorial `h1`, lime account badge,
      stacked subcards, red danger border, and native form-control styling.
- [x] Add `select_account_tab(host, name, focus)` and wire click plus ArrowLeft,
      ArrowRight, Home, and End behavior without document-global listeners.
- [x] Implement display-name loading/commit while retaining concurrent summary,
      devices, profiles, and activation requests and their isolated errors.
- [ ] Run the focused tests and the complete account Wasm suite; expect success.

### Task 2: Bring Choice, Create, Login, and CLI handoff panels to ceremony spec

**Files:**

- Modify: `rust/tonk-ui/src/account.html:#account-choice, #account-create, #account-link, #account-handoff`
- Modify: `rust/tonk-ui/src/account.css`
- Modify: `rust/tonk-ui/src/account.rs:set_mode, set_busy, show_error, clear_error, bind`
- Test: `rust/tonk-ui/src/account.rs:tests`
- Test: `rust/tonk-ui/src/account_flow.rs`

**Interfaces:**

- Every intermediary mode uses one centered, maximum-432px `.account-ceremony`
  cluster: 36px header block, 7px-separated data/action blocks, one narrator
  block, and a bare-word exit beneath it.
- Keep native `<form>`, `<label>`, and `<input type=email>` semantics even when
  visually rendered as a FABB editor row. Enter still submits Create account;
  the form never navigates.
- Choice offers separate `create account` and `log in` blocks. Other browser
  profiles use the existing roster label precedence and switch behavior;
  current profile rows are not rendered as switch targets. Other-profile rows
  use the same soft chevron action rather than a visible button label.
- Create keeps the real current flow: email input followed by the existing
  passkey/account operation. Login remains a single discoverable-passkey
  action. Do not add the prototype's verification-code or display-name steps.
- Handoff reads as a permission request, not a form: it names the requesting
  CLI/device, states which account and spaces it will use, and places Device
  and Profile DID values behind an optional technical-details disclosure. It
  only continues when the callback is valid, posts denial on Cancel, and
  preserves the `next`/callback redirect.
- `set_busy` sets `aria-busy`, disables every in-cluster navigation/action,
  changes only the initiating action label, and applies the calm waiting wash.
  `show_error` leaves the current cluster mounted and announces through a
  narrator/error region rather than switching modes.

- [x] Extend `it_authors_the_create_and_self_link_controls`,
      `it_prevents_every_account_form_from_navigating`, and
      `it_switches_between_account_panels_without_reauthoring_the_dom` to assert
      observable ceremony semantics, focus destination, and busy/error state.
      Run the focused account tests; expect failure against the old page/card
      structure.
- [x] Re-author the four modes while retaining their IDs and current click
      handlers. Add no lifecycle step that is absent from production.
- [ ] Run the focused tests; expect success.
- [ ] Run
      `nix develop . -c cargo test -p tonk-ui --features integration-tests it_signs_up_through_the_account_panels -- --test-threads=1 --nocapture`
      and
      `nix develop . -c cargo test -p tonk-ui --features integration-tests it_links_the_cli_through_the_browser_callback -- --test-threads=1 --nocapture`;
      expect both real-browser flows to pass without selector fallbacks to the
      former page layout.

### Task 3: Replace native confirms and alerts with one FABB confirmation cluster

**Files:**

- Modify: `rust/tonk-ui/src/account.html:confirmation dialog`
- Modify: `rust/tonk-ui/src/account.css:confirmation styles`
- Modify: `rust/tonk-ui/src/account.rs:render_deletion_plan, bind, begin_revoke`
- Test: `rust/tonk-ui/src/account.rs:tests`
- Test: `rust/tonk-ui/src/account_flow.rs`

**Interfaces:**

- Add one authored `role=alertdialog`, `aria-modal=true` cluster plus scrim.
  It has a heading, body/scope region, optional arming controls, Cancel, and a
  solid confirm action. Escape, Cancel, and scrim click close it and restore
  focus; Tab and Shift+Tab loop within it.
- Store the pending operation as a serializable internal enum:
  `Confirmation::SignOut`,
  `Confirmation::Revoke { did, self_revoke }`, and
  `Confirmation::Delete { plan, requested_space }`.
  Opening a second confirmation replaces the first; closing clears it.
- Sign-out and revocation use their current precise copy and API calls. Device
  removal identifies the selected device by DID at action time.
- Deletion first loads a fresh `AccountDeletionPlan`, then opens the dialog.
  The dialog renders only the selected owned space for `delete-space`, or all
  owned spaces for account deletion. The solid confirm remains disabled until
  the email exactly matches `plan.email` and the acknowledgement checkbox is
  checked. Account deletion still verifies the passkey; owned-space deletion
  still does not.
- Remove `window.confirm` and `window.alert` from these paths. Revocation
  success closes the dialog and uses the mounted status region. Deletion
  success replaces the dialog body with the existing result summary and a
  single `back to settings` action; only that action performs the canonical
  `/settings` navigation. Errors leave the confirmation available for retry
  when safe.

- [x] Add component tests for open/close/focus restoration, focus looping,
      destructive arming, wrong-email rejection, and operation replacement.
      Add real-browser assertions that sign-out, revoke, and deletion display
      the authored `alertdialog` rather than a browser prompt. Run the focused
      tests; expect failure because the current implementation uses native
      confirms/alerts and an in-flow deletion review.
- [x] Implement the confirmation enum, rendering, focus management, and action
      dispatch. Preserve the current fresh-plan and authority checks verbatim.
- [ ] Run the focused component tests; expect success.
- [ ] Run
      `nix develop . -c cargo test -p tonk-ui --features integration-tests it_deletes_the_account_and_its_hosted_spaces -- --test-threads=1 --nocapture`
      and
      `nix develop . -c cargo test -p tonk-ui --features integration-tests it_revokes_the_cli_device_from_the_browser -- --test-threads=1 --nocapture`;
      expect success.

### Task 4: Prove visual parity, responsive behavior, and lifecycle coverage

**Files:**

- Modify: `rust/tonk-ui/src/account_flow.rs`
- Test: `rust/tonk-ui/src/account_flow.rs`

**Interfaces:**

- A browser assertion compares the computed `/settings` values for `--page`,
  `--ink`, `--on-ink`, `--soft`, `--ring`, `--sep`, `--frost-solid`, `--panel`,
  `--wash`, and `--wash-2` with the production Hub values in both light and
  dark mode. This catches account-only palette drift without creating a new
  shared runtime dependency.
- At a 1200px viewport, the signed-in settings shell is 576px with a 144px
  rail and 432px body. At 607px it is at most 432px with a 108px rail and
  324px body. At 390px the tabs and body share the available width, no element
  creates horizontal overflow, and every visible interactive target is at
  least 44px in one dimension.
- Browser coverage visits and snapshots Choice, Create, Login, Handoff,
  Account, Devices, busy, error, and confirmation states. It checks semantics
  and geometry, not pixel hashes.

- [ ] Add token-parity and geometry tests and run them against the old account
      page; expect failure on missing Hub tokens, 760px card geometry, and
      absent compact tab layout.
- [ ] Make only the CSS/markup corrections exposed by those failures; do not
      weaken dimensions or add screenshot tolerances.
- [ ] Run the focused browser tests, then run:
      `nix develop . -c cargo test --target wasm32-unknown-unknown -p tonk-ui --lib account::tests -- --nocapture`,
      `nix develop . -c cargo test -p tonk-ui --features integration-tests it_adds_a_second_account_and_switches_between_disjoint_space_lists -- --test-threads=1 --nocapture`,
      `cargo fmt --all -- --check`, and `git diff --check`; expect success.
- [x] Run `nix develop . -c build:web`; expect success. Then start
      `nix develop . -c dev:web` and inspect `http://127.0.0.1:8080/settings` in
      an isolated headless Chrome session at 1200px, 607px, and 390px in light
      and dark mode. Record screenshots and computed geometry, stop both the
      browser and dev server, and report any account-service,
      physical-authenticator, or deployment path not exercised.

## Delivery order and review boundaries

1. Tasks 1 and 2 should land together only if the shared CSS cannot leave both
   old and new structures styled in an intermediate commit; otherwise keep
   them as separate signed-in and intermediary-state commits.
2. Task 3 is independently reviewable after the new shell exists.
3. Task 4 adds durable cross-surface evidence and is the final gate, not a
   substitute for each task's red/green tests.

The completed change supersedes only the presentation in
`plan/account-panel-dashboard.md`; that plan's verified authority and data
boundaries remain in force.

## Verification record (2026-08-26)

- Passed: `cargo fmt --all -- --check`, `git diff --check`, the native router,
  CLI account, CLI deployment, and workspace Hub tests, Wasm test compilation,
  integration-test compilation, changed-crate `cargo check --all-targets`, and
  `nix develop . -c build:web`.
- Configured-state browser inspection passed at 1200px (`576 = 144 + 432`),
  607px (`432 = 108 + 324`), and an emulated 390px viewport (358px available,
  no overflow, no visible target below 44px). Choice, Create, Login, Handoff,
  Account, Devices, busy/error, light/dark, and authored sign-out confirmation
  states were inspected. Escape closed the dialog and restored focus.
- Unverified: the Wasm account suite did not start because ChromeDriver was
  killed before the test page became available. The serialized integration
  flow did not start because the repository supplies ChromeDriver 150 while
  the installed Google Chrome is 152. Real sign-up, display-name persistence,
  CLI handoff, revoke, deletion, and second-account lifecycle flows therefore
  remain unchecked.
- The live accountless dev session reported a profile-roster initialization
  `500` with a version mismatch. It did not block ceremony rendering, but the
  signed-in browser states above were configured from the authored DOM rather
  than established through a real account.
