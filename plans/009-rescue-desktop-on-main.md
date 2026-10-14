# Plan 009: Recover Tonk Desktop on current main

Status: IN PROGRESS. Priority: P1. Effort: L, delivered as tested increments.
Planning baseline: `origin/main` at `55af716ef5`, fetched during this planning pass.
Source: `feat/desktop` at `28f1f72212`, plus the working changes inventoried below.
This plan interprets “rescue Desktop over the latest” as preserving the existing
native app and porting it onto current main. It does not reimplement the app or
make Desktop depend on hosted ChatGPT OAuth.

## Outcome

A reviewable Desktop branch based on current main builds and runs the native app,
uses the current canonical query/evaluate API, preserves local accounts/chats and
unfinished activity UI, and passes native plus real-worker integration checks.
The existing browser/CLI release pipeline remains intact. Signed passkey and live
provider acceptance are recorded separately from compilation and fixture tests.

## Why selective recovery is necessary

The source branch contains the pre-split combined MCP history, not just Desktop.
Main now includes the general MCP (#1089), hosted ChatGPT (#1091), dialog 0.3.0
(#1058), revised build profiles (#1090), release readiness (#1092), and account
linking UI (#1095). Main still has no Desktop subtree or Desktop Nix commands.
A wholesale merge/cherry-pick would risk reintroducing retired runtimes, tool
schemas and stale Rust/library behavior. The older Desktop catalog also directly
requests tools absent from main's canonical catalog.

Verified source stack (use as provenance, not an instruction to cherry-pick all):

| Commit | Intent | Recovery treatment |
| --- | --- | --- |
| `94eb307310` | Import standalone app unchanged | Reference import boundary; prefer final Desktop subtree plus reviewed adaptations |
| `aaf4caa931` | Shared MCP/context integration | Port only Desktop consumers/generation against current core; do not copy its core, Rust or browser changes |
| `6126b0b765` | Root Nix Desktop commands | Reapply command intent to current flake; retain main's lock and profiles |
| `bdb1d5bcb1` | Desktop signing CI | Adapt scripts/workflow after ordinary build passes |
| `28f1f72212` | Linking investigation notes | Historical evidence, not current deployment proof |

## Preserve before doing anything

The source worktree is `/Users/jackdouglas/tonk/tonk/.wt/feat/desktop-shared-mcp`.
It currently has these tracked, uncommitted changes:

- `desktop/Sources/HarnessCore/Conversation.swift`
- `desktop/Sources/Tonk/ContentView.swift`
- `desktop/Sources/Tonk/HarnessCLI.swift`
- `desktop/Sources/Tonk/HarnessModel.swift`
- `desktop/Sources/Tonk/HarnessSessions.swift`
- `desktop/Sources/Tonk/HarnessSpaceTools.swift`
- `desktop/Tests/TonkTests/ChatTranscriptTests.swift`
- `desktop/Tests/TonkTests/HarnessProviderTests.swift`

Also preserve the untracked `plans/2026-10-09-chat-turn-activity.md` and
`plans/2026-10-12-desktop-link-revocation.md`. Never clear this worktree, stash it
implicitly, force-push it, or modify its index/history during recovery. Inventory
again: another session may have changed it since planning. Do not copy .build,
node_modules, artifacts, secrets, signing profiles or user application data.

## Scope and architecture

Allowed implementation scope: `desktop/**`, a new Desktop generator/test under
`mcp/scripts/`, minimal `mcp/package.json`/test-glob changes, `flake.nix`, Desktop
CI, relevant README sections and this plan/index. Any Rust or core evaluator
change needs a reproduced prerequisite and separate review; do not port old Rust
files just to resolve conflicts. No dependency/lock changes without demonstrated
need. No changes to the separate client-neutral MCP worktree or its plan.

Current source points to inspect:

- `desktop/Sources/HarnessCore/RuntimeLocation.swift`: production loads
  `https://tonk.network`; local/custom environments have separate data identifiers.
  Preserve those identifiers and production's existing data directory.
- `desktop/Sources/Tonk/RuntimeBuild.swift`: routes query/evaluate via generated
  JS into the account WebKit worker; unconfirmed writes instruct query-first.
- `desktop/Sources/HarnessCore/LocalRuntimeBridge.swift`: private native bridge,
  capability/authority gates and obsolete tool branches to inventory.
- `desktop/Sources/HarnessCore/SpaceTools.swift`: directly requests
  `tonk_inspect_view`, `tonk_space_info`, `tonk_space_schema`, `tonk_rename_space`
  from the old canonical catalog. Main intentionally removed those core tools.
- `desktop/Sources/Tonk/HarnessSpaceTools.swift`, `RuntimeCLI.swift`,
  `HarnessCLI.swift`, `SpaceBuildTools.swift`, `SpaceProposal.swift`: trace native
  controls and saved tool dispatch separately from newly advertised agent tools.
- Old `mcp/scripts/sync-desktop.mjs`: generates Swift catalog/context and bundles
  primitive JS; its imports/output assumptions must be reconciled with main.
- Old `build-agent-surface.mjs`: also runs browser generation not present on main.
  Do not restore that umbrella command unchanged.
- Main `mcp/browser.mjs`: portable `createAgentTools`, canonical guides and worker
  dispatch; prefer this supported boundary for the native JS projection.
- `desktop/scripts/build-app.sh`, `dev.sh`, `test-agent-workflow.py`,
  `serve-local-runtime.py`, `ci-sign.sh`: build, isolated workflow and signing.
- Main `.github/workflows/release-readiness.yml`, `release-verify.yml`,
  `promote.yml`, `publish.yml`: current release boundaries, not Desktop release proof.

## Checkpoint 1: Make an isolated, reproducible recovery inventory

Fetch main, capture both exact SHAs and dirty status, and create a fresh worktree
(e.g. branch `feat/desktop-on-main`). Keep the old branch untouched. First export
its tracked dirty diff with `git diff --binary HEAD -- desktop` to a protected
local backup and copy the two untracked notes separately. Record checksums and
paths in local recovery notes, never credential contents. Review the diff before
applying it. If the source changes during capture, stop and recapture coherently.

Extract only the tracked `desktop/` subtree at `28f1f72212` into the new worktree,
then rebase its integration at the next checkpoint. Do not copy its entire
`mcp/`, Cargo files, flake.lock, browser assets or Rust trees. Preserve import
provenance in `desktop/MIGRATION.md`. A provenance-only import commit may be
non-buildable; label it as such and pair it with the working adaptation before
opening a PR. Prefer the first independently buildable commit where practical.

Gate: source `git status` and diff hashes unchanged; target contains only intended
tracked Desktop files and planning notes; no private/local build material copied.

## Checkpoint 2: Restore one executable native query/evaluate path

Port a small Desktop generation command using current `mcp/browser.mjs`,
`catalog.mjs`, `authoring.mjs` and guides. Generate the existing Swift consumers
rather than hand-editing generated code. Support generate and `--check`; include
core/catalog/guide changes in invalidation. Run generation twice and verify it is
deterministic, then verify check mode detects an intentionally stale test fixture.
Do not change canonical tool semantics to accommodate the old generator.

Build the app and prove one selected-space read, one evaluate and readback through
the native bridge into a current local Tonk WebKit worker. Keep account selection,
local transport and native rendering in Desktop. Do not route native operations
through the hosted ChatGPT server, or add a second worker semantic implementation.
Use current main's matching runtime and CLI; record their revisions. The dialog
0.3.0 changes make old serialized fixtures and prompts suspect: regenerate from
canonical sources and test current notation. Do not downgrade production data or
rewrite existing stores as a test repair.

Inventory every legacy tool reference. Remove retired tools from new advertised
catalogs and dispatch, replacing internal metadata/schema/rename operations with
canonical query/evaluate recipes. Preserve legitimate native UI affordances such
as inspect/render and proposal confirmation as explicit Desktop-only controls;
their schemas must not be re-added to `mcp/tools.json`. Historical chat records may
retain names for display, but must not silently replay old writes. Handle stale
saved proposals explicitly without auto-executing or silently discarding them.

Authority tests: selected space is frozen per invocation; another space cannot be
chosen through tool arguments; read-only connection rejects direct writes;
closed/revoked connections fail; cancelled/lost write replies never cause replay.
No model-visible credentials. Preserve unknown-write errors through all providers.

Gate: focused Swift bridge/tool tests, core MCP tests, generated-file check,
ad-hoc app build, and one actual local native read/write/readback pass.

## Checkpoint 3: Recover unfinished activity UI separately

Apply the eight-file dirty patch onto the adapted app, reconciling removed legacy
tool branches by preserving activity semantics rather than restoring those tools.
`Conversation.swift` adds optional `toolActivity` to messages and records entries
on the originating user turn. Preserve decoding of older saved conversations.

Test per-turn separation, persistence, legacy decoding, cancellation, errors and
late results: a slow tool from turn A must not get assigned to a newer turn B.
Inspect the actual ContentView with two turns, independently expanded dropdowns
and a narrow window. Then verify one real provider-streamed tool call; fixture
screenshots alone establish layout, not provider event delivery. Keep any unrelated
activity behavior changes separate from the initial migration commit.

Gate: ChatTranscriptTests and HarnessProviderTests pass; old/new saved histories
round-trip; fixture evidence is recorded separately from live streaming evidence.

## Checkpoint 4: Port root build commands and add CI

Add macOS-only root commands `build:desktop`, `dev:desktop`, `dev:desktop:open`,
`test:desktop`, `test:desktop:workflow` against current flake patterns. Retain current
CI/release profiles, cache behavior and lockfile; do not restore the old flake.
Use the new generator directly instead of importing obsolete browser generation.
Xcode stays a host prerequisite; isolate Nix SDKROOT/DEVELOPER_DIR from Xcode tools
as the existing Desktop wrappers intended. Confirm full Xcode/macOS requirements
against the actual build rather than claiming the old README is current proof.

Add `.github/workflows/desktop.yml`: Swift tests, generator drift, signing-profile
validator tests and ad-hoc app build on relevant PR paths. Include canonical guide,
MCP and worker/frontend changes that can affect Desktop, not only Swift paths.
Preserve failures as failures; separate optional cache failures from test outcomes.
Keep signed artifacts optional for trusted events and never expose signing secrets
to PR code. Reuse existing CI conventions and current runners after verification.
Do not make browser/CLI release gates imply Desktop acceptance or automatically
publish/install this app. Signed artifacts and notarized releases are distinct.

Gate: root commands work on macOS; workflow validation and PR CI pass; main's MCP,
ChatGPT, browser and CLI checks remain unaffected. Verify the exact built app.

## Checkpoint 5: Verify accounts, remote sync and signing without data loss

Start in isolated local storage, then a matching development/preview environment.
Exercise existing-account login, new-account continuation, cancellation, timeout,
wrong-account rejection, deep-link destination preservation, reconnect/relaunch,
space selection and remote write/readback from a second replica. Trace actual
worker/SDK request and callback boundaries instead of assuming UI completion means
sync finished. Verify foreground and background approval completion without an
extra click, focus event or polling request that could mask a delivery bug.

For existing profiles, back up safely and test a disposable copy where possible.
Never wipe WebKit stores, Keychain, local replicas or chats as recovery. For a
revoked grant, record the exact error and inspect the loaded runtime version before
attributing it to signing or local auth. Historical notes diagnosed an older stable
worker; that diagnosis is not evidence about today's deployed version. App rebuilds
do not update the remote runtime. Record provider URL and actual runtime hash.

Port signing/profile validators and confirm certificate, profile, team, bundle ID,
entitlements, embedded profile and AASA association agree. Never transfer signing
material without approval for its exact file and destination. Ad-hoc builds cannot
prove native passkeys. Stable's associated domain does not automatically authorize
preview/development hosts; use supported browser handoff there, or separately
provision verified native associations. Do not weaken domain checks to pass tests.

Gate: signed-app validation plus an actual passkey/account flow and correct native
callback are separate evidence rows. If credentials/accounts are unavailable,
report that row unverified and retain all completed independent work.

## Verification commands and expected results

Commands observed in source; adapt only the generation entrypoint as described.
Run from recovery root, using current locked dependencies:

```sh
npm --prefix mcp ci --ignore-scripts
npm --prefix mcp test
npm --prefix mcp run test:chatgpt
npm --prefix mcp run check:format
node mcp/scripts/sync-desktop.mjs --check
bash desktop/scripts/dev.sh test:unit --filter ChatTranscriptTests
bash desktop/scripts/dev.sh test:unit --filter HarnessProviderTests
bash desktop/scripts/dev.sh test:unit
python3 -m unittest discover -s desktop/scripts/tests -p 'test_*.py'
TONK_SIGN_IDENTITY=- bash desktop/scripts/build-app.sh
codesign --verify --strict desktop/.build/Tonk.app
```

`sync-desktop.mjs` is to be ported; it is not present on the current main baseline.
Update build/dev scripts to invoke it before these gates. Expected result for each:
exit 0; no skipped required cases. Use repository Swift/Nix formatting checks.
For workflow proof, build the matching Tonk UI, run
`python3 desktop/scripts/serve-local-runtime.py /absolute/path/to/runtime 4187`,
then in a second shell `bash desktop/scripts/dev.sh test:workflow`. Expected: real
native tool call, visible checkbox interaction, readback and restart persistence
in the runner's isolated app/data copy. Verify the script's current arguments
before execution; never substitute the user's installed app or production store.

After integration run `nix develop .#ci -c test:desktop` and the new generation
check through CI; use `actionlint .github/workflows/desktop.yml`, `git diff --check`
and compare the final diff against the captured main SHA. Final evidence includes
app, runtime and source revisions, not just test counts from the old branch.

## Delivery and completion

Recommended increments: (1) Desktop import plus current-core adaptation, (2) activity
UI recovery, (3) root commands/ad-hoc CI, (4) signed-account acceptance and fixes.
Open the Desktop PR only when the first increment works; stack follow-ups if they
remain independently reviewable. Do not merge or deploy as part of merely executing
this plan without the applicable user authorization.

- [ ] Original Desktop branch and dirty work preserved and inventoried.
- [ ] Recovery branch is based on freshly fetched main and carries no legacy MCP/Rust import.
- [ ] Canonical tools/guides generated deterministically; native read/write/readback passes.
- [ ] Old chats and proposals handled without lost data or replayed writes.
- [ ] Activity UI changes recovered, tested and separately reviewable.
- [ ] Current root commands and ad-hoc CI pass; release gates remain correctly scoped.
- [ ] Local workflow, live model, remote sync and signed passkey evidence distinguished.
- [ ] Remaining failures/unverified cases and exact revisions recorded in this plan.

Stop and report if source changes during recovery, an adaptation requires restoring
removed core tools, stored-state migration is destructive, or source/runtime formats
are incompatible. Scope any prerequisite separately. A missing signing credential
blocks signed acceptance only, not preservation or ordinary implementation.

## Planning evidence and limits

Read-only review of source history, dirty diffs, native bridge/catalog/generation,
Desktop scripts, main MCP boundary and current CI/release file names. No merge,
rebase, source edits, builds, runtime tests, signing or deployment performed in this
planning pass. Historical test counts and stale production diagnosis were not used
as current validation. Refresh main and source status when implementation starts.

## Recovery execution — 2026-10-14

Checkpoint 1 capture complete. Recovery worktree `feat/desktop-rescue` started at
freshly fetched `origin/main` `55af716ef5d4c026a1578c4dc4b06a8067225133`.
Source HEAD remains `28f1f72212ee693def902113ff6092e2d244511a`.
Protected backup (directory 0700, files 0600):
`/private/tmp/tonk-desktop-rescue-20261014-152458/`.
`inventory.json` records exact status and checksums; the eight modified files and
two untracked notes match the planning inventory. Source status, HEAD, index and
binary dirty diff were identical before/after capture. No source writes performed.

- Desktop dirty patch SHA-256: `07e5bd3ede658b3a6263cbc854cce6e3038409d9f9162b94af9d1c41418ee60e`
- Source index SHA-256: `ff6f4b976728819ad7c207a3919aa3db6ef9cd1bfbb95df9d4fa3fdb41162c91`
- Activity note SHA-256: `6655e7bf6c1e531b3cd4d73e2bcfcb2e837b0cc1946436c0bfb21135d84e034d`
- Revocation note SHA-256: `0698056c6422c21c8ed3b011aad9cd9af2dde67f755f146a122df3d6770c6659`

Root AGENTS.md is absent on disk; supplied session instructions apply. Read root
README, current MCP browser/catalog/primitive/authoring boundary and Desktop
README/build scripts before adaptation. Imported tracked `desktop/` only using
`git archive 28f1f72212 desktop`; no build material or user profiles copied.
`npm --prefix mcp ci --ignore-scripts` passed (16 packages, unchanged lock).
Initial `swift --version` failed because inherited Nix DEVELOPER_DIR selects SDK
14.4 without Swift; full `/Applications/Xcode.app` exists, so use host Xcode with
SDKROOT cleared for native validation. Fetch required scoped sandbox escalation
for this worktree's Git metadata and then succeeded.

Next: current-core generation and native dispatch adaptation, focused tests and
actual isolated worker workflow before activity recovery and root CI increments.
