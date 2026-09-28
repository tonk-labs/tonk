# Plan 007: Separate agent invitations from named space access

Status: DONE for checkpoints 1–3; local integration gates passed. Checkpoint 4 is out of scope.
Priority: P1. Effort: M for checkpoints 1–3; L for the optional redemption protocol.
Risk: medium for presentation and receipts; high for authority changes.
Planned at `503a5d637`, 2026-09-28. Category: product / architecture.

## Intent

Opening the connect-agent panel continues to generate an invitation without an
extra action. Settings ignores invitations without a completed setup receipt and
shows manageable agent delegations under the space they grant. A CLI should supply a recognizable connection label.
The UI must distinguish labels from authorization identity and show the actual
unit of revocation.

This plan preserves the existing authority model in checkpoints 1–3. Single-use
invitations with independent device grants are a separate proposed checkpoint,
not an assumed requirement or an existing capability.

## Current state and drift check

Before editing run `git status --short` and
`git diff --stat 503a5d637..HEAD -- rust/tonk-fab rust/tonk-worker rust/tonk-worker-api rust/tonk-schema rust/tonk-cli rust/tonk-invite rust/tonk-core/assets/library`.
Compare changed code with these anchors and update the plan if needed.

- `rust/tonk-fab/src/agent_panel.rs`: opening the panel calls
  `dispatch_handoff(&host, &state, true)`. It reuses the mounted panel's link,
  but requests fresh authority when that transient state is lost.
- `rust/tonk-worker/src/router/repository.rs`: the handoff already has a
  `!fresh` reuse/history guard. It intentionally does not persist bearer URLs.
- `rust/tonk-worker/src/router/agent_connections.rs`: `mint` writes a public
  account-scoped grant ledger. Its label is space name plus issue timestamp.
  `summarize` distinguishes authority expiry/revocation from `confirmed`.
  `confirmation` reads ordinary space-main data at the grant-set entity.
- `rust/tonk-cli/src/connections.rs`: import uses
  `Ed25519Signer::import(connection.invite.secret_seed())`. Every holder of the
  same URL therefore shares the same signing identity, not just the same label.
- `rust/tonk-cli/src/handoff.rs`: `record_scoped_connection` writes one fixed
  status to `id:tonk:agent-connection:{grant_set_id}`. Copies share this receipt.
- `rust/tonk-core/assets/library/profile.yaml`: account settings renders every
  ordinary grant as a flat record. Confirmation is secondary explanatory text.
  Verify the currently mounted `/settings` surface before editing: the supplied
  screenshot does not show this section, and routes/feature gates may differ.
- `rust/tonk-invite/src/connection.rs`: current signed bearer format and exact
  space scopes; no atomic redemption protocol exists here.

Plan 005 deliberately retained invitation identities and rejected a shared CLI
device key. Do not silently undo that decision. Preserve separate tool/person
flows, isolated credentials, original signed scopes, and revocation ancestry.
Follow `DESIGN.md`: existing stone surfaces, aubergine ink, compact controls,
textual statuses, and existing responsive layouts.

## Product contract

1. Preserve invitation generation on opening connect agent, including existing
   in-memory reuse. Do not add a create-invitation step. Preserve uncertain-retry
   behavior so a timeout cannot silently mint twice.
2. Settings groups records by stable space DID and shows the current display
   name, with DID fallback. Include only grants issued by the active
   account and label that scope: this is not a complete space-wide agent census.
3. Show only setup-confirmed delegations in management. Do not show pending
   invitations, counts, or empty space headings caused by unconfirmed invitations.
   Expired/revoked confirmed records belong in history. Partial revocations of
   managed delegations stay prominent with retry.
4. Use the completed setup receipt as the visibility criterion, not an
   authorization gate. A missing receipt does not prove non-use; filtering
   settings neither consumes nor revokes the bearer.
5. Offer `tonk join LINK --agent-name "Codex on work laptop"`. Keep `--name`
   exclusively as the existing local space alias. Default to the existing CLI
   OS-based device description; do not collect a hostname or local path by default.
6. Store a random local installation identifier and bounded self-reported label
   outside replicated content, then publish a receipt keyed by grant plus that
   installation identifier. Persist before publication; retries reuse it.
   This distinguishes reported installations, not physical hardware or verified
   software. Copies of credentials still share authority.
7. Multiple reported installations under one grant remain visibly grouped.
   The action removes that grant's access for all holders. Do not put a pretend
   per-device revoke button on a shared grant. Existing status-only receipts
   appear as `Name unavailable`.
8. Retain the public ledger and revocation machinery for unconfirmed grants,
   but omit them from normal settings management as requested. Do not delete or
   revoke grants as a side effect of filtering. Each separately minted link
   already has its own independently revocable grant set; only copies of the
   same link share a revocation boundary.

## Scope and checkpoints

### 1. Filter management to completed connections

In the actual settings renderer, filter ordinary grants by their existing
`confirmed` projection before rendering records or deciding the empty state.
Preserve the underlying ledger and mint-on-open behavior. Add a focused test
showing that twenty unconfirmed invitations produce no management rows, while
one confirmed invitation produces one manageable delegation. Keep unrelated
terminal connection handling intact.

Gate: run the named settings browser regression through
`nix develop . -c test:e2e -E 'test(NAME)'` using the actual new test name.
Also run `nix develop . -c cargo test --locked --target wasm32-unknown-unknown -p tonk-fab --test agent_panel`
to verify existing mint-on-open, account recovery, and retry behavior remains.
No create-invitation button should be introduced.

### 2. Publish named, resumable setup receipts

Modify `rust/tonk-cli/src/bin/tonk.rs`, `connections.rs`, `handoff.rs`,
`rust/tonk-schema/src/agent_connection.rs`, and focused CLI tests. Add optional
receipt concepts rather than making legacy confirmation queries require new
attributes. Keep local metadata backward compatible. Validate bounded labels,
serialize them as values, and render with `textContent`, never HTML or YAML
string interpolation. Keep an installation label stable across URL-free resume.
Only report setup complete after the existing acknowledged receipt push.

Gate: `nix develop . -c cargo test --locked -p tonk-cli --test handoff --test connections --test connection_commands --test connection_compatibility`.
Add tests for legacy manifests, Unicode/invalid labels, interrupted publication,
stable resume identity, and two installations using one invitation. Native tests
alone do not establish remote publication or UI visibility.

### 3. Project and render access per space

Extend `rust/tonk-worker-api/src/agent_connections.rs` additively with optional
space display name and a list of reported installations. Read these in worker
`agent_connections.rs` without conflating confirmation with authorization.
Update the actual settings surface (start with `profile.yaml`) and existing
styles in `rust/tonk-ui/styles.css`. Load failures must remain visible, not be
rendered as an empty account. Retain refresh race protection and partial-revoke
receipts. Preserve the existing feature gate.

Gates:

- `nix develop . -c cargo test --locked -p tonk-worker --features connection-invites connection_management`
- `nix develop . -c cargo test --locked -p tonk-worker --test standard_library --test fab_drift`
- Add and run a named browser integration test in `rust/tonk-ui/src/account_flow.rs`
  covering two spaces, hidden unconfirmed links, named receipts, and revocation retry.
  Use `nix develop . -c test:e2e -E 'test(NAME)'` with the actual new test name.

Require account isolation, duplicate display names, renamed/unavailable spaces,
legacy confirmations, revoked/expired confirmed entries, and malicious label
rendering tests. Inspect desktop and mobile with the invitation-enabled preview.
For the full local journey: create link, join with a label, observe it under the
correct space, revoke it, verify subsequent remote access is denied while local
data remains intact. Observe service propagation rather than assuming instant
revocation. Record exact build identities and any unrun browser/hosted gates.

### 4. Out-of-scope option: independent holders of the same invitation

This is unnecessary for revoking separately minted links and is not part of the
requested management change. Only if single-use links are later selected, build a separate executable protocol
spike in `tonk-invite` and the access service. A receipt or UI flag cannot consume
the current bearer: it already contains a usable private key and signed grants.
New invitation tokens must authorize redemption only, with no space-sync rights.
The CLI generates a per-connection key and proves possession. A durable service
operation atomically binds the invitation to that key and returns the same result
on same-key retries; a competing key must lose. Issue independently revocable
space grants with the original scope and bounded expiry.

Resolve who can sign those grants without exporting account keys or keeping the
browser online. Specify and test service-side authority, cancellation races,
expiry, crash recovery, response loss, and two concurrent claims before changing
production callers. If no acceptable signing/custody design exists, stop this
checkpoint and report the constraint; do not describe client-side receipts as
single-use enforcement. Preserve v2 imports and label their shared grant semantics.
This checkpoint needs its own reviewed implementation plan after the spike.

## Completion, boundaries, and maintenance

- Keep each implemented checkpoint independently reviewable; commit separately
  only if commits are requested. Do not push or deploy as part of this plan.
- Do not alter account login, person invitations, billing, local replica data,
  existing bearer storage policy, or lockfiles for presentation changes.
- Before claiming implementation complete, run relevant gates after the final
  edit, `nix develop . -c cargo fmt --all -- --check`, and `git diff --check`.
- These commands are grounded in repository test configuration and prior plans;
  they were not run during this design-only investigation.
- Stop and reassess if the live settings renderer differs, legacy records cannot
  be read additively, or a label is being used to select authorization.
- Update this plan and its index with actual evidence. Future receipt changes
  must preserve old-client visibility and distinguish missing data from denial.

## Execution log

2026-09-28: drift check against `503a5d637` is empty for the planned source
paths. Initial local changes were this plan and its index only. The mounted
settings implementation remains `account-settings` in `profile.yaml`.

Checkpoint 1 filters ordinary grants by `confirmed` before rows and empty-state
selection. Added `settings_hides_unconfirmed_agent_invitations` on the actual
settings component. Packaged browser and Wasm FAB gates are running; the initial
Nix cache SQLite access error required an unchanged retry with host access.
No commits, push, deployment, or authority-protocol changes are in scope.

Checkpoints 2–3 implementation: `--agent-name` is distinct from the local alias
and accepts 1–100 UTF-8 bytes without controls, bidi overrides, or surrounding
whitespace. A locked, atomic `agent-installation-{grant}.json` sidecar keeps a
random 128-bit identity and the chosen/default OS label outside replica data.
URL-free resume reuses it. Installation receipts use a separate status attribute
so old status-only clients keep one banner per invitation. Completion still
requires the existing acknowledged push.

The worker projects optional current `spaceName` and valid self-reported
`installations` additively; it checks the mounted repository DID and exact grant
and receipt entity. Settings groups confirmed ordinary grants by DID, displays
names via `textContent`, keeps shared-holder revocation explicit, and separates
expired/revoked history from active/partial records. Errors and refresh generation
checks are retained; pending grants remain in the ledger and stay usable.

Intermediate evidence: all 7 Wasm agent-panel tests passed. The initial CLI run
exposed a single-sidecar filename collision when two grants were recorded in one
fixture; keying the sidecar by grant fixed it. The subsequent CLI gate passed
25 tests; its released-executable compatibility test was ignored because
`TONK_OLD_CLI` is not supplied. Two worker management tests passed, including
partial receipts across restart. New fixture tests and final source checks are
running; these intermediate passes are not final completion evidence.

An isolated source-renderer preview was inspected at desktop and 390px mobile:
no horizontal overflow, preserved Unicode names, and 44px removal controls.
This preview uses the actual methods/styles with synthetic data; it does not
prove the packaged application or remote journey. The packaged checkpoint-1
browser gate is still building CLI, UI preview, and test archive artifacts.

Final review: changed repository-load handling to distinguish typed NotFound
from storage failures. Missing local replicas retain the existing unconfirmed
fallback; actual load errors and a mounted DID mismatch now fail the list, so
settings shows its retryable load error instead of silently hiding access.
The worker gate is rerunning with a focused regression for this distinction.
The 53 standard-library/FAB-drift tests and all 4 join parser tests passed.
Formatting corrections were applied; final format/diff checks remain to run.
Earlier packaged build snapshots were cancelled after source changes. The final
four-test packaged run uses retries=0 and writes visual artifacts under
`/private/tmp/agent-mgmt-e2e-artifacts`. No packaged pass is claimed yet.

The final management gate passed 4 tests. Its new native fixture initially
reused `rust/tonk-worker/reported-space`, because the worker operator resolves
named spaces from cwd rather than the profile's explicit directory. Moving
the replica into the fixture's absolute temporary path fixed isolation; a
second run passed without clearing data. The obsolete fixed-name directory
was created by this run and removed. No existing account/replica data was reset.

The packaged snapshot uses source `/nix/store/kp6az60mjfxb5cz98fbn904v5mbxdwiz-source`.
A byte comparison of all 11 changed Rust/library/style/browser-test inputs
found no production or browser-test differences from the current worktree;
only the worker's cfg(test) fixture differs. To avoid the stock wrapper's
build/eval race on that test-only edit, the same nextest e2e profile/filter is
run against pinned CLI, preview, and archive output paths, with retries=0.
Final artifact identities and results will follow below after completion.

Packaged setup boundaries: the stock wrapper completed its build but failed
opening a newly evaluated, unbuilt archive after the test-only fixture edit.
The pinned runner first triggered an unnecessary default-UI build to obtain
the server wrapper; that run was interrupted before Chrome started. Selecting
an existing server script (verified equal to the current flake apart from store
dependency paths) avoids that build. The next four tests all failed before app
interaction because Nix ChromeDriver 152 cannot create an installed Chrome 154
session. The unchanged four-test gate is rerunning with verified ChromeDriver
154.0.8037.57, localhost, the matching server, and retries disabled. These setup
failures are not counted as passing browser evidence.

Final evidence after the last production edit:

- CLI handoff/connections/commands/compatibility gate: 25 passed. The one
  released-executable compatibility test is ignored without `TONK_OLD_CLI`;
  native legacy manifest and status-only receipt coverage passed.
- Join parser gate (`cargo test --locked -p tonk-cli --bin tonk join_`): 4 passed.
- Worker management gate (`cargo test --locked -p tonk-worker --features
  connection-invites --lib connection_management`): 4 passed; the real projection
  fixture also passed an immediate second run without clearing storage.
- Worker standard-library/FAB-drift gate: 53 passed.
- Wasm `tonk-fab --test agent_panel` gate: 7 passed.
- Packaged browser run `214e640e-b770-4644-834f-4cae81991ec7`: all 4 selected
  tests passed in 38.660s with retries=0. This includes the twenty-pending/one-
  confirmed mounted settings regression, named grouping/legacy/history/partial-
  retry/error/race/escaped-label assertions, named receipt publication and resume
  after issuer closure, and copied-link/account isolation plus denied remote
  access after revocation while downloaded data survives.
- Actual packaged screenshots inspected at 1200×900, 390×844, and 390×540 dark:
  labels remain literal, shared-holder action is visible, controls have 44px
  targets, keyboard focus has a visible ring, reduced-motion controls do not
  animate, and inner/outer documents have no horizontal overflow. Captures:
  `/private/tmp/agent-mgmt-e2e-artifacts/{connection,connection-two,named-access}-{desktop,narrow,short-dark}.png`.
- Final `nix develop . -c cargo fmt --all -- --check` and `git diff --check` passed.

Exact packaged identities (production/browser source equivalence checked above):

- CLI: `/nix/store/h31s3bmpnsv879n1vjpjnyxgsa0g1y3x-tonk-cli-0.1.0`.
- Invitation-enabled signed preview:
  `/nix/store/5xhiqi7cwp9m9a4pxw6ghpy08cwk1i4h-tonk-ui-preview-0.6.16-rc.1`,
  service-worker build `c7b0869da0deadae`.
- Test archive:
  `/nix/store/p5rv9s20x6qcckmcfp13j4wbmrcxhqa0-tests-e2e-0.6.16-rc.1/tests-e2e.tar.zst`.
- Matching server:
  `/nix/store/54piy83l8wgfzclwnadwmjlhgrbj61r6-tonk-ui-test-server/bin/tonk-ui-test-server`.
- Browser: installed Chrome `154.0.8037.58`; ChromeDriver `154.0.8037.57`.

The successful invocation inside `nix develop .` was:

```sh
export DO_NOT_TRACK=1 TONK_TEST_WEB_HOST=localhost
export CHROMEDRIVER=/private/tmp/pr1008-driver/chromedriver-mac-arm64/chromedriver
export TONK_BIN=/nix/store/h31s3bmpnsv879n1vjpjnyxgsa0g1y3x-tonk-cli-0.1.0/bin/tonk
export TONK_UI_TEST_ARTIFACT=/nix/store/5xhiqi7cwp9m9a4pxw6ghpy08cwk1i4h-tonk-ui-preview-0.6.16-rc.1
export TONK_UI_TEST_SERVER=/nix/store/54piy83l8wgfzclwnadwmjlhgrbj61r6-tonk-ui-test-server/bin/tonk-ui-test-server
export TONK_HANDOFF_TEST_ARTIFACTS=/private/tmp/agent-mgmt-e2e-artifacts
cargo nextest run --profile e2e --workspace-remap ./ \
  --archive-file /nix/store/p5rv9s20x6qcckmcfp13j4wbmrcxhqa0-tests-e2e-0.6.16-rc.1/tests-e2e.tar.zst \
  -E 'test(settings_hides_unconfirmed_agent_invitations) | test(settings_groups_named_agent_access_and_retries_removal) | test(it_connects_with_an_ordinary_bearer_after_the_issuer_closes) | test(it_keeps_copied_agent_grants_independent_of_cli_accounts)' \
  --retries 0
```

These are fresh local-service/browser results, not hosted CI or deployment
evidence. No commit, push, deployment, lockfile change, single-use protocol,
or account/local-data cleanup was performed. The other 96 browser tests were
outside this focused gate; the old released CLI executable was not supplied.

### Rebase follow-up, 2026-09-28

The user rebased onto main with autostash. The rebase finished at `c1f3b4400`;
the two remaining conflicts came from restoring the uncommitted feature work.
Fresh `git fetch origin main` confirms HEAD equals `origin/main`. Resolution
keeps main's routed settings and account/passkey/sign-out/deletion panels,
mounts agent management as a full-width panel within that layout, and retains
the confirmed-only grouping and shared-holder controls. The browser journeys
keep main's direct API revocation checks plus the named installation assertions;
the two mounted settings regressions and capture helper remain. Main's layout
regressions now expect the panel and its existing API feature-gated visibility.

All conflict entries are resolved and staged; the autostash remains available
as a safety copy. The plan remains untracked as it was before the rebase. No
commit, push, or second rebase was performed. Post-rebase standard-library and
FAB-drift checks passed 56 tests; formatting and staged diff checks passed.
Browser-test compilation initially found a screenshot helper removed by main
but still needed by the restored visual regression. Restoring that helper and
gating both capture helpers on `connection-invites` fixed the error. Final
`nix develop . -c cargo test --locked -p tonk-ui --features
web-integration-tests,connection-invites --lib --no-run` passed, producing
`target/debug/deps/tonk_ui-2c3b2698a9bdf735`; its test inventory includes both new
settings regressions, both remote journeys, and main's account-layout test.
Final formatting and diff checks passed after this helper restoration. Runtime
browser tests were not rerun after the rebase: the packaged results above
belong to the pre-rebase source and are not a runtime pass for this new layout.

### Settings UI refinement, 2026-09-28

User review requested shorter copy, clearer connection identity, and a regular
settings grid. Connection names now lead each row, followed by read/edit scope
and a locale-formatted expiry. Legacy confirmations show `Name unavailable`.
Partial removal retains its visible progress and retry action; expired/removed
connections remain in history. Sign out and Delete account share the desktop
row and use the existing single-column layout on narrow screens.

A second review requested removing the per-connection Details toggle and the
`All holders of this link` caption. Both are removed, including the diagnostic
identifier fields. Copies of a link still share the same grant: one removal
action affects all copies. That scope remains in the button's accessible label
and hover title; the visible button says `remove access`. No authority or
revocation semantics changed.

The final browser artifact is `/private/tmp/agent-access-polish-final-build`,
service-worker build `e598be7ad8fb3f6a`. Its profile and stylesheet were verified
byte-for-byte against the edited source. The initial runtime launcher used
`web-integration-tests`, which selected Wasm mode and failed on unsupported
networking before UI assertions. The repository's native browser configuration
uses `integration-tests,connection-invites`; the same three focused checks are
now passing under that configuration with retries disabled.

Final evidence after removing the Details toggle and caption:

- Native browser run `20e71cc3-a084-4c10-80ad-41ab5b21fc56`: 3 passed in
  23.769s, retries=0. Checks cover main's account/passkey layout plus the new
  Sign out/Delete account row, confirmed-only visibility, and named grouping,
  history, escaping, partial-removal retry, error recovery, and stale responses.
  The mounted renderer asserts there are no per-connection Details elements.
- Actual screenshots reviewed: desktop 1200×900, narrow 390×844, and short dark
  390×540. Connection identities, readable expiry, retry progress, and removal
  actions remain visible with no Details toggle or shared-holder caption.
  Browser assertions verify no horizontal overflow, 44px targets, keyboard focus
  rings, and reduced-motion controls. A separate 1200×1200 account view confirms
  the two-column settings grid. Captures are in
  `/private/tmp/agent-access-polish-artifacts/`.
- `nix develop . -c cargo fmt --all -- --check` passed after the final source
  change; final `git diff --check` passed. No unresolved Git entries remain.

The 56 standard-library/FAB-drift tests passed during the first UI refinement;
that run preceded the later Details/caption removal. The three browser tests
above are runtime evidence for the final UI. The other 105 native browser tests,
hosted CI, and a new packaged release were not run. Existing staged feature work
was preserved; this UI refinement remains unstaged. No commit, push, account
mutation outside isolated test profiles, or local-data cleanup was performed.

### Connection-name alignment follow-up, 2026-09-28

The screenshot exposed Web Awesome's native `li { margin-inline-start: 1.125em }`
offsetting names by 18px despite the list itself having no padding. A scoped
`.connection-installations > li { margin-inline-start: 0 }` override aligns
names with permissions/expiry without changing ordinary lists.

The existing native named-access browser regression passed once (10.08s) against
`/private/tmp/agent-access-aligned-build`, whose profile and CSS match the current
source. Desktop, narrow, and short dark screenshots in
`/private/tmp/agent-access-aligned-artifacts/` confirm aligned text and unchanged
responsive actions. No new tests or Rust changes were needed for this CSS fix.
200% zoom and RTL were not verified in this focused follow-up.
