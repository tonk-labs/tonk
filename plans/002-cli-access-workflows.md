# CLI access workflow simplification

Status: LOCAL IMPLEMENTATION VERIFIED. User instruction supersedes Plan 001's legacy command compatibility on 2026-09-16.

Only two ordinary CLI access workflows remain: humans use `tonk link` for browser-selected scoped grants; agents use `tonk connect` for one-way scoped invitations without browser approval. Remove `join` entirely, including `--agent`. Reject older sharing/account-approval invitations and legacy URL-free resumes before changing account or space state; never fall back to account credentials.

Keep existing local data, scoped import recovery, terminal linking, and separately scoped administrative/recovery APIs intact. Do not deploy, publish, or withdraw credentials. Browser issuance remains subject to the release gates in Plan 001; old prompts require regeneration from a deployment supporting scoped invitations.

Verification: parser/help removal and rejected account flags; real process rejection of old invitation formats and legacy resume with unchanged registry/credentials; existing scoped import/confirmation/crash recovery; full CLI integration checkpoint. Update current guides and Storybook while keeping historical evidence labeled.

## Progress

- Baseline executable offered `join --agent` and `connect --no-open/--via/--switch-account`; source dispatched both into account approval.
- Removed parser/dispatch entries and the executable legacy claim/account approval/resume implementation. `connect` accepts only scoped envelopes, and URL-free resume requires a scoped local binding.
- Added a real-process regression for retired commands/old link formats/legacy resume, retaining exact local files and suppressing link secrets. Updated guides, help and retired browser test coverage.
- Removed account-switch/browser-approval copy from core and generated playground prompts. Old envelopes have no copy action. Feature-disabled browser issuance returns an unavailable state instead of minting account-bound handoffs; release gating remains intact.
- First parser checkpoint passed 35 and failed two stale guide/flag examples; both were updated to `connect`. Initial CLI space checkpoint passed 61 and failed two stale help assertions expecting `--agent`; updated to assert its absence.
- Full CLI, browser library and final browser artifact checks are running. No credentials, local data or generated replica directories were removed.

- Updated parser/help gate passes 37 tests. The freshly built process preservation regression passes (3.25s): removed join modes, old invitation formats, and legacy resume all refuse; exact registry/replica files and shared credential files remain unchanged, and secret link material is absent from errors.

- Full CLI integration checkpoint: 628 passed, 0 failed, 2 ignored across 33 targets. The ignored cases are the environment-dependent released-CLI executable gate and the existing schema-introspection analyzer port. Both were already identified in Plan 001; this run does not claim they ran.
- Final browser artifact `eebc5c6a953f7630`, worker `329ed62028149d06`, manifest `112bdb39b464085a2e31d5abb1c39e8e8e6d994c5bde2a008d66bc49031218a7` builds successfully. Actual multiple-holder agent copy/closed-issuer/import/revoke journey passes (84.48s); one/many/all-current browser-selected terminal linking passes (78.18s). Both use the new CLI with join removed. Existing surviving native browser fixtures were reused against the fresh Wasm artifact and final CLI; the removed legacy test is no longer part of the source suite.
- Browser captures remain local in `/private/tmp/tonk-two-workflows-captures`; older committed captures keep their original provenance. Full workspace lint and the changed standard-library target remain pending.

- Strict all-target/all-feature workspace Clippy passes. The browser-library target initially passed 32/33; the remaining assertion expected the removed legacy prompt's same-invite reclaim instruction. Updated it to require the scoped prompt's fresh-invitation recovery instruction. No production code changed for this correction.

- Corrected browser-library gate passes all 33 tests; strict workspace Clippy passes again after that test edit. Formatting/whitespace and Storybook generation/199 links pass.
- Binary packaging audit found the tested CLI had current command/help behavior but still embedded the older core seed template because the first compilation overlapped the asset edit. The extra build was initially stopped as apparently redundant; this audit showed it was needed. Rebuilding from frozen sources, then checking embedded template bytes and re-running scoped process/browser checks against that exact executable. The 628-test checkpoint remains command/regression evidence, not evidence for the final embedded template.

## Final checkpoint

- Final CLI build succeeds from frozen sources. SHA-256: `97612a2b4abbfd4b866ef8441a2822e31421ed912721ff131ce18966f9d18ae1`. Binary inspection confirms the new seed prompt is embedded and the retired account-approval prompt is absent.
- All five scoped process tests pass against that exact executable (15.06s), covering old-flow/flag refusal, exact state and credential preservation, scoped import/confirmation and interrupted recovery.
- Both actual browser journeys pass again against that executable and browser artifact `eebc5c6a953f7630`: multiple-holder agent invitation/closed issuer/unrelated CLI account/revocation (68.00s); human one/many/all-current selected-space linking (62.13s). These replace the earlier command-only binary checkpoint for final artifact evidence.
- Full CLI command/regression checkpoint: 628 passed, two explicitly ignored. Final standard library: 33 passed. Final strict workspace Clippy, formatting, whitespace, Storybook generation and all 199 local links pass.
- No deployment, publication, external credential changes or automatic account conversion occurred. The existing three untracked generated replica directories remain untouched. Browser issuance retains the release gate, and disabled deployments no longer mint legacy account-bound agent invitations.

## Local development follow-up

- Manual localhost testing exposed a stale running `.Trunk.dev.toml` without `/connection/`: an unsigned POST to `/connection/read` returned 405 through port 8080 and the expected 401 directly from the existing access service. This CLI refusal is independent of the browser's HTTP warning. The current dev command already generates the required proxy; existing dev shells and servers need to load it.
- `dev:web` now generates a local HTML target enabling `connection-invites` on the UI and worker pipelines only. Production defaults remain gated; the guest pipeline is unchanged. CLI polling failures now include the HTTP status and endpoint guidance.
- Nix formatting, Rust formatting, whitespace checks, executable HTML-generation assertions, and `cargo check --offline -p tonk-cli --bin tonk` pass. The user's running server was not restarted, and the changed dev command has not had a fresh full browser build in this follow-up.

## Terminal picker simplification

Scope: `/settings/link` selected-space approval only. Applied `better-interface` and all six domain skills using `DESIGN.md`, the existing Rust DOM handlers, the HTML template, and shared Tonk CSS tokens. Administrative connection lists and legacy account approval are outside this change.

| Domain | Evidence inspected | Result |
| --- | --- | --- |
| Accessibility | Native full-row labels; browser accessibility tree; keyboard focus; actual Wasm selection test | Named checkboxes and buttons, 2px focus outline, unavailable spaces excluded |
| Layout | Source-template previews at desktop and actual 320px emulation, long names, RTL, 2x CSS zoom | No horizontal overflow; actions in normal flow |
| Writing | Loading, available, unavailable, empty, error, retry and success source paths | Short action-oriented copy; identifiers and absolute deadline removed |
| Typography | Existing Plex font assets and source-template screenshots | Clear heading; names wrap; product lowercase action labels preserved |
| Colors | Computed text/background pairs in both themes | Light ratios 15.26 / 7.71 / 14.60; dark 12.19 / 9.60 / 12.65 for row / explanation / primary action |
| UI polish | Existing rectangular controls, selected row, focus and disabled source states | Token-based surfaces and checkbox accent; no added motion |

| Severity | Domain | Location | Before | After | Why |
| --- | --- | --- | --- | --- | --- |
| MEDIUM | Writing | `rust/tonk-workspace/src/ui_account_settings.html:65`; `rust/tonk-workspace/src/terminal_link.rs:206` | Visible terminal, account and space DIDs, technical counts and timestamp | Heading, short permission summary and space names | The choice should be understandable without protocol knowledge |
| MEDIUM | Layout | `rust/tonk-ui/styles.css:2217` | Constrained settings panel, tall metadata rows | Full-width selectable rows and normal page scrolling | Keeps spaces and actions easy to scan and reach |
| LOW | UI polish | `rust/tonk-workspace/src/terminal_link.rs:248` | Generic action label and repeated selection summary | `link 1 space` / `link N spaces` | Puts the selection count in the action itself |

Verification: `cargo test --offline -p tonk-workspace --target wasm32-unknown-unknown terminal_link::tests -- --nocapture` passes (1 test). Assertions cover eligible-only select-all, mixed selection, disabled empty selection, full-row labels, absence of visible DIDs, and singular/plural action text. Initial sandbox browser startup failed; the unchanged test passed with local browser access. Eight pre-existing dead-code warnings came from `tonk-fab` test dependencies. `cargo fmt --all -- --check` and `git diff --check` pass.

Strict Wasm Clippy fails on four pre-existing warnings in unchanged `ui_space_remove.rs`, `agent_connections.rs`, and `terminal_connections.rs`; these were not suppressed or edited. Full live browser-to-CLI approval, Safari, and screen-reader operation were not rerun. Browser visual evidence uses the actual template and CSS with representative rows, not an authenticated live account. Empty/error/success copy was inspected in source, not exercised end to end.

Verdict: Approve for the scoped picker presentation; no high-severity interface findings remain in the inspected evidence. The lint and runtime verification limits above remain explicit.

### Row layout refinement

User clarification replaces the enclosing picker panel with separate intro, space, and footer surfaces aligned to the account bar. Each space explicitly fills the available width; gaps follow the existing 7px row stack. Actions are now 32px high. The scoped Wasm selection test passes again. Source-template browser inspection measures matching 580px widths on desktop and 296px at a 320px viewport, no horizontal overflow, and no outer background or border. Both buttons measure 32px. Full authenticated live approval remains outside this presentation check.

- Follow-up: Select all / Refresh now occupy their own boxed row. Cancel / Link spaces are siblings below the permission box, on an unboxed action row; buttons retain their compact 32px sizing. Scoped HTML structure and whitespace checks pass. No approval logic changed.

- Palette/copy follow-up: Refresh moved to the heading row; the select-all row now contains only selection. Fixed copy is lowercase, with terminal labels and space names preserved. The chartreuse came from Web Awesome's custom checkbox `--wa-form-control-activated-color` and `--checked-icon-color`, so the picker overrides those with `--ink` / `--on-ink` as well as native `accent-color`. Verified the preview with the actual `assets/guest/wa.css` loaded: checked fill is dark-theme ink, checkmarks use on-ink, and the mixed state renders the same palette. Desktop and 320px screenshots have no horizontal overflow. Focused Wasm selection test and formatting/whitespace checks pass. Full live approval was not rerun.
