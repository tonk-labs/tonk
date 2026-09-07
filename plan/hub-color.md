# Centered Hub launcher and stone palette implementation plan

**Goal:** Replace the fixed top-right Hub strip with the approved centered,
432px launcher; make settings an attached in-flow view; and bring Hub and FABB
chrome onto the neutral stone-ink palette in the current
`~/tonk/gooey/fabb/hub.html`.

**Approach:** Retoken the shared chrome first, then reshape the Hub as one
self-contained column whose header, space rows, creation row, and settings view
all share a 432px grid. Keep the account/device data plumbing already present
in `<ui-hub-account>`, but replace its modal shell with horizontally joined
tabs and move its settings trigger into the header.

**Approved reference:**

- Palette: `~/tonk/gooey/fabb/hub.html` (current stone light/dark token blocks).
- Geometry and interaction:
  `~/tonk/tonk/.wt/wireframes-proto-hub-layout/prototype/hub-layout.html`,
  launcher variant with the gridded end. The 810px centered comparison,
  detached mode cell, and semicircular cap are rejected.

**Constraints:**

- Copy the color values from the current `hub.html` token blocks when
  implementing; the explicit values below pin the review contract.
- The production launcher has one stable maximum width: `432px`. Do not port
  the prototype's comparison-only 810px layout or its 520-680px width branch.
- The logo and launcher are horizontally centered near the top third. The
  result must read as a complete launcher, not a toolbar waiting for page body
  content.
- The mode control stays in the grid as a square cell. No pill, semicircle,
  detached cell, or solid ink end-cap.
- Show the word `settings` whenever the available header width is the full
  432px (`min-width:464px` with the 16px page gutters); below that, retain its
  icon and accessible name while hiding only the visible word.
- `spaces` and `settings` are mutually exclusive in-flow views. No scrim,
  modal dialog, close button, focus trap, or inert background.
- A provider-free local profile keeps its local spaces and local creation
  authority. It must not become a false “no spaces available” wall. Account
  attachment, customer activation, sync, and local space access remain
  separate states.
- WebAuthn and destructive account/device operations remain in the trusted
  top-document `/account` surface. The Hub links out for those operations.
- Dark mode continues to key off `:root.wa-dark`; no second theme signal.
- No Usage/billing UI and no new dependencies.

## Implementation status (2026-08-25)

The plan is implemented, with these approved follow-up refinements superseding
the original settings geometry in Task 4:

- [x] The account roster is a tab, not a toggle: clicking the active account
      tab again is a no-op; Escape, outside click, spaces, or settings dismisses
      or replaces it. Its 44px rows are independent outlined blocks separated
      by 7px.
- [x] Empty spaces show only the solid `create a new space` row.
- [x] Opening the account roster moves `aria-current="page"` from the previous
      header control to the account trigger, then restores it on close.
- [x] Wide desktop uses one proportional 576px launcher: header cells are
      224/192/112/48px and settings is a 144px side rail plus 432px body, with
      the selected rail tab fused to a 408px-tall panel.
- [x] At 607px and below, the launcher returns to the original 432px
      168/144/84/36 grid and settings returns to 108/324px.
- [x] At 463px and below, settings uses two full-width top tabs and retains zero
      horizontal overflow.
- [x] Selected and deselected settings tabs share the panel's inset-border
      geometry, so their outer edges stay aligned; the selected tab removes
      only its docking edge to keep the tab and panel one continuous surface.
- [x] Settings uses label/value rows, passkey explanation, section dividers,
      flat device rows, and an underlined `/account` handoff. WebAuthn and
      destructive operations remain outside the Hub.

Fresh verification after the final refinement:

- [x] `cargo fmt --all -- --check` and `git diff --check`.
- [x] `cargo test -p tonk-worker --test standard_library` (11 passed).
- [x] `cargo test -p tonk-workspace --target wasm32-unknown-unknown` in the Nix
      development shell (65 passed).
- [x] `nix develop . -c build:web` (the first alignment rebuild was interrupted
      externally during Wasm compilation; the clean retry passed).
- [x] Isolated Chrome measurements at 1200px, 608px, 607px, and emulated 390px:
      wide desktop is 576px with 144/432 settings; 607px is 432px with 108/324
      settings; compact rail and body are both 358px; no horizontal overflow.
- [x] Isolated Chrome reproduced the initial alignment mismatch caused by mixed
      shadow and border geometry; the final shared inset-border model preserves
      aligned outer edges and erases only the selected tab's docking seam.
- [x] Fresh production-build screenshots and computed styles confirm account
      and devices both fuse to the side panel in desktop light/dark modes and
      to the top panel at the emulated 390px compact breakpoint, with no
      horizontal overflow.
- [ ] The real-account WebDriver integration test was not rerun for these
      presentational follow-ups; its selectors were updated and the component's
      attached-account behavior remains covered by the Wasm suite.

## File map

- `rust/tonk-fab/src/skin.rs`: neutral FABB color defaults and dark public
  token twins; floating frost/panel materials remain translucent.
- `rust/tonk-fab/src/markup.rs`: native token/law regressions.
- `rust/tonk-core/assets/library/profile.yaml`: Hub tokens, centered geometry,
  header/stack markup, settings view, and responsive rules.
- `rust/tonk-workspace/src/ui_mode_switch.rs`: square split-tone mode mark.
- `rust/tonk-workspace/src/ui_hub_account.html`: complete header, account menu,
  and settings-view markup.
- `rust/tonk-workspace/src/ui_hub_account.rs`: header/settings view state and
  local-profile presentation.
- `rust/tonk-worker/tests/standard_library.rs`: profile-library structure and
  token contracts.
- `rust/tonk-ui/src/account_flow.rs`: real-browser Hub/settings regression.

### Task 1: Retoken FABB chrome from olive to stone ink

**Files:**

- Modify: `rust/tonk-fab/src/skin.rs`
- Modify: `rust/tonk-fab/src/markup.rs`

**Interfaces:**

- Light public defaults are `--fabb-ink:#131313`,
  `--fabb-ink-soft:#55544f`, `--fabb-on-ink:#fbfaef`,
  `--fabb-sep:rgba(19,19,19,.28)`,
  `--fabb-hover:rgba(19,19,19,.06)`,
  `--fabb-press:rgba(19,19,19,.12)`, and
  `--fabb-ring:rgba(19,19,19,.85)`.
- Dark public twins are `--fabb-ink-dark:#e9e6d6`,
  `--fabb-ink-soft-dark:#cdcaba`, `--fabb-on-ink-dark:#22221c`,
  `--fabb-sep-dark:rgba(233,230,214,.28)`,
  `--fabb-hover-dark:rgba(251,250,239,.09)`,
  `--fabb-press-dark:rgba(251,250,239,.15)`, and
  `--fabb-ring-dark:rgba(233,230,214,.55)`.
- Internal `--_ring`/`--_ringc` read the public ring token instead of a
  hardcoded literal.
- Keep the floating FABB surfaces as their existing translucent defaults
  (`--fabb-bg:rgba(255,255,255,.72)` and
  `--fabb-panel:rgba(255,255,255,.92)`, plus their existing dark twins).
  Hub's opaque `--panel:#d2d2d2` is not a floating-menu material.

- [ ] Add native assertions for every light/dark public token, reject
      `#34332b` and `rgba(43,44,20`, and require a `var(--fabb-ring` reference;
      run `cargo test -p tonk-fab` and record the failures.
- [ ] Change only the color/token declarations, keeping FABB geometry and
      translucent surface defaults unchanged.
- [ ] Run `cargo test -p tonk-fab`; expect green.
- [ ] Run
      `nix develop . -c cargo test --target wasm32-unknown-unknown -p tonk-fab`;
      expect the existing element and geometry suites green.

### Task 2: Complete the Hub stone token contract

**Files:**

- Modify: `rust/tonk-core/assets/library/profile.yaml`
- Modify: `rust/tonk-worker/tests/standard_library.rs`

**Interfaces:**

- The light Hub block contains:
  `--page:#ececec`, `--ink:#131313`, `--on-ink:#fbfaef`,
  `--soft:#55544f`, `--ring:rgba(19,19,19,.85)`,
  `--sep:rgba(19,19,19,.28)`, `--frost:rgba(255,255,255,.72)`,
  `--frost-solid:#fafafa`, `--panel:#d2d2d2`, `--card:#fff`,
  `--card-hover:#f1f1f1`, `--wash:rgba(19,19,19,.06)`,
  `--wash-2:rgba(19,19,19,.12)`,
  `--wash-p:rgba(251,250,239,.16)`, `--canvas:#e9e9e7`,
  `--stub-ink:#9a9993`, `--veil:rgba(236,236,236,.9)`,
  `--dim:rgba(16,16,12,.32)`, and `--track:rgba(19,19,19,.22)`.
- The dark block contains the current `hub.html` twins:
  `#161613`, `#e9e6d6`, `#22221c`, `#cdcaba`, ring/sep `.55`/`.28`,
  frost `rgba(32,32,26,.78)`, frost-solid `#1e1e19`, panel `#3b3a34`,
  card/card-hover `#26251f`/`#32312a`, wash/wash-2 `.09`/`.15`,
  wash-p `.14`, canvas `#1b1b18`, stub `#6f6e66`, veil `#161613`,
  dim `.45`, and track `.25`.
- Define `--cur:var(--panel)` in the shared block. Current header cells,
  active settings tabs, and settings body all consume that one register.
- Pure white `--card:#fff` is intentional because it is the current
  `hub.html` value; do not retain the old plan's contradictory rejection.

- [ ] Extend `standard_library.rs` to require `--panel`, `--cur`,
      `--card-hover`, `--canvas`, `--stub-ink`, `--veil`, and `--track`; run
      `cargo test -p tonk-worker --test standard_library` and record failure.
- [ ] Replace/complete both token blocks and repoint matching hardcoded color
      uses to tokens without changing behavior.
- [ ] Run `cargo test -p tonk-worker --test standard_library`; expect green.

### Task 3: Build the centered 432px launcher grid

**Files:**

- Modify: `rust/tonk-core/assets/library/profile.yaml`
- Modify: `rust/tonk-workspace/src/ui_hub_account.html`
- Modify: `rust/tonk-workspace/src/ui_mode_switch.rs`
- Modify: `rust/tonk-worker/tests/standard_library.rs`

**Interfaces:**

- `.hubcol` is `position:relative; width:min(432px, calc(100vw - 32px));
  margin-inline:auto; padding-top:clamp(148px,23vh,220px)`.
- `.hub-logo` is centered independently at
  `top:clamp(62px,11vh,106px); left:50%; transform:translateX(-50%)`, with a
  132px-wide mark. At `max-width:519px`, use 98px and `top:62px`; the launcher
  starts at 132px.
- The square, 36px-high header is one 432px row:
  account `168px`, spaces `144px`, settings `84px`, mode `36px`. It has no
  radius and no detached gap. The selected view uses `background:var(--cur)`.
- Below `464px`, account is `50%`, spaces flexes, settings is `44px`, mode is
  `44px`, and `.settings-word` is visually hidden. At `min-width:464px`, restore
  the exact `168 + 144 + 84 + 36` grid and show `settings` beside its icon.
- `profile.yaml` seats `<ui-hub-account>` as the first child of `.hubcol`.
  `ui_hub_account.html` owns the complete `.hubbar`: account trigger, spaces
  button, settings button, and `<ui-mode-switch>`. This keeps all header and
  settings controls inside the component's existing event/listener lifetime.
- `<ui-mode-switch>` renders an inert `.mode-mark` child: a 10px square with
  `linear-gradient(90deg,var(--ink) 50%,transparent 50%)` and a 1px ink ring.
  The button remains the existing labelled `role=switch` and 36/44px cell.
- `.stack` is 432px/max-available wide, starts 7px below the header, and uses
  7px vertical gaps. Each space remains a 36px outlined row. Move the existing
  create form into the final solid-ink row labelled `create a new space`; keep
  its `Untitled`, remote, and revocation fields unchanged.

- [ ] Add library assertions rejecting fixed/right Hub geometry,
      `border-radius:0 18px`, and a separate `.shead`; require `.hubcol`,
      `.hc-view`, `.hc-cfg`, the in-stack create form, and the 432px maximum.
- [ ] Add a Wasm `ui_mode_switch.rs` DOM test for one labelled switch and one
      `.mode-mark`; run the focused Wasm test and record failure.
- [ ] Restructure the profile CSS/markup and mode switch; preserve all space
      list, remove/archive, default-remote, and rename behavior.
- [ ] Run `cargo test -p tonk-worker --test standard_library` and
      `nix develop . -c cargo test --target wasm32-unknown-unknown -p tonk-workspace ui_mode_switch -- --nocapture`;
      expect green.

### Task 4: Replace settings modal with the attached settings view

**Files:**

- Modify: `rust/tonk-workspace/src/ui_hub_account.html`
- Modify: `rust/tonk-workspace/src/ui_hub_account.rs`
- Modify: `rust/tonk-core/assets/library/profile.yaml`
- Modify: `rust/tonk-ui/src/account_flow.rs`

**Interfaces:**

- `ui_hub_account.html` owns the sibling header buttons
  `[data-return-spaces]` and `[data-open-settings]` as well as
  `[data-settings-view]`. The component's existing host click/keydown
  listeners therefore delegate every header, menu, tab, and settings action;
  do not add document- or `.hubcol`-lifetime listeners.
- Opening settings closes the account menu, hides `.stack`, reveals
  `[data-settings-view]`, stamps the closest `.hubcol` with
  `data-hub-view="settings"`, marks settings `aria-current="page"`, and clears
  the spaces current state. Spaces click or Escape removes the root state,
  reverses the visibility/current attributes, and returns focus to the
  initiating header control.
- Remove the account-menu settings row, overlay, scrim, dialog role/header,
  close button, `set_hub_inert`, and `trap_settings_focus`.
- The view is 432px wide and begins 7px below the header. `account` and
  `devices` form a fused 2×216px horizontal tab row directly above the body.
  The active tab is `var(--panel)` and uses a 3px bottom seam plug of the same
  color. The body is `width:100%; min-height:258px; padding:24px;
  background:var(--panel)` with a one-pixel ink ring.
- Keep concurrent account-summary/devices loading, isolated failure messages,
  display-name commits, current-device marking, revoke links, and the
  `/account` handoff unchanged.

- [ ] Rewrite the wasm DOM test to open from the external settings header,
      assert the stack/view and `aria-current` swap, assert no modal/scrim/close
      nodes, switch tabs, then return by both spaces click and Escape. Run
      `nix develop . -c cargo test --target wasm32-unknown-unknown -p tonk-workspace ui_hub_account -- --nocapture`
      and record the failures.
- [ ] Implement the in-flow markup and root view state using only the existing
      component listener ownership. Preserve the existing data loaders and
      mutations.
- [ ] Update `account_flow.rs` selectors from `[data-settings-dialog]` /
      `[data-settings-close]` to `[data-settings-view]` /
      `[data-return-spaces]` and verify both panes still show real data.
- [ ] Run the focused wasm suite and the integration test
      `nix develop . -c cargo test -p tonk-ui --features integration-tests it_adds_a_second_account_and_switches_between_disjoint_space_lists -- --test-threads=1 --nocapture`;
      expect green.

### Task 5: Preserve truthful provider-free local behavior

**Files:**

- Modify: `rust/tonk-workspace/src/ui_hub_account.rs`
- Modify: `rust/tonk-core/assets/library/profile.yaml`
- Test: `rust/tonk-workspace/src/ui_hub_account.rs:tests`

**Interfaces:**

- The existing `data-active-provider="false"` roster fact means only that the
  active local profile has no attached account provider. It does not prove
  whether the profile was never attached or signed out, and it does not remove
  local repositories.
- In that state, the account header retains the active local profile label and
  opens the profile roster; do not replace it with a `/account` link that makes
  profile switching unreachable. Settings remains available: its account pane
  says the profile is not connected to an account and offers the existing
  `/account` handoff; its devices pane explains that attached devices require
  an account. The spaces header, rows, open actions, and solid create row remain
  present and enabled. A newly created space remains local-only until sync is
  configured. The create row remains the screen's only solid ink block.
- Do not add `data-out`, “this device was signed out”, or “no spaces
  available” claims without a future explicit lifecycle field.

- [ ] Add a DOM test with a provider-free roster and existing space markup:
      the account trigger still opens the roster, settings opens the local
      account/devices panes and their `/account` handoff, and spaces/create
      remain enabled; run it and record failure.
- [ ] Adjust only misleading local-pane copy; keep roster, settings, and local
      space behavior intact.
- [ ] Run the focused workspace wasm suite; expect green.

### Task 6: Verify the complete slice

- [ ] Run `cargo fmt --all -- --check` and `git diff --check`.
- [ ] Run `cargo test -p tonk-fab` and
      `cargo test -p tonk-worker --test standard_library`.
- [ ] Run
      `nix develop . -c cargo test --target wasm32-unknown-unknown -p tonk-workspace --lib`
      and the same command for `-p tonk-fab`.
- [ ] Run `nix develop . -c build:web` and the final Hub integration test
      from Task 4 against that build.
- [ ] Inspect the built Hub in isolated Chrome at 1440px, 464px, 463px, and
      390px,
      light and dark: centered logo/launcher; stable 432px maximum; square
      gridded end; settings word present only when it fits; attached horizontal
      tabs; exactly one solid creation row; local spaces usable without an
      account; no horizontal overflow or new console errors.
- [ ] Re-read the final diff: no 810px comparison layout, detached mode cell,
      semicircle, modal settings code, local-space suppression, new theme
      signal, Usage UI, in-guest WebAuthn, or unrelated color/material changes.
