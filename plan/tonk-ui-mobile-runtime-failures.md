# Tonk UI confirmed mobile runtime failures implementation plan

**Goal:** Resolve the five failures reproduced on 2026-08-28 without widening
the mobile-hardening scope or weakening the existing accessibility contracts.

**Approach:** Preserve the current native-dialog, local-first, and compact FABB
designs. Add or strengthen a real-browser regression for each observed failure,
prove the current implementation fails at that boundary, then change the
smallest owning module: portal/UI focus transport, registration ceremony state,
the shared dialog primitive, Join CSS, or `tonk-display` lifecycle ordering.
Keep compilation, Wasm execution, WebDriver execution, and production-artifact
checks as separate evidence.

**Plan basis:** `a8f849cf11` plus the uncommitted mobile-hardening worktree,
inspected 2026-08-28.

**Constraints:**

- Preserve normal Tonk browser storage. Every browser run uses a disposable
  profile; never clear a person's service-worker, IndexedDB, or local space
  state.
- Use native `<dialog>` for modality and Escape. The cross-frame fix may add a
  composed-Tab guard, but must not replace the shared dialog or add a second
  modal primitive.
- A mobile actionable target must be at least `44px` in both dimensions. Keep
  the Join wordmark and input typography at their current visual scale.
- A registration retry reuses the already committed email receipt. It must not
  rerun address discovery, make the settled row editable, or admit concurrent
  WebAuthn ceremonies.
- Keep fresh spaces local-only until the existing share flow attaches sync.
  The share-state fix must not invent a remote or suppress the refusal.
- Seeded-view browser verification must create a fresh space after the change;
  an existing space can retain the prior seeded standard library.
- Do not fold the canonical-terms decision, cold-boot/preload work, subscription
  console cleanup, or the separate fresh-space runtime incident into these
  fixes.
- Before claiming browser completion, align the Nix ChromeDriver with installed
  Chrome 152. A compiled test archive is not browser execution.

## Failure coverage

| Confirmed failure | Resolution task |
| --- | --- |
| Registration does not restore the Settings or Hub opener | Task 1 |
| Registration cannot retry after WebAuthn rejects | Task 2 |
| Space-removal Tab focus escapes the sealed guest | Task 3 |
| Join wordmark and input are shorter than `44px` | Task 4 |
| Fresh-space progress reappears beside a settled refusal | Task 5 |

## File map

- `rust/tonk-portal/src/bridge.rs`: carry a guest focus-return token with a
  registration request and return focus through the same `MessagePort`.
- `rust/tonk-portal/src/lib.rs`: re-export the registration focus-return handle.
- `rust/tonk-ui/src/bin/ui.rs`: hand a portal-origin focus return to the
  top-page registration dialog.
- `rust/tonk-ui/src/register_dialog.rs`: own opener restoration, the committed
  email receipt, and retry behavior.
- `rust/tonk-ui/src/account.rs`: keep account-panel refresh event-driven rather
  than repainting unconditionally during dialog dismissal.
- `rust/tonk-ui/src/account_flow.rs`: real-browser regressions and an exact CDP
  mobile viewport helper.
- `rust/tonk-fab/src/dialog.rs`: keep Tab cycling inside the shared dialog's
  composed shadow/light-DOM focus order.
- `rust/tonk-workspace/src/ui_space_remove.rs`: exercise the real remove wrapper
  against the shared dialog behavior.
- `rust/tonk-core/assets/library/profile.yaml`: Join mobile hit geometry only.
- `rust/tonk-worker/tests/standard_library.rs`: seeded Join CSS contracts.
- `rust/tonk-display/src/element.rs`: prevent an obsolete async no-entity
  diagnostic from overwriting a newer entity frame.
- `plan/tonk-ui-mobile-hardening.md`: replace the five failure statuses only
  after their runtime regressions pass.

### Task 1: Restore registration focus across top-page and portal boundaries

**Files:**

- Modify: `rust/tonk-portal/src/bridge.rs:guest tonk.register, port dispatcher, on_register`
- Modify: `rust/tonk-portal/src/lib.rs:bridge exports`
- Modify: `rust/tonk-ui/src/bin/ui.rs:on_register installation`
- Modify: `rust/tonk-ui/src/register_dialog.rs:RETURN_FOCUS, open, close`
- Modify: `rust/tonk-ui/src/account.rs:registration close refresh path`
- Test: `rust/tonk-ui/src/account_flow.rs`
- Test: `rust/tonk-portal/src/bridge.rs`

**Interfaces:**

- Replace the portal callback with:

  ```rust
  pub struct RegisterFocusReturn {
      port: web_sys::MessagePort,
      token: String,
      handled: bool,
  }

  impl RegisterFocusReturn {
      pub fn restore(self);
  }

  pub fn on_register(
      handler: impl Fn(&str, Option<RegisterFocusReturn>) + 'static,
  );
  ```

- Add `register_dialog::open_with_return_focus(impl FnOnce() + 'static)` while
  retaining `open()` for top-page/service-worker callers that can use the
  current `document.activeElement`.
- `RegisterFocusReturn` owns cleanup: restoring posts the token back to the
  guest; dropping an unused handle removes the guest's stored token without
  focusing it.

- [x] Extend `it_scopes_registration_focus_and_restores_the_opener` to close
  the Settings dialog with Escape and assert `#account-choose-link` remains the
  active element after the account panel has had one asynchronous settle turn.
  Add `it_restores_registration_focus_to_the_guest_opener`: open registration
  from the provider-free Hub account trigger, close with Escape, return to the
  sealed frame, and assert that exact trigger—not merely the iframe—has focus.
- [x] Run
  `nix develop . -c cargo test -p tonk-ui --features integration-tests it_scopes_registration_focus_and_restores_the_opener -- --test-threads=1 --nocapture`
  and
  `nix develop . -c cargo test -p tonk-ui --features integration-tests it_restores_registration_focus_to_the_guest_opener -- --test-threads=1 --nocapture`;
  expect the current
  Settings assertion to settle on `BODY` and the Hub assertion to stop at the
  outer iframe.
- [x] In the injected guest bridge, capture `document.activeElement`
  synchronously when `tonk.register(reason)` is called, store it in a map under
  a minted opaque token, and include the token in the `register` envelope. Do
  not inspect or serialize guest selectors in the parent.
- [x] Pass the dispatching `MessagePort` into `handle_register`. Construct
  `RegisterFocusReturn` only when a non-empty token is present. Its `restore()`
  posts a `register-focus` envelope; the guest consumes the token, verifies the
  element is still connected and enabled, focuses it, and deletes the entry.
  An unused handle posts a discard envelope so repeated register asks do not
  leak element references.
- [x] In `ui.rs`, pass the returned handle to
  `open_with_return_focus`; registration requests without a guest token keep
  using `open()`. Preserve `describe(reason)` ordering after the dialog opens.
- [x] Remove `close()`'s unconditional `account::resettle()`. Account creation
  already dispatches `ACCOUNT_CHANGED`, which the mounted account element
  observes; a canceled ceremony changed no account state and must not repaint
  the opener out from under focus restoration. Keep the direct active-element
  restore for Settings and invoke the portal callback only after the native
  dialog is closed and removed.
- [x] Add bridge parser/round-trip tests rejecting empty tokens and proving a
  focus-return envelope is sent through the request's own port. Run
  `nix develop . -c test:web:debug -E 'package(tonk-portal)'`; expect the
  bridge tests to pass. Rerun the two focused Tonk UI commands above; expect
  both opener-restoration regressions to pass at runtime.
- [x] Rerun
  `nix develop . -c cargo test -p tonk-ui --features integration-tests it_explains_email_verification_before_account_sync -- --test-threads=1 --nocapture`;
  expect a completed
  account still refreshes the Settings panel through `ACCOUNT_CHANGED` without
  the dismissal-time repaint.

### Task 2: Retry WebAuthn with the committed registration address

**Files:**

- Modify: `rust/tonk-ui/src/register_dialog.rs:address, run_signup_ceremony, close`
- Test: `rust/tonk-ui/src/account_flow.rs`

**Interfaces:**

- Add `const COMMITTED_EMAIL_ATTR: &str = "data-register-email"` on the dialog
  host. `address()` reads the live input before commitment and this attribute
  afterward.
- The attribute lasts only for the dialog element's lifetime; closing/removing
  the host clears it without another global cell.

- [x] Add `it_retries_the_committed_address_after_a_failed_passkey_ceremony`.
  Use a disposable provider-free profile, resolve an available email, and stub
  `navigator.credentials.create` before the first action so each call rejects
  with a controlled `NotAllowedError`. Click the re-enabled action twice in
  sequence and assert: call count is two, the second click does not show “Enter
  the address you want to use,” the settled email row still names the original
  address, and each attempt is single-flight while pending.
- [x] Run
  `nix develop . -c cargo test -p tonk-ui --features integration-tests it_retries_the_committed_address_after_a_failed_passkey_ceremony -- --test-threads=1 --nocapture`;
  expect the first rejection to re-enable
  “create a passkey” and the second click to make no credential call because
  `address()` can no longer find `#tonk-register-email`.
- [x] After validating and trimming the address—but before
  `settle_named_row(EMAIL_ROW, ...)` removes the input—write the committed value
  to `COMMITTED_EMAIL_ATTR`. Make `address()` fall back to that attribute when
  the live input is absent. Do not recreate the input or rerun the availability
  lookup on retry.
- [x] Keep the existing `ACTION_PENDING` sequence unchanged: `begin_action()`
  claims the attempt; a retryable error calls `set_action(label, true)`; a
  second click/Enter in the same pending turn is rejected.
- [x] Rerun the retry command above, then run
  `nix develop . -c cargo test -p tonk-ui --features integration-tests it_begins_only_one_registration_action_per_offered_step -- --test-threads=1 --nocapture`;
  expect two
  sequential rejected ceremonies in the former and exactly one concurrent
  ceremony in the latter.

### Task 3: Keep shared-dialog Tab focus inside the sealed guest

**Files:**

- Modify: `rust/tonk-fab/src/dialog.rs:TonkDialog listeners and focus helpers`
- Modify test: `rust/tonk-workspace/src/ui_space_remove.rs:tests`
- Test: `rust/tonk-ui/src/account_flow.rs`

**Interfaces:**

- `tonk-dialog` remains a native modal. Add only a boundary guard for joint
  document focus navigation: forward Tab on the last composed focusable moves
  to the first; Shift+Tab on the first moves to the last.
- The ordered focusables are the shadow close button followed by visible,
  enabled light-DOM controls in composed slot order. A custom-element control
  focuses its internal `button`, `input`, or non-negative `tabindex` target.

- [x] Add a FABB Wasm regression that opens a real `<tonk-dialog>` with a body
  control and two slotted actions, focuses the final action, dispatches Tab,
  and expects the shadow close button; reverse with Shift+Tab. Extend the
  `ui-space-remove` test to use the same real close/cancel/remove sequence.
- [x] Add `it_keeps_space_removal_focus_inside_the_sealed_guest`: create a
  disposable local space, return to Hub, open its remove dialog, Tab for more
  steps than the dialog contains, and after every step assert the top document's
  active element remains the guest iframe and the guest active path belongs to
  the open `tonk-dialog`. Escape must close and restore the remove opener.
- [x] Run
  `nix develop . -c cargo test -p tonk-ui --features integration-tests it_keeps_space_removal_focus_inside_the_sealed_guest -- --test-threads=1 --nocapture`;
  expect current forward Tab from “remove
  space” to reach a top-shell control while the guest dialog remains open.
- [x] In `dialog.rs`, bind `keydown` on the native shadow dialog. Build the
  candidate list on each Tab press so disabled/hidden slotted actions are not
  cached. Filter `[hidden]`, disabled controls, negative `tabindex`, and nodes
  with no rendered box. Use `KeyboardEvent.composed_path()` to recognize the
  shadow close button and custom-element internals; prevent default only at the
  two wrap boundaries.
- [x] Preserve native Escape/cancel and ordinary Tab movement between interior
  controls. Do not mark the parent document inert: the guard exists because a
  native dialog cannot scope the joint tab order outside its iframe.
- [x] Run
  `nix develop . -c test:web:debug -E 'package(tonk-fab)'`,
  `nix develop . -c test:web:debug -E 'package(tonk-workspace)'`, and
  `nix develop . -c cargo test -p tonk-ui --features integration-tests it_keeps_space_removal_focus_inside_the_sealed_guest -- --test-threads=1 --nocapture`;
  expect all focus-cycle and restoration assertions to pass.

### Task 4: Give the Join wordmark and input real mobile targets

**Files:**

- Modify: `rust/tonk-core/assets/library/profile.yaml:join route compact CSS`
- Modify: `rust/tonk-worker/tests/standard_library.rs:mobile Join contracts`
- Modify test: `rust/tonk-ui/src/account_flow.rs:mobile viewport and target helpers`

**Interfaces:**

- Add a browser helper using CDP `Emulation.setDeviceMetricsOverride` with
  `{width, height, deviceScaleFactor: 2, mobile: true}` plus touch emulation, so
  Chrome's outer-window minimum cannot silently turn `320px` into `500px`.
- The target scanner reports every visible `a`, `button`, and non-hidden `input`
  whose width or height is below `44px`; it never uses the larger-dimension
  predicate.

- [x] Add `it_keeps_join_targets_accessible_at_phone_sizes`. Visit `/join` in a
  clean profile at `320x568x2` and `390x844x2`, enter the sealed guest, and
  assert exact `innerWidth`/`innerHeight`, no horizontal overflow, no undersized
  targets, and a `16px` computed share-link input font. Run
  `nix develop . -c cargo test -p tonk-ui --features integration-tests it_keeps_join_targets_accessible_at_phone_sizes -- --test-threads=1 --nocapture`;
  expect
  `.edge-mast` at about `98x32.8` and `.edge-input` at about `186x43` on the
  short viewport.
- [x] At `max-width:680px`, make `.edge-mast` a `44px`-minimum flex hit area
  centered around the unchanged `98px` wordmark. Do not enlarge the image.
- [x] Move the compact `.edge-field`'s 8px visual baseline inset from the field
  container onto the noun/cursor presentation, then stretch `.edge-input` to a
  `44px` minimum height inside the `44px` row. Preserve the right-aligned value,
  `16px` input font, cursor position, and total row width; do not rely on the
  enclosing label's rectangle as the input target.
- [x] Strengthen `it_declares_mobile_target_and_input_floors_for_hub_and_join`
  to require the wordmark and actual input floors, not only `.edge-field`.
  Run `cargo test -p tonk-worker --test standard_library`; expect the current
  seeded CSS to fail the two new contracts, then pass after the CSS change.
- [x] Rerun
  `nix develop . -c cargo test -p tonk-ui --features integration-tests it_keeps_join_targets_accessible_at_phone_sizes -- --test-threads=1 --nocapture`
  in light and dark schemes at both viewports; expect
  all targets at least `44x44`, no horizontal overflow, and unchanged wordmark
  dimensions.

### Task 5: Discard a stale no-entity diagnostic after a ready frame

**Files:**

- Modify: `rust/tonk-display/src/element.rs:handle_entity_frame, diagnose_no_entity`
- Test: `rust/tonk-display/src/element.rs`
- Test: `rust/tonk-ui/src/account_flow.rs:it_replaces_agent_link_progress_with_the_share_refusal`

**Interfaces:**

- Every entity frame already increments `Inner::entity_serial`. An async
  no-entity diagnostic may mutate DOM only while its captured serial is still
  current and the display is not disposed.
- `rust/tonk-core/assets/library/core.yaml` remains unchanged unless the guarded
  lifecycle test disproves this diagnosis; the current pending/refusal nesting
  is valid when `data-state` ordering is monotonic.

- [x] Add `it_discards_a_no_entity_diagnostic_superseded_by_a_ready_frame` in
  `element.rs`. Start an empty single-entity frame, capture its diagnostic
  serial, apply a non-empty frame that renders `ready`, then attempt to apply
  the old diagnostic. Assert the host remains `data-state="ready"`, its direct
  `slot="no-entity"` child stays hidden, and rendered content remains mounted.
- [x] Run
  `nix develop . -c test:web:debug -E 'test(it_discards_a_no_entity_diagnostic_superseded_by_a_ready_frame)'`;
  expect the current diagnostic path to restore `no-entity` because it has no
  generation check.
- [x] In the empty-frame branch, capture `entity_serial` and clone the shared
  `Inner` state before spawning `diagnose_no_entity`. Pass both into the async
  function. Immediately before either `set_absence` or
  `set_no_entity_diagnostic`, return without mutation when `disposed` is true
  or `entity_serial` differs. Keep the per-attribute queries cancellable only
  by this final generation check; they are reads and may finish harmlessly.
- [x] Do not paper over the race with a `:has(.local-invite-notice)` CSS rule.
  The observed DOM had a rendered refusal under a host reverted to
  `data-state="no-entity"`; generation ordering is the owning invariant and
  also protects every other display from stale diagnostics.
- [x] Run the new Wasm command above,
  `nix develop . -c test:web:debug -E 'test(it_keeps_nested_display_lifecycle_slots_scoped_to_their_owner)'`, and
  `nix develop . -c cargo test -p tonk-ui --features integration-tests it_replaces_agent_link_progress_with_the_share_refusal -- --test-threads=1 --nocapture`.
  The browser test must create a fresh space and see “sharing unavailable” with
  no visible “Generating link…”.

### Task 6: Close the five failures with fresh, separated evidence

**Files:**

- Modify after runtime success: `plan/tonk-ui-mobile-hardening.md:implementation status and evidence`
- Test: all files above

**Interfaces:**

- Produces five runtime-green regressions and an evidence record that does not
  equate compilation with browser execution.

**Verification result (2026-08-28):** All five focused WebDriver regressions
pass, as do the short destructive-dialog and exact-CDP Settings geometry
regressions found while running the broad gate. The canonical serialized
`test:e2e` run reached 51/57; its two UI failures were corrected afterward and
pass exactly. The four remaining failures are native CLI-backed account tests
whose `tonk.network` traffic resolves to public Cloudflare addresses instead
of the loopback harness, so the full-suite item below remains unchecked.

- [x] Run `cargo fmt --all -- --check` and `git diff --check`; expect no diff
  or whitespace errors.
- [x] Run `cargo test -p tonk-worker --test standard_library`,
  `cargo test -p tonk-workspace`, `cargo test -p tonk-fab`,
  `cargo test -p tonk-display`, and `cargo test -p tonk-portal`; expect all
  native suites to pass.
- [ ] With a ChromeDriver compatible with Chrome 152, run
  `nix develop . -c test:web:debug`; expect all Wasm browser tests to
  execute, not merely compile.
- [ ] Run
  `nix develop . -c cargo test -p tonk-ui --features integration-tests -- --test-threads=1 --nocapture`;
  expect the whole serialized browser suite to pass, including the five focused
  regressions.
- [ ] Run `env -u NO_COLOR nix develop . -c build:web`; serve that exact
  production artifact from a disposable profile and repeat the affected
  journeys at `320x568x2` and `390x844x2`, light and dark. Also verify normal
  and OS-level reduced motion if the automation environment can genuinely set
  the media query.
- [ ] Inspect the affected journeys' console and network output. Record
  unrelated preload/subscription messages under their existing incident plans;
  do not suppress them or misclassify them as one of these five fixes.
- [x] Only after the runtime commands pass, update Tasks 2, 3, 4, 5, and 9 in
  `plan/tonk-ui-mobile-hardening.md` from “Runtime defect found/failing” to
  verified, with the exact commands and date. Leave iOS Safari, real passkey,
  valid-invite, reduced-motion, terms, and production-device manual checks open
  unless they were actually performed.

## Handoff order

Tasks 1-5 are independently reviewable and can be implemented as focused
commits. Task 1 changes the portal callback interface and should be completed
within one commit across `tonk-portal` and `tonk-ui`. Task 3 deliberately fixes
the generic dialog primitive, so its FABB and workspace regressions must land
together. Task 6 runs after every focused task is green; no status may be marked
verified from compilation alone.
