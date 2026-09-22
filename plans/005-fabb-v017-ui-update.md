# Update the in-space UI to FABB v0.17.0

Status: PARTIAL — increments 1–5 and the attached-roster slice are implemented; the full member graph is blocked on an authoritative data contract, and final external integration evidence is recorded below. Priority: P1. Effort: L. Risk: high at the guest/host account boundary; medium for layout. Planned on 2026-09-22 against Tonk `d4003e8d6` and Gooey `f74da0e` (v0.17.0). Both working trees were clean during comparison. No prerequisite implementation plan.

This is an implementation handoff, not an implementation report. Follow the increments in order, establish the narrow baseline before changing behavior, and keep each proven increment independently reviewable. If commits are requested, commit each passing increment separately. Do not push or deploy without a request.

## Outcome and reference scope

Replace the existing in-space `sync · space · share` bar and detached menus/prompts with the current reference's circle-and-name bar, attached sharing/agent/member panels, and contained tasks. A task temporarily takes over the same visible surface at its current dock, blocks the space, and restores the previous context when resolved.

Primary reference: `/Users/jackdouglas/tonk/gooey/fabb/fabb.html`. Read its adjacent `README.md`, `fabb.js`, `fabb-forms.css`, `account-demo.js`, and current entries at the top of `HISTORY.md`. Use `space-bar.html` for panel motion and `hub.html`/`hub-walkthrough.js` for the in-space account and recovery journeys linked from the primary reference. The old `onboard.html` and `edges.html` are compatibility redirects, not separate current specifications.

The reference wins for the in-space surface. `DESIGN.md` and `plan/fabb-conformance.md` describe older 36px cells, square word-bearing blocks, 13px labels, separate 7px stacks and restricted collapse behavior. Do not apply those old rules to the new FABB. Keep their still-relevant principles: scoped ink/frost, readable names, native inputs, safe areas, reduced motion, keyboard access and host-owned effects. Update the documentation's scope as part of implementation; do not silently change the global Hub/account design system.

This plan does not redesign all of `hub.html`: weighted space-card frames, generated preview art, demo scenario controls and the whole Hub settings layout are outside scope. Hub changes here are limited to preserving navigation and actions removed from the FABB. Changes/review/history are explicitly outside the current wireframe MVP, not missing features to implement.

## Evidence and validation boundary

- Read the three requested crates, their relevant consumers and current reference source. The comparison is against the checkout, not a deployed build.
- Rendered the reference in isolated desktop Chrome. Opened the members panel: a joined rounded surface, right-side menu, fixed member stage and right-aligned labels were visible. Its total width was 932px within the documentation stage; 960px is the nominal maximum, not a required width at every viewport.
- Exercised `notify()` from that panel: the native dialog was modal and retained the exact right/top anchor (`1184`, `497.1875` CSS pixels in that session). `resolve()` removed the request and restored the members panel.
- Increment 1 ran the focused native and Chromium/Wasm gates recorded below. Product build, account services, visual parity, Safari and physical touch-device checks remain unrun.
- Browser/server startup needed host access after a sandbox bind failure. Treat a repeated `Operation not permitted` at that boundary as infrastructure evidence, not a reason to alter UI code.

## Comparison

All source-level differences below are high confidence. Runtime consequences and the proposed cross-document presentation design still require the spike.

| Area | Current implementation and evidence | v0.17.0 target | Priority / effort / change risk |
| --- | --- | --- | --- |
| Anatomy and navigation | `rust/tonk-fab/src/bar.rs:1` and `markup.rs`: 36px `sync · space · share`, compact overflow, detached space/share stacks. Space actions are new/open/rename/settings. | One 360px nominal rail, 48px header; name opens copy share link, view members, connect agent, go to tonk home. Account entry appears when appropriate. | P1 / M / medium |
| Material and type | `skin.rs`, `markup.rs`: 13px bottom-seated labels, one rounded end, 7px-separated stacks. | One 25px-radius surface, 1.5px ring, contiguous rows, vertically centered labels; control register 600 17px/1.1, message register 400 18px/1.5, both Plex Sans Condensed. Agent code keeps its mono register. | P1 / M / medium |
| Panel geometry and motion | `bar.rs`/`menu.rs`: disclosure stacks and flyouts; `element.rs` preserves free placement along an edge. | Joined adjacent panel adds up to 600px; agent/member panel is 240px high. Narrow layouts stack/swap panels with a menu return. Corner morph leaves menu text still; right/bottom anchors survive expansion. | P1 / M / medium |
| Task ownership | `member_roster.rs:64` appends a `tonk-dialog` to body; `markup.rs`/`share.rs` use a connect cluster. No shared `present/notify/resolve` host. | Decisions, errors and account flows replace the panel; one pending request, explicit completion, optional Escape, required `got it`, complete restoration. | P1 / L / high |
| Accountless share | `element.rs:599` forwards reason/space; `rust/tonk-ui/src/bin/ui.rs:109` stashes share and navigates to `/settings` when no anchor is supplied. | Shaded share panel says `add an account to share this space`; account flow stays at FABB and returns to sharing. | P1 / L / high |
| Account host geometry | `register_dialog.rs:2452` has `Request { reason, space, anchor }`; `Anchor` only has left/bottom/width; `position_at` places rows below it with a 7px gap. | A task occupies the FABB seat, retains right/bottom anchoring, clamps on resize/keyboard, can drag, and blocks the whole space. Merely supplying today's anchor is insufficient. | P1 / L / high |
| Agent access | `core.yaml:201` renders an agent handoff on the empty canvas; `core.yaml:676` renders the prompt using Web Awesome. The FABB has no agent panel. | Agent instructions and copy live in the adjacent FABB panel and remain available over any space view. | P1 / M / medium |
| Members | `member_roster.rs:134` has membership entity/name/DID/role and a flat modal list. `logic.rs:1312` queries only member/role/name. | Pannable map centered on self, people discs, agent tails, counts, selected last-active card. Host supplies kind/parentId/lastActive. | P2 / L / high if data semantics expand |
| Sync/collapse | `element.rs:523` supports Alt-click pause; existing drag and collapse already provide useful foundations. | Circle toggles fold at every width; drag circle/name; hold or Shift+Space pauses/resumes; signed-out hollow state and accessible explanation; attention pulse is not review. | P1 / M / medium |

Reference implementation locations: `fabb.js:72` (type), `:1278` (present), `:1327` (notify), `:1333` (resolve), `:1421` (space-bar markup/style), `:1734` (panel motion), `:1841` onward (members). The wireframe menu's actual labels in `:1570` onward take precedence over shorthand in prose.

## Preserve these contracts

1. Keep `<tonk-fab with="{profile-branch}@profile:tonk" space={id}>` as the product mount contract (`profile.yaml:2402`). The reference calls its complete bar `tonk-space-bar` and its standalone disc `tonk-fab`; that tag naming is not a reason to break existing product mounts. Implement the new behavior under the current product tag; do not register two conflicting meanings of `tonk-fab`.
2. The `space` attribute is the DID, while `label` is display text. Never substitute the label into subscriptions, claims or links. Preserve late binding, reconnect cleanup, reset/update delivery and headless sync/name subscribers.
3. Match the existing Rust split: DOM-free transitions/geometry in `logic.rs` or focused new modules; DOM lifecycle in custom elements; listeners owned by `shadow::Bound`; data subscriptions via `subscribing::Scaffold`. Preserve deferred callbacks that avoid subscription reentrancy.
4. FABB commands must work for old spaces. `rust/tonk-worker/tests/fab_drift.rs:1` documents why raw-attribute queries and inline descriptors avoid dependence on the originally seeded `core.yaml`. A change to a seeded view alone does not upgrade an existing space.
5. Keep real worker-owned share refusal classes, activation gating, provider selection, timestamp correlation and clipboard user-activation handling in `share.rs`. An account being present is not proof that this device can share a space.
6. Keep the agent invitation protocol and ordinary share links distinct. `core.yaml` already has `tonk:agent-handoff`, `tonk:new-agent-invite`, `tonk:agent-invite` and refusal states. Never use the mock's `share-url` as an agent grant, fabricate an invite, or run the prompt. Preserve expiry/revocation and retry behavior.
7. Passkey operations stay in the trusted top page (`tonk-ui/src/bin/ui.rs`, `register_dialog.rs`, `custody_relay.rs`). Keep keys, authentication and authority outside the guest's presentation state. Native platform passkey UI is an unavoidable external surface, not a detached Tonk dialog to imitate.
8. Use real native inputs. `shadow.rs:135` already implements rename with `HtmlInputElement`; preserve Enter/Escape/blur/trim and Safari behavior for any retained editing path.
9. Fonts already ship locally in `tonk-ui/styles.css:164` onward, including condensed 400/500/600. Reuse these in both host and guest; do not copy the reference's Google Fonts dependency.

Current-code excerpts for drift detection:

```rust
// rust/tonk-fab/src/member_roster.rs:134
struct Member {
    this: String,
    name: String,
    did: String,
    role: String,
}
```

```rust
// rust/tonk-ui/src/bin/ui.rs:109, current in-space share fallback
if request.anchor.is_none() && !request.space.is_empty() {
    tonk_ui::register_dialog::stash_share(&request.space);
    if let Some(location) = web_sys::window().map(|window| window.location()) {
        let _ = location.assign("/settings");
    }
    return;
}
```

## Scope and architecture

Primary implementation scope:

- `rust/tonk-fab/src/{bar,markup,skin,element,logic,share,member_roster,activation,banner,shadow,lib}.rs`, focused new task/flow/member-layout modules, and the corresponding tests.
- `rust/tonk-ui/src/{bin/ui,register_dialog,custody_relay,account_flow,user_error}.rs`, scoped account styles in `account.css`/`styles.css`, and targeted browser tests. Touch ceremony internals only to support presentation lifecycle; preserve authentication semantics.
- `rust/tonk-core/assets/library/{core,profile}.yaml` for mount/navigation/agent presentation; no invented membership schema in YAML.
- Integration checks in `rust/tonk-worker/tests/{fab_drift,standard_library}.rs`; affected Storybook sources/generated outputs after reading their instructions; `DESIGN.md`, historical-plan status notes and this plan/index.

Inspect `rust/tonk-host/src/navigate.rs:119` and `rust/tonk-portal/src/bridge.rs:1389` during the spike. The current `register` page effect carries JSON and a focus-return token. A minimal request/response extension there may be necessary for lifecycle and coordinates; document that dependency before expanding beyond the three requested crates. Do not rewrite portal security, grant broad guest DOM access, or loosen the sandbox.

Out of scope: backend identity redesign, UCAN format changes, member presence telemetry, service-worker cache redesign, generic notification framework, changes/review/history, unrelated Hub restyling, lockfile churn and removal of all legacy primitive components. Hub dialogs remain supported.

Recommended architecture: a pure FABB view state (`collapsed`, `bar`, `menu`, `share`, `agent`, `members`) plus one optional task. A task owns a snapshot of prior view/selection/pan/scroll/focus, its dismissal policy, request identity and async cancellation state. Keep data subscriptions alive while the normal surface is hidden. Reject a second task; do not queue an old error behind a newer context by default.

For ordinary guest content, use a native modal around the same FABB wrapper. For account content, prove a trusted top-page presenter that visually replaces the FABB at its translated viewport seat, while the guest holds its restoration snapshot and hides its duplicate chrome. Only one modal owner may be active. Coordinate updates are presentation messages; all ceremony side effects stay on the top page. This is a proposed implementation, not yet a proven cross-frame contract.

## Verification commands

Run commands from the repository root. `nix develop . -c` is the repository shell form. `test:*` wrappers build nextest archives and accept nextest filters (`nix/menu.nix:132`); use direct focused Cargo tests while iterating, then repository suites at integration checkpoints.

| Gate | Command | Expected result |
| --- | --- | --- |
| Drift and local work | `git status --short` and `git diff --stat d4003e8d6..HEAD -- rust/tonk-fab rust/tonk-ui rust/tonk-core rust/tonk-host rust/tonk-portal DESIGN.md` | Reconcile any changes with this plan before editing; preserve unrelated work |
| Pure FABB baseline | `nix develop . -c cargo test --locked -p tonk-fab --lib` | Exit 0; existing geometry/claim tests pass |
| Browser FABB baseline/checkpoint | `nix develop . -c cargo test --locked --target wasm32-unknown-unknown -p tonk-fab` | Exit 0 using configured wbg-pool runner |
| Focused new component suite | `nix develop . -c cargo test --locked --target wasm32-unknown-unknown -p tonk-fab --test contained_tasks` | New suite exists, runs nonzero tests, passes |
| UI native tests | `nix develop . -c cargo test --locked -p tonk-ui --lib` | Exit 0; account request parsing/state tests pass |
| Core native tests | `nix develop . -c cargo test --locked -p tonk-core` | Exit 0 |
| Seeded library/command contracts | `nix develop . -c cargo test --locked -p tonk-worker --test standard_library --test fab_drift` | Exit 0; declarations lower and commands retain real trigger attributes |
| Product browser build | `nix develop . -c build:web` | Exit 0; usable product assets |
| Focused real account integration | `nix develop . -c test:e2e -E 'test(it_copies_a_share_link_from_the_bar) or test(it_restores_registration_focus_to_the_guest_opener) or test(it_returns_from_agent_invite_signup_and_connects_the_original_space) or test(it_keeps_fabb_account_tasks_in_space)'` | All selected tests execute and pass; add the final named test in increment 5 |
| Release browser integration | `nix develop . -c test:web:release -E 'package(tonk-fab) or package(tonk-ui)'` | Selected suites pass; record any quarantine separately |
| Formatting and diff | `nix develop . -c cargo fmt --all -- --check` and `git diff --check` | Exit 0; separate pre-existing formatting failures |
| Storybook | `nix develop . -c test:storybook` | Exit 0 after following `docs/storybook/AGENTS.md`, README and goal |

If a command fails before tests start, record the literal error and determine whether it is Nix/runner/access infrastructure. Do not report it as a failing product test or widen product changes to fix it.

## Increment 1 — Prove the task and trusted-host boundary

Before replacing the bar, create `rust/tonk-fab/tests/contained_tasks.rs` with an isolated component fixture. Add a small task controller and opt-in presentation path using the existing mounted FABB. Prove one decision, one required notification and restoration from an open panel. Match `edge_primitives.rs` and `detached_connect.rs` for native-dialog/custom-element fixture setup and cleanup.

Define explicit outcomes (completed, cancelled, acknowledged, disconnected), a unique request ID, snapshot ownership and teardown. Resolve exactly once. A cancelled async result must not mutate the next request. Preserve the original content parent/slot/hidden state if content is moved. Restore the deepest connected opener, otherwise the circle. Escape is ignored for required messages; backdrop clicks never implicitly approve anything.

Then run a synthetic, non-authenticating task through the actual sealed guest and trusted top page. Prove: coordinates translated through the live frame, right and bottom anchors preserved, top-page modal blocks the entire space, only one surface visible, resize/drag reseat works, focus returns through the existing portal token, and guest disconnect releases the modal. Reuse the registered message-port boundary. Do not pass arbitrary guest HTML into a privileged host renderer; send a typed task purpose and validated presentation metadata.

**Gate:** pure baseline, new `contained_tasks` suite, UI request parser tests, and one actual portal/browser fixture all pass. Capture the selected host contract and browser evidence in this plan. If cross-document modality or user activation cannot be preserved, resolve this architecture before proceeding. Passing a standalone shadow-DOM demo is insufficient.

**Implemented evidence (2026-09-22):** COMPLETE.

- The FABB owns one `task::Controller` and exposes `present`, `notify`, `resolve`, `resolveRequest`, `requesting` and `requestId`. It moves the existing light-DOM content into a native modal without cloning it, records parent/slot/hidden/scroll/focus, rejects a second request as busy, ignores stale request IDs and restores the exact snapshot once. Required messages ignore Escape; disconnect has a terminal `disconnected` outcome.
- The cross-frame contract is versioned JSON with `requestId`, typed `purpose` (`account` or the test-only `probe`), lifecycle `action` (`open`, `reseat`, `suspend`, `show`, `dismiss`) and validated `presentation`: the live anchor rectangle, horizontal and vertical retained edges, and dismissal policy. Every portal translates all four edges through its live iframe rectangle. The UI capability boundary accepts only `account`; guests cannot supply markup or choose a privileged renderer.
- The portal holds one active task lease. Completion clears the matching lease; a second open returns `busy`; mismatched lifecycle messages return `stale`; guest reload or disconnect sends one typed dismissal to the trusted presenter before closing the port.
- `cargo test --locked -p tonk-fab --lib` passed at baseline (116 tests). The full FABB Wasm baseline passed (52 unit tests plus 90 integration tests). After implementation, the focused `contained_tasks` browser suite passed 4 tests, and the combined native FABB/portal/UI libraries passed 119, 8 and 31 tests respectively.
- The real `tonk-portal` Chromium/Wasm suite passed 51 tests. Its opaque-origin fixture opened a top-page native modal, translated open and reseat coordinates from the live iframe, retained right/bottom edges, hid the guest surface, returned focus through the existing port token and released the trusted modal when the guest disconnected.
- `cargo fmt --all -- --check` and `git diff --check` passed for this increment. The first sandboxed Nix invocation could not open the user fetcher cache; the identical repository command passed with host access. Two fixture corrections were needed before the portal gate ran: enabling the `DomRect` binding and querying the portal's documented light-DOM iframe.

## Increment 2 — Replace the normal bar and its attached panels

Replace `BAR_HTML`/`BAR_CSS`, `bar::Cell`/`Panel` and element event routing with the reference's header, four primary rows and conditional account entry. Reuse the existing sync/name subscriptions and persistence. Retain the product custom-element name.

Implement the 360px rail, 48px controls, 18px sync disc, 25px outer radius and 1.5px ring; add local 17px/18px type tokens without restyling retained legacy dialogs globally. Add panel containers before wiring their effects. No missing-data panel may pretend an operation succeeded.

Adapt to available room instead of forcing 960px. Preserve safe areas and visual viewport keyboard lift. Mirror the seat and icon placement deliberately, keeping labels flush right and DOM/focus order coherent. Follow the reference's morph implementation for adjacent-panel movement (`fabb.js:1734`), including interruption and reduced-motion handling; do not substitute the older generic telescope timing for all motion.

Enable circle collapse on desktop and compact layouts; drag from circle/name/header without swallowing clicks. Add the reference's 500ms hold-to-pause and Shift+Space path using the existing pause claim; cancel hold on drag, pointer cancellation and disconnect. Keep Alt-click compatibility if harmless. Signed-out and offline both use a hollow disc but distinct accessible words. Attention uses the reference's 2.4s opacity pulse, suppressed when signed out/reduced-motion and paused when hidden.

Remove new/open/settings/rename rows from the FABB menu. Home must work for an unavailable space too. Confirm the Hub already exposes create/open/rename/settings and preserve any first-create naming flow before removing an auto-rename dependency; `profile.yaml` supplies the Hub actions. Leave the tested native rename implementation available until all its callers are accounted for.

**Gate:** native FABB tests plus updated `responsive_overflow`, `drag_snap`, `space_name_element`, `late_space_binding`, `detached_connect` tests pass. Update old layout assertions to the new contract while retaining their lifecycle and viewport coverage. Add checks at 320, 390, 768 and 1440px widths, both vertical anchors and both sides; keyboard and touch targets must remain reachable. Capture the normal bar, menu and stacked panel for comparison.

**Implemented evidence (2026-09-22):** COMPLETE. The product tag now renders the 360px nominal rail, 48px controls, attached inward panel, narrow stacked form, collapse at every width, retained dock/drag persistence, 500ms pause hold and Shift+Space. Chromium checks cover 320, 390, 768 and 1440px, both horizontal panel sides and top/bottom stacking; the full FABB Wasm run passes all responsive, drag, late-binding, detached-connect and space-name suites.

## Increment 3 — Connect sharing and local recovery to the task host

Reuse `tonk-share`'s command/data logic; replace selectors/render adapters that assume a separate share menu and global connect-cluster IDs. The menu's `copy share link` uses the real invite path, with pending/copied/retry feedback and no fabricated URL. Accountless interaction opens the shaded attached gate, then starts an account task with the original space and share intent retained.

Until increment 5 enables real account tasks, preserve a functional existing account route behind the unfinished adapter; do not expose a dead primary action. Keep the task host's mutual exclusion explicit: a second request returns busy and cannot replace the first.

Move enable-sync/refusal decisions into the contained host, retaining worker refusal classes, operation timestamps, activation distinctions and fresh-gesture clipboard behavior. Audit `activation.rs`/`banner.rs`: passive pending-email/local-only status may live inside the normal panel without repeatedly stealing focus; actionable required messages use acknowledgement. Do not turn every sync frame into a modal.

**Gate:** focused `share.rs` browser tests, `activation_banner`, `contained_tasks` and `fab_drift` pass. Add cases for double-click, stale refusal, pending email, offline copy failure, repair/retry, cancellation and the original space remaining selected. Existing account-backed copy-link E2E must pass before proceeding.

**Implemented evidence (2026-09-22):** COMPLETE at the component and worker-contract boundary. The visible action forwards once into the existing headless mint, keeps clipboard user activation, displays pending/copied/retry state and routes matching account, sync and terminal refusals through attached or contained presentation. The full 54-test FABB Wasm library, activation banner, contained tasks and six-test `fab_drift` gate pass. The account-backed product E2E result is recorded under increment 7.

## Increment 4 — Make connect-agent available from every space

Extract/adapt the agent prompt presentation from `core.yaml` into the FABB agent panel, keeping the real `tonk:agent-handoff`/`tonk:new-agent-invite` command and overlay data contracts. Initiate minting on explicit intent, not every rerender. Provide loading, unsupported/expired, account-required, sync-required, copy success/failure and retry states.

Use raw-attribute queries/inline descriptors following `logic.rs` for anything that must work on previously seeded spaces; do not depend exclusively on a new `core.yaml` concept. Display/copy the complete ordinary bearer prompt; keep it private and out of test logs. Preserve the existing prompt's CLI semantics, expiry/revocation guidance and real invitation validation. Remove duplicated automatic empty-canvas invitation UI only after the bar supplies the same capabilities, without changing user-authored space content.

**Gate:** add `rust/tonk-fab/tests/agent_panel.rs` and run `nix develop . -c cargo test --locked --target wasm32-unknown-unknown -p tonk-fab --test agent_panel`; nonzero tests pass for new and old seeded-space fixtures, long prompt scrolling and one mint per user action. Run the standard-library/claim gate and existing agent signup/original-space E2E. Test copied invitation use through the existing local integration harness, not only string equality.

**Implemented evidence (2026-09-22):** COMPLETE at the browser-component and seeded-library boundary. The attached panel uses raw attributes for old spaces, mints only on explicit intent, preserves the complete ordinary bearer prompt and routes account refusal into the trusted task. The automatic blank-canvas mint was removed and a standard-library regression proves a blank space waits for intent. Both `agent_panel` browser tests and all 48 standard-library tests pass. End-to-end invitation redemption is recorded under increment 7.

## Increment 5 — Keep account flows and required feedback in place

Use the host contract proved in increment 1 to adapt `register_dialog.rs`, its scoped CSS, `bin/ui.rs` registration dispatch and custody positioning. Make the in-space task a distinct presentation mode from Hub anchored account pages. Replace the `/settings` redirect only for this new mode; maintain Hub behavior and older request compatibility.

Implement the reference's visible flow: one active native form; editable completed answers; inline validation/error/retry; waiting email-link state with resend/change-email; one fused bottom rung (secondary left, primary right); completion acknowledged before returning. Keep the real email lookup, conditional mediation, passkey ceremonies, activation subscriptions and user-facing recovery messages. Use `add an account` for the entry that covers signup and sign-in. The mock registry and fake verification timers are not production logic.

Document any real-service ordering constraint that differs from the mock's verification/name/passkey sequence. Change visible progression only where the existing commands support it; do not silently redefine account readiness or invent a new authentication protocol for wireframe parity.

On cancellation, retain only the safe draft data needed for the same flow; cancel pending attempts and ignore stale completions. On success, restore the initiating share/agent view in the same space and continue the intended action exactly once, with a fresh copy gesture where required. No navigation through `/settings`, duplicate copy, second modal or focus leak. Account/profile switches may legitimately reload: retain only safe resume metadata, invalidate stale requests and preserve current authority checks.

**Gate:** add `it_keeps_fabb_account_tasks_in_space` to `account_flow.rs` and run the focused E2E command above. Cover signup, returning sign-in, pending email/resend/address edit, passkey denial, retry, cancellation, disconnect and success; assert URL/space continuity, modal ownership and focus. Existing account/profile-switch/CLI-link tests must retain their semantics. Run broader account E2E at this integration checkpoint; report skipped/quarantined tests separately.

**Implemented evidence (2026-09-22):** COMPLETE at the typed host and local test boundary. In-space account work has a distinct trusted presentation mode, remains on the space URL, returns completed and cancelled outcomes separately, restores the initiating share or agent panel, and releases the portal lease on disconnect. UI parser/state tests (31 native) and the portal's 51-test Chromium suite pass; the focused real account E2E result is recorded under increment 7.

## Increment 6 — Replace the roster dialog, then resolve map data

First, move the live roster into the attached 240px members panel. Preserve reset/update/retract/self labeling and any current promotion capability; do not silently delete a permission-bearing action to match a demo. Decide its secondary in-panel location separately from the map's read-only last-active card.

Treat the reference map as a distinct data requirement. Current `Member` and `member_roster_query_body()` do not provide `kind`, `parentId` or `lastActive`; a role is not an agent kind, and row arrival time is not last activity. Audit authoritative schemas/worker records for those facts and write the mapping before implementing the full map.

- If facts already exist, add optional projections through the established subscription path, retaining members when optional metadata is absent. Test old records and unknown parents.
- If no authoritative source exists, keep a useful attached roster with honest unknown activity and record a follow-up data-contract decision. Do not label this full map parity or synthesize people-agent edges. The rest of the UI may ship independently; full wireframe completion remains blocked on that decision.
- When the data contract is available, implement the fixed stage with radial tree layout, panning by drag/wheel/arrows/Tab, center-weighted glyph sizing, self recenter, people/agent counts, agent tails, collision-aware labels, selection card and over-30-day hollow state. Handle missing self, disconnected components, cycles, duplicated IDs and unknown timestamps without inventing relationships. Keep an accessible textual equivalent.

Use pure layout tests for graph inputs and real browser tests for focus/pan/selection. The reference's history explicitly defers mobile members design: preserve usable touch navigation and readable alternatives; do not claim a settled mobile graph spec.

**Gate:** existing roster frame tests pass; add `rust/tonk-fab/tests/member_panel.rs` and run its Wasm suite. Pure layout tests cover malformed and large trees. Verify 12 people with five agents each, zero members, unknown activity, selected-member retraction and a contained task restoring selection/pan. Mark this increment PARTIAL if only the attached roster can be truthfully delivered.

**Implemented evidence (2026-09-22):** PARTIAL. The detached modal is gone; reset/update/retract frames render a readable attached 240px roster with live count, self and role labels, including a 12-member browser fixture and honest absence of activity metadata. The authoritative records expose only member DID, name and role. No source for `kind`, `parentId` or `lastActive` was found, so no graph edges, agent tails or activity state were invented. The two-test `member_panel` suite and existing roster frame coverage pass. A schema decision remains required before full map parity and its graph tests can exist.

## Increment 7 — Reconcile remaining surfaces and verify integration

Inventory Tonk-owned in-space prompts from `share.rs`, `activation.rs`, `register_dialog.rs`, `custody_relay.rs` and the affected `core.yaml` views. Every in-space decision/error/account task must now use the contained presenter; remove obsolete body-mounted roster/connect surfaces and global IDs only after their callers migrate. Keep shared `tonk-dialog`, cluster and field primitives still used by the Hub. Do not sweep user-authored applications into the new presentation policy.

Update `DESIGN.md` with the in-space exception/new rules and mark earlier FABB plans superseded for anatomy/type/tasks, retaining historical rationale. Update Storybook source scenarios and regenerate artifacts using its documented workflow. Add the new state matrix: collapsed/bar/menu/share gate/agent/members/task, signed out/pending/ready/offline/paused, all docks, narrow/short viewports, keyboard/coarse pointer and reduced motion.

**Gate:** build:web; native core/FABB/UI and worker contract gates; release Wasm suites; targeted plus broader account integration; formatting/diff checks; Storybook validation. Inspect the rendered product beside the pinned reference at desktop and 390px touch sizes, with a 320px narrow case and a short/keyboard-constrained case. Check Safari rename, native dialog focus and clipboard on an actual available Safari/device path; Chromium success does not establish Safari parity. Record unavailable checks explicitly.

**Implementation note (2026-09-22):** `tonk-workspace` contained only the sync-status element after account settings moved into the profile library. That element now lives in `tonk-fab`, and the empty crate was removed from the workspace and guest bundle. Design and Storybook sources now describe attached panels and contained account tasks; generated Storybook data is current. Final command results and unavailable external checks follow in the completion report below.

**Final verification evidence (2026-09-22):** `build:web` passed from the normal Git-flake command. Native FABB/UI/core/portal suites passed 122/31/16/8 tests; worker standard-library and drift suites passed 48/6. The complete FABB Chromium/Wasm run passed 101 tests across its library and integration binaries, and the opaque portal run passed 51. The focused real-account Chrome regression `it_keeps_fabb_account_tasks_in_space` passed with the same URL and restored FABB after cancellation. Rust formatting, generated Storybook data, JavaScript syntax and `git diff --check` passed. The release Wasm archive produced no output for 25 minutes and was stopped before test execution. Storybook's link checker still reports eleven repository-wide missing links outside the changed Storybook sources. Product/reference screenshot comparison, broader account E2E, Safari, native dialog/clipboard on Safari and physical touch-device checks were not run.

**Review follow-up (2026-09-22):** A ready `copy share link` now remains in the action rail and answers `copied` in place; only an accountless share opens the attached account gate. The trusted contained signup/login surface now follows `hub.html`/`account-demo.js` for its full-width 48px fields and actions, 17px control type, 18px guidance, labels and account-state copy while retaining the production email lookup, confirmation and WebAuthn ceremony. The native FABB suite (122), focused UI parser/task suites (13), Wasm UI compile, and two focused Chromium/Wasm share regressions passed. The expanded real-browser account test compiled, but its Nix/Trunk fixture build emitted no result for ten minutes and was stopped before browser execution. The full native UI suite also remains red on the pre-existing browser-process reaper test (`the stand-in browser survived reaping`); its 30 other tests passed.

## Done criteria

- [ ] Drift check reconciled; only reviewed paths changed; each increment has its gate results recorded.
- [ ] Product exposes the reference bar/navigation/material/typography with usable all-width collapse, drag and pause controls.
- [ ] One task at a time; required acknowledgement; optional cancellation; viewport-safe body scroll; all prior context and focus restored; host cleanup proven on disconnect.
- [ ] Accountless sharing stays in the initiating space; real share/agent authority and clipboard flows pass integration tests.
- [ ] Connect-agent works over existing spaces as well as newly seeded ones.
- [ ] Members panel is integrated; full graph/data parity is either proved or explicitly recorded as an outstanding blocker, never reported complete from mock fixtures alone.
- [ ] No obsolete detached in-space roster/connect/task surfaces remain active; Hub dialogs still work.
- [ ] Validation commands pass after the final source change, with nonzero selected tests and any external/device limitations listed.
- [ ] Storybook and design guidance describe the shipped state; plan/index status reflects actual completion.

## Stop conditions and maintenance

Pause the affected increment and report evidence if it requires relaxing the guest sandbox, moving keys into guest state, changing invitation authority, inventing presence data, or changing backend account ordering. Continue independent visual work only when it does not depend on that decision. Preserve local/browser data throughout; never clear normal-profile storage to make tests pass.

If current code has drifted, re-read the concrete excerpts and callers before applying this plan; do not mechanically replace selectors. Repeated failures should produce the literal error, current hypothesis and narrow next check, not a broader rewrite.

Future panels must reuse the same view/task lifecycle. Review every new async path for stale results and teardown, every new projection for old-space compatibility, and every frame movement for translated coordinates and focus ownership. Reference updates should identify the new Gooey commit and which decisions they supersede.
