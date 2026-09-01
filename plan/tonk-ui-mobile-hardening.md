# Tonk UI mobile hardening implementation plan

**Goal:** Make the audited Tonk UI journeys usable, internally consistent, and
accessible at phone widths without changing the approved compact FABB design or
folding runtime/performance incidents into presentation work.

**Approach:** Fix each observable behavior at the component that owns it, with a
focused failing browser or Wasm test before production changes. Prefer platform
modal and form semantics over hand-built approximations, keep desktop geometry
unchanged unless the same correctness defect applies there, and finish with
clean-profile checks at both audited phone sizes.

**Pinned source:** `498bca7e2` on `fix/harden-mobile`, audited 2026-08-28.

## Implementation status (2026-08-28)

The source work for Tasks 1-9 and 11 is implemented in this worktree. Task 10
remains intentionally blocked by the canonical-terms decision gate, so
`SIGNUP_TERMS = "2026-08"` and the unlinked activation copy are unchanged.

| Task | Status | Current evidence |
| --- | --- | --- |
| 1 | Runtime verified | Hub, Join, Activation, Settings, registration, and space-removal surfaces stay inside the `320x568` and `390x844` viewports. The activated-account destructive dialog keeps both actions visible and focusable at both audited sizes. |
| 2 | Runtime verified | Registration is a native modal with initial focus/narration and Escape dismissal. Focus returns to the exact Settings control and crosses the sealed Hub boundary back to the exact guest opener. |
| 3 | Runtime verified; Storybook pending | Registration retries the committed address after sequential WebAuthn rejection while same-turn registration and activation submissions remain single-flight. Storybook `B-05` remains unverified. |
| 4 | Runtime verified | Repeated Tab stays within the composed shared-dialog order in the sealed Hub; Escape closes the native dialog and restores the remove opener. |
| 5 | Runtime verified | At exact CDP `320x568x2` and `390x844x2` viewports, Join's wordmark/input and every visible actionable target meet the `44x44` floor without horizontal overflow. |
| 6 | Chromium verified; Safari unavailable | Editable inputs compute to `16px` in Chromium. Real iOS Safari visual-viewport zoom remains unverified because no device or simulator is available. |
| 7 | Runtime emulation unavailable | Normal-motion computed styles were inspected, but both isolated Chrome reduced-motion launch flags were ignored (`matchMedia` stayed false). |
| 8 | Partially browser-verified | Compact FABB disclosure naming, focus, Escape restoration, dark/light pressed state, and `44x44` controls pass. The attached-account Hub menu remains unreachable without completing WebAuthn registration. |
| 9 | Runtime verified | A freshly created local space replaces `GENERATING LINK…` with the `sharing unavailable` refusal; the stale async no-entity diagnostic cannot overwrite a newer ready frame. |
| 10 | Blocked by product/legal input | No canonical, versioned terms document was supplied; no legal copy or recorded version was invented. |
| 11 | Implemented and smoke-tested | A fresh disposable-snapshot `build:web` succeeds; `dev:web` served the isolated browser run after unsetting the inherited incompatible `NO_COLOR=1`. |
| 12 | Focused runtime verified; full gate limited | ChromeDriver `152.0.7977.65` matches Chrome 152. All five focused WebDriver regressions, the destructive-dialog and exact-CDP settings geometry checks, adjacent retry/single-flight checks, focused Wasm tests, native suites, and the direct production build pass. The full-gate limitations below remain. |

The repository `test:web:debug` and `build:web` wrappers evaluate an inner
`git+file:` flake, which omits the untracked `ui_space_remove.rs` source and
fails with `E0583`; the direct `path:.` production build passes. The equivalent
direct full Wasm workspace suite compiled and executed in Chrome, but one
pre-existing fallback-observer test failed in the full order and passed
immediately in isolation. These are recorded as gate limitations rather than
claimed green.

The canonical serialized `test:e2e` run reached 51/57. Its two non-CLI UI
failures were then corrected and pass exactly: initial registration focus now
waits for the dialog's deferred focus task, and compact Settings geometry uses
the exact CDP phone viewport plus a real `44px` width floor. The four remaining
failures are CLI-backed account-link/backup/revocation cases: their native CLI
processes resolve `tonk.network` to public Cloudflare addresses instead of the
loopback harness, so the full-suite checkbox remains open.

### Fresh verification evidence (2026-08-28)

- `cargo fmt --all -- --check`, `git diff --check`, Storybook generation/link
  checks, Wasm checks, integration-test compilation, and `build:web` pass.
- Native tests pass: `tonk-worker` standard library 15, `tonk-workspace` 18,
  `tonk-fab` 109, and `tonk-display` 73.
- The isolated Chromium matrix covered Hub, Join, Activation, Settings,
  registration, space removal, fresh-space creation, FABB overflow, light/dark
  appearance, horizontal overflow, target geometry, input font size, focus,
  Escape, and held-request duplicate-submission probes.
- Runtime failures closed: registration restores both opener kinds; a failed
  WebAuthn ceremony retries the committed address; space-removal Tab focus stays
  inside the sealed native dialog; Join meets the target floor; and a fresh
  space renders only the settled refusal after it replaces pending progress.
- Manual-only coverage remains: real iOS Safari focus/keyboard/safe-area
  behavior, real passkey account lifecycle and destructive account dialog,
  valid invitation completion, genuine OS reduced motion, and the production
  artifact's device matrix. Task 10 still requires the canonical terms document.

**Constraints:**

- The fresh-space runtime failure is owned by
  `plan/fresh-space-runtime-panic.md`; do not hide its console failures in this
  work or treat a still-visible canvas as recovery.
- Cold-boot bundle and request work is owned by
  `plan/mobile-cold-boot-audit.md`; Web Awesome, PostHog, Wasm splitting, and
  bundle budgets are outside this plan.
- Preserve the approved fit-driven compact FABB behavior in
  `plan/fabb-mobile.md`: `44px` compact cells, overflow behavior, safe-area
  docking, and intentional collapse to the sync disc.
- Preserve local/offline state. Browser verification uses disposable profiles;
  it must not clear a person's normal Tonk storage.
- Do not add a UI framework, motion library, icon package, or a second modal
  primitive.
- Use native `<dialog>` for modal focus/inert/Escape behavior. The sealed guest
  already registers `<tonk-dialog>` through `tonk-fab`; the top-page
  registration ceremony uses its own native `<dialog>` because it does not load
  the guest component tree.
- `44px` means both dimensions of an actionable target unless the visible
  control is a checkbox/radio inside a label whose hit rectangle is at least
  `44x44`.
- Mobile text-entry controls use a computed font size of at least `16px` to
  avoid iOS focus zoom. Surrounding labels and display text keep the existing
  type scale.
- Keep `SIGNUP_TERMS = "2026-08"` unchanged unless the canonical terms
  document names a different version. Legal copy is external product content,
  not something an implementer should invent.
- Generated Storybook files remain generated. Read `docs/storybook/README.md`
  and `docs/storybook/goal.md` before changing triage, then run the commands
  required by `docs/storybook/AGENTS.md`.
- Use `path:.` for Nix checks so the build sees uncommitted plan implementation
  files.

## Finding coverage

| Audited finding | Planned work |
| --- | --- |
| Short-phone destructive account dialog clips its footer | Task 1 |
| `100vh` surfaces ignore dynamic mobile browser chrome | Task 1 |
| Registration overlay lacks modal focus/inert/Escape semantics | Task 2 |
| Registration narrator initially renders as a blank block | Task 2 |
| Registration and activation accept duplicate async submissions | Task 3 |
| Hub removal uses labels/radios as an inaccessible modal mechanism | Task 4 |
| Hub/account/join actions have inconsistent sub-44px targets | Task 5 |
| Existing mobile target test checks the larger dimension | Task 5 |
| Inputs below `16px` can trigger iOS focus zoom | Task 6 |
| Registration transitions/cursors ignore reduced motion | Task 7 |
| FABB advertises menus but implements disclosure/Tab behavior | Task 8 |
| Hub account menu lacks menu keyboard behavior | Task 8 |
| New-space share state shows “Generating link” beside a refusal | Task 9 |
| Share refusal copy points to a condition banner that may not exist | Task 9 |
| Activation claims terms acceptance without a reachable document | Decision gate and Task 10 |
| Tonk UI README names nonexistent `nix run` apps | Task 11 |

## File map

- `rust/tonk-ui/src/account.css`: account/activation mobile geometry, touch
  sizes, input font size, and short-viewport modal bounds.
- `rust/tonk-ui/styles.css`: top-page registration ceremony geometry, motion,
  touch/input rules, and board dynamic viewport sizing.
- `rust/tonk-ui/src/register_dialog.rs`: registration native-dialog lifecycle,
  initial narration, focus restoration, and one-submit gate.
- `rust/tonk-ui/src/activate.html`: activation narration, terms link, and
  accessible button/status relationship.
- `rust/tonk-ui/src/activate.rs`: activation one-submit state and terminal
  result ordering.
- `rust/tonk-ui/Cargo.toml`: `web-sys` support for `HtmlDialogElement`.
- `rust/tonk-ui/src/account_flow.rs`: real-browser regressions at `320x568`
  and `390x844`, including corrected target-size checks.
- `rust/tonk-core/assets/library/profile.yaml`: Hub/remove/join markup and
  phone CSS.
- `rust/tonk-core/assets/library/core.yaml`: blank-canvas pending/refusal
  lifecycle and refusal copy.
- `rust/tonk-workspace/src/ui_space_remove.rs`: Hub remove-dialog opener and
  modal lifecycle.
- `rust/tonk-workspace/src/lib.rs`: register `<ui-space-remove>`.
- `rust/tonk-workspace/src/ui_hub_account.html`: Hub account menu semantics.
- `rust/tonk-workspace/src/ui_hub_account.rs`: menu focus and arrow/Home/End
  keyboard behavior.
- `rust/tonk-fab/src/dialog.rs`: compact native-dialog target sizing used by
  Hub removal.
- `rust/tonk-fab/src/markup.rs`: FABB disclosure names and overflow-mode state.
- `rust/tonk-fab/src/menu.rs`: disclosure group semantics and propagation.
- `rust/tonk-fab/src/mi.rs`: reflect overflow appearance state onto its real
  shadow button.
- `rust/tonk-fab/src/bar.rs`: update reflected appearance pressed state.
- `rust/tonk-fab/src/field.rs`: compact input target and iOS-safe font sizing.
- `rust/tonk-fab/tests/responsive_overflow.rs`: FABB accessibility and compact
  control regression coverage.
- `rust/tonk-display/src/state.rs`: nested lifecycle projection regression if
  the real blank-canvas test localizes the stale pending label here.
- `rust/tonk-worker/src/router/create_invite.rs`: complete user-facing refusal
  detail when the worker owns the remedy.
- `rust/tonk-worker/src/router/repository.rs`: share/refusal behavior and seeded
  standard-library assertions.
- `rust/tonk-worker/tests/standard_library.rs`: Hub markup and mobile CSS
  contracts.
- `rust/tonk-ui/assets/service_worker.js`: dynamic-viewport sizing for the
  service-worker fatal-error document.
- `rust/tonk-ui/README.md`: correct repository build and development commands.
- `docs/storybook/bug-triage.md`: mark `B-05` fixed only after its duplicate
  submission journey passes.

### Task 1: Keep full-height surfaces and destructive controls inside the visible viewport

**Files:**

- Modify: `rust/tonk-ui/src/account.css:.account__dialog and mobile media rules`
- Modify: `rust/tonk-ui/styles.css:.board-view`
- Modify: `rust/tonk-core/assets/library/profile.yaml:join/route-view CSS`
- Modify: `rust/tonk-ui/assets/service_worker.js:fatal error document CSS`
- Modify: `rust/tonk-ui/src/account_flow.rs`
- Test: `rust/tonk-ui/src/account_flow.rs`
- Test: `rust/tonk-worker/tests/standard_library.rs`

**Interfaces:**

- Consumes: the existing fixed `.account__dialog`, its internal scrolling
  surface, and the current `320x568` destructive confirmation markup.
- Produces: a dialog whose top and bottom are bounded by `100dvh` plus safe-area
  insets, and full-height surfaces that use `100vh` only as an older-browser
  fallback immediately followed by `100dvh`.

- [ ] Add
  `it_keeps_destructive_dialog_controls_inside_a_short_mobile_viewport` to
  `account_flow.rs`: open the account deletion confirmation at `320x568`,
  expose its longest arming state, scroll `.account__dialog` to its maximum,
  and assert `dialog.top >= 0`, `dialog.bottom <= innerHeight`, and both Cancel
  and the destructive submit button are visible and focusable. Run
  `nix develop path:. -c cargo test -p tonk-ui --features integration-tests it_keeps_destructive_dialog_controls_inside_a_short_mobile_viewport -- --test-threads=1 --nocapture`;
  expect the current bottom near `604px` to exceed the `568px` viewport.
- [ ] Restrict the current `top: max(12vh, safe-area + 48px)` phone placement
  to viewports taller than `700px`. For `max-height:700px`, place the dialog at
  `max(16px, env(safe-area-inset-top))`, calculate its maximum height from
  `100dvh` minus that top inset and a `16px` bottom inset, and retain vertical
  scrolling with horizontal clipping.
- [ ] Replace the remaining product-owned `100vh` uses with an ordered fallback:
  `100vh` first, `100dvh` second. Apply this to `.board-view`, `.join-view`, and
  the service-worker fatal-error document; do not edit vendored Web Awesome
  CSS.
- [ ] Add a standard-library assertion that `.join-view` contains the dynamic
  viewport declaration. Run
  `cargo test -p tonk-worker --test standard_library`; expect success after the
  CSS change.
- [ ] Rerun the focused browser test at both `320x568` and `390x844`; expect all
  controls inside the viewport with no horizontal overflow.

### Task 2: Make registration a real modal with useful initial guidance

**Files:**

- Modify: `rust/tonk-ui/Cargo.toml:web-sys features`
- Modify: `rust/tonk-ui/src/register_dialog.rs:DIALOG_HTML, open, open_when_upgraded, close`
- Modify: `rust/tonk-ui/styles.css:.tonk-ceremony modal styles`
- Modify: `rust/tonk-ui/src/account_flow.rs`
- Test: `rust/tonk-ui/src/account_flow.rs`

**Interfaces:**

- Consumes: `register_dialog::open()`, `close()`, `describe()`, and the stable
  `#tonk-register-*` selectors used by account/share flows.
- Produces: a native `<dialog id="tonk-register">` shown with `showModal()`,
  with the same inner selectors, native focus containment/background inertness,
  Escape cleanup, and focus restoration to the element active before `open()`.

- [ ] Add `it_scopes_registration_focus_and_restores_the_opener`: open the
  registration ceremony from Settings, assert the initial status text is
  non-empty, press Tab through more steps than the dialog has focusable
  controls and assert focus never leaves `#tonk-register`, press Escape and
  assert the dialog is removed, then assert focus returns to the original
  opener. Also use a WebDriver pointer click to assert a background action is
  not interactable while the modal is open. Run the focused integration test; expect failure
  because the current fixed `<div role="dialog">` does not make the background
  inert, trap focus, or close on Escape.
- [ ] Add `HtmlDialogElement` to `rust/tonk-ui`'s `web-sys` features. Have
  `open()` capture `document.active_element`, create the native dialog host,
  append the existing ceremony contents, wire `cancel` to `preventDefault()`
  plus `close()`, and call `show_modal()` after the current deferred layout
  turn. Keep the stable descendant IDs so existing WebAuthn and account tests
  do not need selector churn.
- [ ] Replace `.tonk-dim` with `#tonk-register::backdrop`; remove the nested
  `role="dialog"`/`aria-modal` duplication. Override Web Awesome's native
  dialog defaults on this host so the current block-stack geometry and light/
  dark tokens remain unchanged.
- [ ] Initialize `#tonk-register-status` with: “Enter your email address. We’ll
  tell you whether to create a passkey or sign in.” Add
  `aria-describedby="tonk-register-status"` to the dialog, while retaining the
  status element's `aria-live="polite"` updates.
- [ ] In `close()`, close the native dialog before removing it, clear the
  subscription/delegates as today, and focus the captured opener only if it is
  still connected and enabled.
- [ ] Rerun the focused test plus
  `nix develop path:. -c cargo test -p tonk-ui --features integration-tests it_explains_email_verification_before_account_sync -- --test-threads=1 --nocapture`;
  expect both to pass without changing the passkey flow.

### Task 3: Make registration and activation submissions monotonic

**Files:**

- Modify: `rust/tonk-ui/src/register_dialog.rs:thread-local state, action_is_offered, submit, set_action, close`
- Modify: `rust/tonk-ui/src/activate.rs:TonkActivate and bind`
- Modify: `rust/tonk-ui/src/activate.html:#activate-accept`
- Modify: `rust/tonk-ui/src/account_flow.rs`
- Modify: `docs/storybook/bug-triage.md:B-05`
- Test: `rust/tonk-ui/src/account_flow.rs`

**Interfaces:**

- Produces in `register_dialog.rs`:

  ```rust
  thread_local! {
      static ACTION_PENDING: Cell<bool> = const { Cell::new(false) };
  }

  fn begin_action() -> bool;
  fn finish_action();
  ```

- Produces in `activate.rs`: per-element `Rc<Cell<bool>>` submission state;
  no global activation gate, so independent activation tabs remain independent.

- [ ] Add a registration browser regression that resolves an actionable email,
  dispatches click and Enter in the same JavaScript turn, and asserts only one
  passkey ceremony/registration request begins. Add an activation regression
  that replaces `window.fetch` for `/ucan/` with a held promise, double-clicks
  `#activate-accept`, and asserts a request count of exactly one before settling
  the response. Run each focused test; expect duplicate work on current code.
- [ ] In registration, make `begin_action()` atomically reject a second action,
  immediately set the real button's `disabled` property and `aria-busy=true`,
  and make `action_is_offered()` require visible and not disabled. Make every
  retryable error and every newly offered step call `set_action(label, true)`,
  which clears the gate and attributes. `close()` also clears the gate. Audit
  the early returns in signup, login, copy-link, and return-to-space paths so no
  failed prerequisite leaves the singleton permanently busy.
- [ ] In activation, check and set the per-element gate before `spawn_local`,
  immediately disable `#activate-accept`, and set `aria-busy=true` on the
  owning section. Successful activation is terminal: later responses cannot
  replace the done panel. Re-enable only after a retryable network/service
  failure; an expired/unauthorized one-use link stays disabled and points the
  user back to the device for a fresh link.
- [ ] Run both focused tests in both response orders (success then unauthorized,
  unauthorized then success); expect one request and one final state.
- [ ] After the runtime test passes, update `B-05` to `Fixed and verified` with
  the current commit/test evidence. From `docs/storybook`, run
  `python3 scripts/build.py --check` and `python3 scripts/check-links.py .`;
  expect both to pass.

### Task 4: Replace the Hub's radio-driven remove overlay with a native dialog

**Files:**

- Create: `rust/tonk-workspace/src/ui_space_remove.rs`
- Modify: `rust/tonk-workspace/src/lib.rs`
- Modify: `rust/tonk-core/assets/library/profile.yaml:Hub removal CSS and markup`
- Modify: `rust/tonk-fab/src/dialog.rs:compact target geometry`
- Modify: `rust/tonk-worker/tests/standard_library.rs`
- Test: `rust/tonk-workspace/src/ui_space_remove.rs`
- Test: `rust/tonk-ui/src/account_flow.rs`

**Interfaces:**

- Produces this seeded-view contract:

  ```html
  <ui-space-remove>
    <button type="button" data-space-remove-open>remove</button>
    <tonk-dialog data-space-remove-dialog heading="confirm space removal">
      <form id="remove-{subject}" onsubmit="space/remove" data-remove="{subject}">
        <!-- warning copy -->
      </form>
      <button slot="actions" type="button" data-dialog="close">cancel</button>
      <button slot="actions" type="submit" form="remove-{subject}">remove space</button>
    </tonk-dialog>
  </ui-space-remove>
  ```

- `ui-space-remove` calls the existing `<tonk-dialog>.show()` property API; it
  does not own removal, duplicate the `space/remove` command, or add storage.

- [ ] Add a Wasm test that mounts `<ui-space-remove>` and a registered
  `<tonk-dialog>`, activates the real button with Enter, and asserts the inner
  native dialog is open; Escape and Cancel must close it and restore focus to
  the remove button. Add a standard-library test rejecting `.rm-radio`,
  label-based open/close controls, `.mscrim`, and the hand-authored
  `role="alertdialog"`. Run the focused tests; expect failure on current markup.
- [ ] Implement `ui_space_remove.rs` as a small custom element with one retained
  click listener. On `[data-space-remove-open]`, call the descendant dialog's
  `show()` function through `Reflect`; leave the existing form submit event to
  `tonk-display` and `RemoveSpaceHandler`. Register the element in
  `tonk-workspace::register()`.
- [ ] Replace the hidden radios, labels, scrim, and modal markup with the
  contract above. Keep the current warning text and destructive command data.
  Do not close optimistically on submit: the repeated row disappearing after
  successful removal should remove its dialog; a failed command must not look
  successful.
- [ ] Give `<tonk-dialog>` header close and slotted actions `44px` minimum
  targets at `max-width:519px`, without changing its desktop `36px` block law.
- [ ] Run `nix develop path:. -c test:web:debug -E 'package(tonk-workspace)'`
  and `nix develop path:. -c test:web:debug -E 'package(tonk-fab)'`; expect the
  modal, Escape, and focus tests to pass.

### Task 5: Enforce a real 44-by-44 mobile action target floor

**Files:**

- Modify: `rust/tonk-core/assets/library/profile.yaml:Hub and join mobile CSS`
- Modify: `rust/tonk-ui/src/account.css:phone action geometry`
- Modify: `rust/tonk-ui/styles.css:registration phone geometry`
- Modify: `rust/tonk-ui/src/account_flow.rs:mobile target assertions`
- Modify: `rust/tonk-worker/tests/standard_library.rs`
- Test: `rust/tonk-ui/src/account_flow.rs`

**Interfaces:**

- Produces a shared browser-test predicate:

  ```javascript
  const tooSmall = ({ width, height }) => width < 44 || height < 44;
  ```

- Checkbox/radio inputs are measured through their enclosing label; hidden
  inputs and non-interactive status elements are excluded.

- [ ] Replace both current `Math.max(rect.width, rect.height) < 44` checks in
  `account_flow.rs` with the two-dimensional predicate. Add a route matrix for
  Hub, Settings, activation, registration, and join at `320x568` and `390x844`.
  Report selector, width, and height for every failure. Run the focused test;
  expect the current `36px` rows/actions to be reported.
- [ ] At `max-width:640px`, make Hub header cells, account-menu rows, space rows,
  empty/create rows, and remove controls at least `44px` high. Keep visual glyphs
  and text baselines optically seated with padding; do not scale icons to fill
  the target.
- [ ] At `max-width:680px`, make `.edge-field`, `.ebtn`, and the actual button
  inside `.ebtn.solid` at least `44px` high. Preserve the existing stacked
  mobile run.
- [ ] At `max-width:463px`, give actionable Settings/activation rows, tabs,
  buttons, links, inputs, and dialog actions a `44px` target. A `20px` checkbox
  remains visually `20px`; its `.account__confirm-check` label owns the target.
- [ ] At `max-width:519px`, make registration header, input row, action row,
  and dismiss control at least `44px` high. Preserve the desktop `36px` rhythm.
- [ ] Add standard-library assertions for the Hub/join mobile target contract,
  then run `cargo test -p tonk-worker --test standard_library` and the focused
  browser matrix; expect no undersized target and no horizontal overflow.

### Task 6: Prevent iOS focus zoom without enlarging all mobile typography

**Files:**

- Modify: `rust/tonk-ui/src/account.css:mobile input rules`
- Modify: `rust/tonk-ui/styles.css:.tonk-ceremony .ed mobile rule`
- Modify: `rust/tonk-core/assets/library/profile.yaml:.edge-value mobile rule`
- Modify: `rust/tonk-fab/src/field.rs:mobile .value rule`
- Modify: `rust/tonk-ui/src/account_flow.rs`
- Modify: `rust/tonk-fab/tests/edge_primitives.rs`
- Test: `rust/tonk-ui/src/account_flow.rs`
- Test: `rust/tonk-fab/tests/edge_primitives.rs`

**Interfaces:**

- Produces `font-size:16px` only on editable native inputs at phone widths or
  coarse pointers; noun labels, settled values, and desktop inputs retain the
  existing condensed scale.

- [ ] Add computed-style assertions at `390px` for registration email/name,
  account display-name and confirmation email, join URL, and `<tonk-field>`'s
  shadow input. Assert each editable input is at least `16px`, then assert the
  same controls keep their current desktop size at `1200px`. Run the focused
  tests; expect mobile failures at `13px`/`13.5px`.
- [ ] Add narrow/coarse media rules for only `.tonk-ceremony .ed`, account text
  inputs, `.edge-input`, and `tonk-field .value`. Increase row padding/height as
  needed to retain baseline alignment; do not use transforms or page-wide
  `text-size-adjust` suppression as a substitute.
- [ ] Run the focused browser and FABB Wasm tests; expect mobile computed input
  sizes of `16px` and unchanged desktop typography.
- [ ] Manually verify one real iOS Safari focus on registration, join, and
  Settings. Record as unverified if no iOS device/simulator is available;
  Chromium computed styles do not prove Safari will avoid visual-viewport zoom.

### Task 7: Honor reduced motion in the registration ceremony

**Files:**

- Modify: `rust/tonk-ui/styles.css:registration reduced-motion rules`
- Modify: `rust/tonk-ui/src/account_flow.rs`
- Test: `rust/tonk-ui/src/account_flow.rs`

**Interfaces:**

- Consumes: `.tonk-cluster`, `.orow`, `.obtn`, `.cur`, `.obtn.wait`, and
  `.flash` animation/transition declarations.
- Produces: zero-duration disclosure/dim transitions and no cursor, wait, or
  flash animation when `prefers-reduced-motion: reduce` is active.

- [ ] Add a browser check that emulates `prefers-reduced-motion: reduce`, opens
  registration, and reads computed animation/transition duration and name for
  every selector above. Run it; expect current registration cursor/wait/flash
  animation and `.orow` transition failures.
- [ ] Add one registration-specific reduced-motion media block that sets
  transitions to `none` on the modal/rows/actions and animations to `none` on
  cursor, wait, and flash states. Keep existing account, join, Hub, and FABB
  reduced-motion rules intact.
- [ ] Rerun the focused test in normal and reduced modes; normal mode retains
  the authored motion, reduced mode reports none.

### Task 8: Make menu and disclosure semantics match their keyboard behavior

**Files:**

- Modify: `rust/tonk-fab/src/markup.rs:BAR_HTML and STACKS_HTML`
- Modify: `rust/tonk-fab/src/menu.rs:connected_callback and semantics`
- Modify: `rust/tonk-fab/src/mi.rs:observed attributes and sync`
- Modify: `rust/tonk-fab/src/bar.rs:update overflow mode state`
- Modify: `rust/tonk-fab/tests/responsive_overflow.rs`
- Modify: `rust/tonk-workspace/src/ui_hub_account.html`
- Modify: `rust/tonk-workspace/src/ui_hub_account.rs:open_menu and keydown`
- Test: `rust/tonk-fab/tests/responsive_overflow.rs`
- Test: `rust/tonk-workspace/src/ui_hub_account.rs`

**Interfaces:**

- FABB stacks become named disclosure groups: trigger buttons keep
  `aria-expanded` and `aria-controls`, but remove `aria-haspopup`; each
  `<tonk-menu>` host carries `role="group"` plus a concrete `aria-label`, and
  its shadow rows remain ordinary Tab-reachable buttons.
- `<tonk-mi pressed="true|false">` reflects `aria-pressed` onto its real shadow
  `.row` button. The host itself does not claim `menuitemcheckbox`.
- The Hub keeps its genuine `role="menu"` contract and gains ArrowDown,
  ArrowUp, Home, End, Escape, initial-focus, and focus-restoration behavior.

- [ ] Add FABB accessibility assertions that no trigger advertises a menu, each
  controlled group has a name, all actions remain buttons in DOM/focus order,
  and the appearance action exposes `aria-pressed` on its actual button. Add
  Hub Wasm tests for initial focus, wrapping arrows, Home/End, Escape, and
  focus restoration. Run the package Wasm tests; expect failures against the
  mixed current semantics.
- [ ] Remove FABB `aria-haspopup` and `menuitemcheckbox`, name every stack
  (`space actions`, `spaces`, `share actions`, `more actions`), and set group
  roles. Extend `TonkMi` to observe `pressed`, copy it to the shadow button's
  `aria-pressed`, and have `bar.rs` update `pressed` with the resolved theme.
- [ ] In `ui_hub_account.rs`, focus the first enabled menu item from
  `open_menu()`. Implement a visible menuitem list and wrap ArrowDown/ArrowUp;
  Home/End go to endpoints, Escape closes and restores trigger focus, and Tab
  closes without preventing the browser's normal focus move. Do not add roving
  semantics to FABB disclosures.
- [ ] Run
  `nix develop path:. -c test:web:debug -E 'package(tonk-fab)'` and
  `nix develop path:. -c test:web:debug -E 'package(tonk-workspace)'`; expect
  all keyboard and accessibility assertions to pass.

### Task 9: Make blank-space share progress and refusal mutually exclusive

**Files:**

- Modify: `rust/tonk-core/assets/library/core.yaml:blank-canvas agent-link and share/blocked view`
- Modify: `rust/tonk-worker/src/router/create_invite.rs:RemoteRefusal::detail if required by the failing copy cases`
- Modify: `rust/tonk-worker/src/router/repository.rs:seeded-library tests`
- Modify if localized here: `rust/tonk-display/src/state.rs:update_slot_children and nested state test`
- Modify: `rust/tonk-ui/src/account_flow.rs`
- Test: `rust/tonk-ui/src/account_flow.rs`
- Test: `rust/tonk-display/src/state.rs`
- Test: `rust/tonk-worker/src/router/repository.rs`

**Interfaces:**

- The blank canvas may show “Generating link…” only while no invite or refusal
  result is available. A ready `tonk:share/blocked` view hides it in the same
  frame.
- The refusal view displays a neutral “sharing unavailable” label plus exactly
  the worker-owned `{detail}` sentence. It does not append the unconditional
  “Use connect in the condition banner” instruction.

- [ ] Add `it_replaces_agent_link_progress_with_the_share_refusal`: create a
  local-only fresh space without a registered provider, trigger sharing, wait
  for `.local-invite-notice`, and assert the visible canvas contains exactly one
  state—no “Generating link…” and no reference to a nonexistent condition
  banner. Run the focused integration test; expect both current strings to be
  visible.
- [ ] Add a nested `tonk-display` lifecycle test that makes the inner display
  move from `no-entity` to `ready` and asserts both the `hidden` attribute and
  computed `display:none` on its pending slot. Use this result to localize the
  fix: if it fails, correct direct-child projection in `state.rs`; if it passes,
  keep `tonk-display` unchanged and repair the seeded blank-canvas markup/CSS.
  Do not apply both fixes without evidence.
- [ ] Remove the fixed banner instruction from the refusal view and use a
  state-neutral label. Make every worker detail used here a complete sentence;
  retain existing specific instructions such as confirming the email, and do
  not tell suspended/unshareable accounts to connect.
- [ ] Update `it_routes_refused_agent_links_to_the_local_only_notice` to require
  mutual exclusion and reject the stale copy. Run
  `cargo test -p tonk-worker it_routes_refused_agent_links_to_the_local_only_notice`
  and `nix develop path:. -c test:web:debug -E 'package(tonk-display)'`; expect
  success before rerunning the whole browser journey.

## Decision gate: canonical terms content

The repository contains no terms document, while `activate.html` says the user
accepts one and `tonk-access-service` records version `2026-08`. Implementation
must pause this finding until the product/legal owner supplies a canonical,
stable terms document and confirms that its version is `2026-08` (or explicitly
authorizes a coordinated version migration). Do not invent the document, link
to a generic home page, or remove recorded acceptance as a UI cleanup.

### Task 10: Link the activation decision to the approved terms version

**Files:**

- Modify after the decision gate is satisfied: `rust/tonk-ui/src/activate.html`
- Modify if the approved version differs: `rust/tonk-access-service/src/registration.rs:SIGNUP_TERMS`
- Modify if the approved version differs: `rust/tonk-access-service/tests/registration.rs`
- Modify: `rust/tonk-ui/src/account_flow.rs`
- Test: `rust/tonk-ui/src/account_flow.rs`
- Test if the version changes: `rust/tonk-access-service/tests/registration.rs`

**Interfaces:**

- The exact linked document version and `SIGNUP_TERMS` value are one reviewed
  release contract.
- The activation action's accessible description contains the linked “terms
  of service” phrase before a user can accept.

- [ ] Once canonical content is supplied, add an activation browser test that
  finds the terms link by accessible name, asserts a non-placeholder absolute
  HTTPS URL or an existing same-origin document, opens it without losing the
  activation URL, and confirms the served document identifies the same version
  recorded by `SIGNUP_TERMS`. Run it; expect failure while no link/document
  exists.
- [ ] Link the existing phrase rather than adding a second legal sentence. Use
  `target="_blank" rel="noopener"` for a cross-origin document; use ordinary
  same-tab navigation only if the approved same-origin page preserves a safe
  return to the one-use activation URL.
- [ ] If and only if the approved version differs, update `SIGNUP_TERMS` and
  its storage/registration assertions in the same change. Run
  `cargo test -p tonk-access-service registration` and the focused browser
  test; expect the visible document and stored version to agree.

### Task 11: Correct the Tonk UI build instructions

**Files:**

- Modify: `rust/tonk-ui/README.md:Build and run`

**Interfaces:**

- Produces these repository-defined commands:

  ```sh
  nix develop . -c build:web
  nix develop . -c dev:web
  ```

- [ ] Replace the two `nix run .#...` examples; retain the explanation that
  `build:web` runs `nix build .#tonk-ui` and `dev:web` serves Trunk with the
  access-service proxies.
- [ ] Run `nix develop path:. -c build:web`; expect a successful production
  artifact. Start `nix develop path:. -c dev:web`, wait for its printed local
  URL, request the root and `/.well-known/tonk`, then stop it cleanly; expect
  both endpoints to answer.

### Task 12: Run the complete mobile regression matrix

**Files:**

- Modify only if a real product contract changed: relevant Storybook journey,
  verification, screen, or triage Markdown under `docs/storybook/`
- Test: all files above

**Interfaces:**

- Consumes: the production artifact after Tasks 1-11, including the approved
  terms document if Task 10 is unblocked.
- Produces: fresh evidence separated into rendering, interaction, accessibility,
  console, and performance/runtime exclusions.

- [ ] Run `cargo fmt --all -- --check`; expect no diff.
- [ ] Run `cargo test -p tonk-worker --test standard_library`,
  `cargo test -p tonk-workspace`, `cargo test -p tonk-fab`, and any changed
  access-service package tests; expect success.
- [ ] Run `nix develop path:. -c test:web:debug`; expect the changed UI,
  workspace, FABB, and display Wasm tests to pass.
- [ ] Run `nix develop path:. -c cargo test -p tonk-ui --features integration-tests -- --test-threads=1 --nocapture`;
  expect the whole browser account suite to pass. If ChromeDriver does not
  match Chrome, report that as an infrastructure block rather than product
  evidence.
- [ ] Run `nix develop path:. -c build:web`; serve the production artifact in
  disposable profiles at `320x568x2` and `390x844x2`, mobile/touch. Exercise
  Hub, create space, join, registration, activation, Settings, account delete,
  and space remove in light/dark and reduced-motion modes.
- [ ] For each journey, record: no horizontal overflow; all modal controls
  reachable; focus contained/restored; all actionable targets at least
  `44x44`; editable inputs at least `16px`; no duplicate request; no stale
  progress/refusal copy; and no new console warning/error.
- [ ] Keep the fresh-space recursive mutex panic in its separate incident
  record if still present. It must not be waived as a mobile-plan failure or
  suppressed to make this matrix green.
- [ ] From `docs/storybook`, run `python3 scripts/build.py --check` and
  `python3 scripts/check-links.py .`; expect both to pass after any product
  behavior update.

## Handoff order

Tasks 1, 4, 5, 6, 7, 8, 9, and 11 are independently reviewable. Task 2 must
land before Task 3 because the duplicate-submit tests address the final native
registration host. Task 10 is blocked on canonical terms content and can land
later without blocking the other mobile hardening work. Task 12 runs only after
all unblocked tasks have their focused checks green.
