# Edge states implementation plan

**Goal:** Implement the six captured edge states for “the moments a space
will not open” so every refusal in the product speaks the FABB grammar:
walls get a cluster, conditions get a banner, one solid ink block per screen,
rejection flashes ink instead of taking a hue.

**Approach:** Build the two missing materials first — the cluster grammar
(statement / entry row / narrator / fused run / ghost) as `tonk-fab` shadow
components for ceremonies over a space, and the same grammar as CSS-plus-markup
inside `profile.yaml` for the walls, exactly as the wireframes restate tokens
per file. Then convert each existing refusal surface: the Web-Awesome join
failure and paste form become walls, the share-driven "turn on sync?" dialog
becomes the offline banner's connect ceremony, and the activation nag becomes
the not-activated banner and ceremony.

Reference: the historical `~/tonk/gooey/fabb/edges.html` behavior already
captured in this plan (the source file is no longer present as of 2026-08-25),
`~/tonk/gooey/fabb/README.md` (laws), and the current `hub.html` stone tokens.
This plan is normative for edge anatomy and interaction; implementation must
not depend on recovering the removed prototype.

**Depends on:** `plan/hub-color.md` — Task 1's stone FABB skin, Task 2's Hub
tokens (`--dim`, `--cur`, `--panel`), and Task 5's truthful provider-free local
state. Land that plan first.

**Constraints:**

- One solid ink block per screen; the coloring is the CTA. Soft grey may sit
  on a reading, never on an actionable word.
- Rejection = flash the ink wash twice (`.45s × 2`) and re-arm; no hue, ever.
- A ceremony bails on Escape or its ghost word only — **a click on the dim
  does nothing**. Native `<dialog>` in `rust/tonk-fab/src/dialog.rs` must be
  configured so no backdrop or light-dismiss path closes a cluster.
- Revoked and deleted are one screen. The UI must not reveal which — that
  leaks whether a space exists. Detail stays in the console.
- Email activation stays the access-service **link** flow
  (`rust/tonk-account/src/customer.rs` — `Enrolled, activation email not yet
  acted on`). The wireframe's typed six-digit code row for screen 6 is **not
  built** in this slice: the code ceremony that exists
  (`tonk-account-service` `/codes`) proves address control at account
  *creation* and is consumed by `plan/onboarding.md`. Adding code-based
  *activation* is an access-service change; do it with that service, not
  here. The ceremony structure, banner, resend verb, and success payoff are
  all built now, so the code row later replaces one narrator sentence.
- The sealed guest has no `localStorage`; walls and banners live in guest
  chrome and views. `tonk-fab` components stay shadow-DOM;
  `profile.yaml`/`core.yaml` walls are light-DOM markup+CSS.
- Link validation reuses `rust/tonk-workspace/src/invite_link.rs` parsing —
  not the wireframe's `tonk.xyz` regex, which encodes a deployment origin
  this repo doesn't use.

## File map

- `rust/tonk-fab/src/field.rs`: `<tonk-field>` — the entry/record row.
- `rust/tonk-fab/src/cluster.rs`: `<tonk-cluster>` — statement, narrator,
  slotted rows/run/ghost, dim, bail semantics, focus loop.
- `rust/tonk-fab/src/banner.rs`: `<tonk-banner>` — the condition banner.
- `rust/tonk-fab/src/skin.rs`: edge-grammar tokens shared by the above.
- `rust/tonk-fab/src/share.rs`: enable-sync rerouted through the connect
  ceremony.
- `rust/tonk-fab/src/markup.rs`: refusal dialogs re-authored; law tests.
- `rust/tonk-core/assets/library/profile.yaml`: the walls (no access,
  invitation closed), provider-free Hub treatment, and empty-Hub column.
- `rust/tonk-core/assets/library/core.yaml`: local-spot notice restyled to
  the grammar.
- `rust/tonk-workspace/src/sync.rs`: revoked/deleted collapse at the copy
  boundary.
- `rust/tonk-worker/tests/standard_library.rs`: wall/copy contracts.

### Task 1: `<tonk-field>` — the entry and record row

**Files:**

- Create: `rust/tonk-fab/src/field.rs`
- Modify: `rust/tonk-fab/src/skin.rs` (row tokens: 36px block, noun
  bottom-left soft, value bottom-right ink, the existing 7×13 `.cur` block
  cursor on the tail)
- Modify: `rust/tonk-fab/src/lib.rs` (registration)
- Test: `rust/tonk-fab/src/field.rs:tests` (native markup) and
  `rust/tonk-fab/tests/edge_primitives.rs` (wasm interaction)

**Interfaces:**

- Attributes: `noun`, `value`, `settled` (record row, no cursor),
  `filter="digits"` (strip non-matching input), `autolen="6"` (auto-commit at
  length, `.14em` tracking), `changeable` (the noun swaps to an underlined
  `change` verb + cursor glyph on hover/focus; on `(pointer:coarse)` the
  glyph is always shown and hover does nothing — captured screen 6 behavior).
- Events: composed `fabb-commit {value}` on Enter or autolen;
  `fabb-change-noun` when the changeable noun is picked.
- Method surface for the owner: `reject()` — flash the ink wash twice and
  re-arm with contents selected. No hue.
- This is the `tonk-field` that `plan/fabb-conformance.md` deferred "with the
  surface that needs it"; `plan/onboarding.md` consumes the same element.

- [ ] Add native markup tests: noun/value seating, settled has no cursor,
      digits filter declared, no color literal outside the skin tokens; run
      `cargo test -p tonk-fab field` and record the failures.
- [ ] Add wasm tests: typing commits on Enter; `autolen` commits at exactly
      six digits and strips letters; `reject()` re-arms with the value
      selected; changeable-noun event fires; run
      `nix develop . -c cargo test --target wasm32-unknown-unknown -p tonk-fab field`
      and record the failures.
- [ ] Implement in the `shadow.rs` idiom: `shadow::Bound` for every listener,
      reduced-motion kills the flash and blink.
- [ ] Run both focused suites; expect green.

### Task 2: `<tonk-cluster>` — the ceremony shell

**Files:**

- Create: `rust/tonk-fab/src/cluster.rs`
- Modify: `rust/tonk-fab/src/skin.rs`, `rust/tonk-fab/src/lib.rs`
- Test: `rust/tonk-fab/src/cluster.rs:tests` (native markup) and
  `rust/tonk-fab/tests/edge_primitives.rs` (wasm interaction)

**Interfaces:**

- A 432px column of 36px blocks with 7px gaps over an ink dim, seated at
  modal density: `statement` slot (IBM Plex Sans 600 ink 13.5/1.55), default
  slot for `<tonk-field>` rows, `narrator` slot (sans 400 soft, at most one
  underlined verb bottom-right), `run` slot (fused `<tonk-button>` pair at
  gap 0, the fill boundary is the divider; a lone door fills the rung),
  `ghost` slot (bare underlined word, `◂` prefix, no box).
- Bail: Escape and the ghost emit composed `fabb-bail`; pointer events on the
  dim are swallowed. A minimal Tab loop cycles the open cluster.
- Phone: fused runs unstack, preserving the captured phone behavior.
- Reuses `tonk-button`; does not reuse `tonk-dialog` (heading/×/actions
  chrome is the wrong grammar).

- [ ] Native tests: one dim, slots present, ghost is not a button, run
      buttons have zero gap; run `cargo test -p tonk-fab cluster`, record
      failures.
- [ ] Wasm tests: Escape bails; ghost bails; a click on the dim does nothing
      and the cluster stays; Tab from the last focusable lands on the first;
      record failures, implement, re-run to green.
- [ ] Run the full `cargo test -p tonk-fab`; expect no regressions in the
      existing 96 native tests.

### Task 3: `<tonk-banner>` — conditions get a banner

**Files:**

- Create: `rust/tonk-fab/src/banner.rs`
- Modify: `rust/tonk-fab/src/lib.rs`
- Test: `rust/tonk-fab/src/banner.rs:tests` (native markup) and
  `rust/tonk-fab/tests/edge_primitives.rs` (wasm interaction)

**Interfaces:**

- Fixed, bottom-centered: `bottom:40px`, `width:min(680px, 100vw - 48px)`,
  frost glass with backdrop-blur, a soft message cell plus **one** solid ink
  door cell; slides up 70px after a 450ms beat; composed `fabb-open` when the
  door is picked; `retire()` slides it away.
- Phone: seats at `bottom:max(76px, env(safe-area-inset-bottom) + 68px)` so
  it clears the bar's bottom-right seat, and wraps rather than clips.
- The door is the screen's one solid — a mounted banner and a mounted
  cluster never show together (opening the ceremony hides the banner's
  door under the dim; captured screen 6's live email repaint reaches the
  banner behind the dim).

- [ ] Native tests: one door cell, message cell soft, no hue; wasm tests:
      the beat delay exists (class flips after mount), `fabb-open` fires,
      `retire()` removes it; record failures, implement, green.

### Task 4: The walls — no access, invitation closed

**Files:**

- Modify: `rust/tonk-core/assets/library/profile.yaml:772-807` (join failure
  card), `:958-985` (join paste form), plus the wall CSS block
- Modify: `rust/tonk-workspace/src/sync.rs:99-102` (revoked copy collapse)
- Modify: `rust/tonk-workspace/src/invite_link.rs` (expose the parse verdict
  the wall's row needs)
- Test: `rust/tonk-worker/tests/standard_library.rs`,
  `rust/tonk-workspace/src/invite_link.rs:tests`

**Interfaces:**

- Walls are light-DOM markup+CSS in the profile library (they render in
  views, where `tonk-fab` is chrome, not content): the 432 column keeps its
  seat under a phantom rung (`margin-left:216`, `margin-top:43px`, preserving
  the captured wall geometry), masthead logo fixed at 48/48.
- **No access** (historical screen 2): statement "you do not have access to
  this space", an `invitation link` entry row, narrator, fused run —
  `start a new space` quiet / `join this space` solid. A paste that
  `invite_link.rs` cannot parse flashes the row, rewrites the narrator to
  point at the link shape, and stays armed. This replaces both the
  `<wa-callout variant="danger">` failure card and the `<wa-input>` paste
  form.
- **Invitation closed** (screen 3): one screen for revoked *and* deleted;
  the dead link stays on as a settled row; the run offers the same two
  doors. `sync.rs`'s distinct revoked sentence collapses into the shared
  copy; the variant distinction survives only in console diagnostics.
  `<tonk-join-retry>` stays wired to the solid door where a retry is the
  honest action.

- [ ] Add standard-library contract tests: no `wa-callout`/`variant="danger"`
      in the join surfaces, no `{reason}` interpolation in wall copy, the
      exact statement strings above, exactly one solid (`.ebtn.solid`-class
      count) per wall; run
      `cargo test -p tonk-worker --test standard_library`, record failures.
- [ ] Add an `invite_link.rs` unit test for the parse-verdict function the
      wall consumes (good link → target, bad paste → structured refusal, no
      panic on garbage).
- [ ] Rewrite the two surfaces; wire the flash/narrator-rewrite through the
      library's existing scripting hooks; delete the orphaned Web Awesome
      wall styling.
- [ ] Run the standard-library suite and
      `nix develop . -c cargo test --target wasm32-unknown-unknown -p tonk-workspace`;
      expect green.

### Task 5: No sync server — the offline condition

**Files:**

- Modify: `rust/tonk-fab/src/bar.rs` (mount the banner when the bar's sync
  state is offline-with-no-remote), `rust/tonk-fab/src/share.rs:541-648`
  (enable-sync machinery extracted and reused), `rust/tonk-fab/src/markup.rs`
  (retire `#fab-enable-sync`)
- Modify: `rust/tonk-core/assets/library/core.yaml:435-459` (local-spot
  notice restyled to the grammar)
- Test: `rust/tonk-fab/tests/` wasm + native markup tests

**Interfaces:**

- The banner reads `connect this space` and mounts beside the offline bar
  (real `state="offline"` disc). Its door opens a `<tonk-cluster>` ceremony:
  header `connect this space`, a `sync server` `<tonk-field>` prefilled from
  `default_remote_url(&origin)` (`share.rs:614`), narrator, one full-width
  solid `connect`, ghost `◂ keep it on this device`.
- Commit runs the existing enable-sync path (set upstream, sync); success
  sets the disc to `synced` and retires the banner. The wireframe's
  editable server row is the delta from today's yes/no dialog — the user can
  point the space at their own remote.
- Share on a local-only space routes through this same ceremony instead of
  the `turn on sync?` dialog, which is deleted with its markup test rows.
  The ghost returns to the share stack with sync still off.

- [ ] Wasm test: a bar driven to offline-no-remote mounts one banner; the
      door opens the ceremony with the prefilled server value; ghost retires
      to banner; record failure against current behavior.
- [ ] Update the native markup law tests that pin `REFUSAL_DIALOGS_HTML` to
      expect the ceremony grammar instead of the dialog.
- [ ] Extract the enable-sync commit from `share.rs` into a function taking
      the remote URL; wire both entries (banner, share) through it.
- [ ] Restyle the core-library local-spot notice with the wall/condition
      grammar and copy pointing at the banner's verb, dropping the
      `--wa-*` tokens.
- [ ] Run `cargo test -p tonk-fab`,
      `nix develop . -c cargo test --target wasm32-unknown-unknown -p tonk-fab`,
      and `cargo test -p tonk-worker --test standard_library`; expect green,
      with `rust/tonk-worker/tests/fab_drift.rs` and the browser fixtures in
      `rust/tonk-ui` updated for the removed dialog.

### Task 6: Email not activated — banner and ceremony

**Files:**

- Modify: `rust/tonk-fab/src/bar.rs` (condition detection),
  `rust/tonk-fab/src/share.rs` (the apologising share row swap)
- Test: `rust/tonk-fab/tests/activation_banner.rs` (wasm)

**Interfaces:**

- Condition source: `GET /api/customer`, whose
  `rust/tonk-worker/src/router/customer.rs::CustomerState` response carries
  `status` and `email`. Mount only when `status == "Registered"`; retire when
  it becomes `"Active"`. `rust/tonk-ui/src/api.rs::customer_state` is the
  trusted top-document precedent for the same read.
- Banner: `{email} is not activated yet — nothing syncs until it is` /
  solid `activate`. Ceremony: settled email `<tonk-field changeable>` (the
  change verb routes to `/account`, where email is owned), a narrator
  carrying the link-flow sentence and a `resend activation email` verb. It
  sends `POST /api/customer/enroll` with the same `{email:null,deposits:[]}`
  shape as `rust/tonk-ui/src/api.rs::enroll_customer(None, &[])`; the guest
  reaches both requests through the existing fetch bridge. Ghost copy is
  `◂ back to your space`.
- Success: on the activation state flipping (re-poll on ceremony open and on
  an interval while mounted), dim lifts, disc fills, banner retires, and the
  share menu's `sharing needs an activated email` row becomes `copy link`.
- A live email repaints every mention, including the banner behind the dim.

- [ ] Wasm test with a stubbed `/api/customer` response: `Registered` mounts
      the banner with the response email; resend POSTs once to
      `/api/customer/enroll` and reports in the
      narrator; a flipped activation state retires banner and dim; record
      failures, implement, green.
- [ ] Verify the share-row swap with the existing share wasm fixtures.

### Task 7: Truthful provider-free and empty-Hub states

**Files:**

- Modify: `rust/tonk-core/assets/library/profile.yaml` (provider-free and empty
  states)
- Test: `rust/tonk-worker/tests/standard_library.rs`

**Interfaces:**

- Consumes `plan/hub-color.md` Task 5's provider-free local state.
- **Provider-free local profile** (historical screen 5): the available roster
  data does not distinguish “never attached” from “signed out”, and removing a
  provider does not remove local repositories. Do not render a blocking edge
  wall. The account cell retains the active local profile label and roster
  trigger; settings offers the local-account `/account` handoff; and the spaces
  view, existing local rows, open actions, and solid local create row stay
  available. The create row remains the screen's one solid. This is a
  deliberate truthfulness delta from the historical “this device was signed
  out” wireframe.
- **Empty Hub** (historical screen 1): the provider-free profile/roster header
  sits above the normal empty spaces stack. Render one neutral `no spaces yet`
  fact and the normal solid `create a new space` row; do not claim that account
  state made spaces unavailable.

- [ ] Standard-library contract tests: provider-free markup contains no
      “signed out” or “no spaces available” claim; existing space links and
      the create form remain present; the roster trigger and settings handoff
      remain reachable; the empty state says `no spaces yet` exactly once. Add
      a structural one-solid assertion for each edge surface; run, record
      failures, implement, green.
- [ ] Full-slice verify: `cargo fmt --all -- --check`,
      `cargo test -p tonk-fab`,
      `cargo test -p tonk-worker --test standard_library`,
      `nix develop . -c cargo test --target wasm32-unknown-unknown -p tonk-fab -p tonk-workspace`,
      `nix develop . -c build:web`.
- [ ] Walk all six captured states in isolated Chrome (fresh provider-free
      profile for the local/empty Hub;
      a local-only space for the offline banner; a revoked invite for the
      closed wall), light and dark, 1440px and 390px: one solid per screen,
      flashes not hues, banner clears the bar on the phone seat, Escape/ghost
      bail everywhere, dim clicks dead, no console errors.
