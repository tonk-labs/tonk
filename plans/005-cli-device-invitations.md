# Plan 005: Make CLI joining exclusively a tool connection

Status: DONE — implementation, native gates, and the feature-enabled
real-browser archive passed; remaining external and cross-browser checks are recorded below.
Priority: P1. Effort: M. Risk: high at the credential and compatibility boundaries.
Category: architecture / UX. Planned at `d4003e8d6`, 2026-09-22.
Branch: `fix/cli-device-invitations`.
Worktree: `/Users/jackdouglas/tonk/tonk/.wt/fix/cli-device-invitations`.

This is an implementation handoff, not permission to publish, deploy, delete
local state, or redeem a real bearer link. Implement in independently tested
checkpoints. Update this file and `plans/README.md` with actual evidence.

## Decision and motivation

The product offers two explicit actions:

- **Invite someone** invites a person through the browser as a separate member,
  using the existing ordinary invitation flow.
- **Connect a tool** issues scoped access associated with the issuing account,
  without adding another human member. This generalizes the existing agent flow.

`tonk join LINK` does only the second. Valid supported tool-connection links work
without CLI account login, browser approval, or account selection. Ordinary user
invitations get a useful wrong-kind error; the CLI must not reinterpret them as
tool credentials or claim them as a new account.

The reported account-root-prefix failure occurs in ordinary claim/mount. This
plan removes that path from new CLI joins. It does not reset account state,
create another account, or rewrite existing grants to repair that path.

This supersedes this file's earlier persistent-device-key draft and plan 003's
accept-both-formats new-import behavior. Plan 003 remains historical evidence,
not the current product contract.

## Required invariants

1. Keep the invitation private key as the tool's identity. No shared CLI key,
   device-root redelegation, exported account key, or new onboarding account.
2. Preserve signed scopes, audience, expiry, remote, revocation ancestry,
   original grant-set identity, and confirmation semantics. Account association
   does not mean full account authority.
3. Existing CLI account state is irrelevant to tool import: absent, same,
   unrelated, and malformed legacy account state must not select the identity.
   Do not read it to choose authority, repair it, or delete it.
4. Each connection keeps separate credentials. Different issuing accounts can
   connect different spaces, or the same space, to one CLI. Never merge grants
   or replace a connection just because the space subject matches.
5. Copies of one link share an invitation identity and revocation boundary.
   Independent revocation requires separate links. No promise of one identity
   per physical device or exclusive/online presence from a receipt.
6. Reject user links before replica, credential, binding, registry, account,
   membership, or receipt writes. Shortcut resolution is allowed; claiming is not.
7. Success requires initial pull and acknowledged receipt push, then successful
   naming/binding. Failures return nonzero and retain safe resumable state.
8. No account picker, `connect` command, or browser callback. `join --via ORIGIN`
   may explicitly select the trusted deployment for a non-production
   link; it never selects authority or a browser approval page.

## Current state and drift check

Run first from this worktree:

```sh
git status --short
git diff --stat d4003e8d6..HEAD -- rust/tonk-cli rust/tonk-fab rust/tonk-core/assets/library rust/tonk-worker rust/tonk-ui docs/storybook plans
```

Reconcile drift against these anchors before editing and preserve unrelated work.
These facts are source-inspected, not fresh runtime or hosted-service evidence.

- `rust/tonk-cli/src/join.rs:228`, `prepare`: resolves shortcuts once, rejects
  mixed carriers, and currently returns either prepared agent or ordinary input.
- `rust/tonk-cli/src/bin/tonk.rs:2518`, `join_command`, currently dispatches both:

  ```rust
  Ok(tonk_cli::join::PreparedInvitation::Agent(prepared)) => {
      connect_scoped_agent(prepared, name.as_deref()).await
  }
  Ok(tonk_cli::join::PreparedInvitation::Ordinary(prepared)) => {
      join_ordinary(*prepared, name.as_deref()).await
  }
  ```

- `rust/tonk-cli/src/invite.rs:513`: ordinary claim reads a local account root
  or creates an onboarding account before claiming and mounting. New joins must
  no longer enter this path.
- `rust/tonk-cli/src/connections.rs:507`, `import_at`, already imports an isolated
  credential with:

  ```rust
  let signer = Ed25519Signer::import(connection.invite.secret_seed()).await?;
  ```

  Reuse its manifest phases, private permissions, exact binding checks, and
  fail-closed reopening. Do not replace the storage layout.
- `rust/tonk-cli/src/handoff.rs`: existing pull/receipt/push, synced-name,
  collision-safe alias, explicit naming, and directory-binding behavior.
- `rust/tonk-worker/src/router/agent_connections.rs:308`: `PublicGroup` already
  records issuing `account`, `subject`, `recipient`, and original grant chains.
  This association is not a separate source of authorization.
- `rust/tonk-fab/src/markup.rs:163` and `element.rs:629`: `data-share-link` says
  `copy link` and forwards to the ordinary share mint. Do not implicitly turn it
  into a tool mint.
- `rust/tonk-core/assets/library/{core,onboarding-agent}.yaml`: current agent
  surfaces primarily copy agent prompts. Existing spaces can retain frozen
  seeded views; YAML changes alone do not update every existing space.
- `rust/tonk-worker/src/router/repository.rs`: tool issuance currently uses the
  `connection-invites` feature and retains a transient link until explicit new
  issuance. Preserve its feature gate and issuer account/space checks.
- `rust/tonk-cli/tests/connections.rs:190` checks recipient identity and private
  credentials; line 340 tests distinct invitations for one subject. Extend these.
- `rust/tonk-cli/tests/connection_commands.rs:17` is the executable-test pattern:
  isolated temporary state, telemetry/update checks disabled, `Result<()>`
  tests, and local S3/access-service fixtures. Never use the real user profile.
- `rust/tonk-cli/Cargo.toml` has `autotests = false`; register any new test target.
- `DESIGN.md` requires existing components, lowercase browser labels, and
  “people” rather than “users” in copy. CLI text follows existing CLI patterns.
  README describes central replicas with directory pointers, not copied data
  physically stored in the command's working directory.

## Scope

Only modify the following where necessary:

- CLI `src/{join,handoff,connections,space}.rs`, `src/bin/tonk.rs`,
  `tests/{join_routing,connection_commands,connections,connection_compatibility,handoff}.rs`,
  README, and Cargo manifest only for test registration.
- FAB `src/{markup,element,bar,activation,lib}.rs`; a small new
  `src/tool_connection.rs` is allowed for an app-owned connection surface.
- Core assets `library/{core,onboarding-agent}.yaml` for maintained prompt copy;
  UI `src/register_dialog.rs` for recovery copy and `src/account_flow.rs` for E2E.
- Worker `tests/standard_library.rs` and inline tests in
  `src/router/{repository,agent_connections}.rs`. Do not redesign production
  minting or authority in these modules.
- Related Storybook command, collaboration, handoff, and verification documents
  and generated artifacts; this plan and its index.

Out of scope: browser user-join semantics, wire/schema renames, account
login/custody/session redesign, new scopes or lifetimes, one-time redemption,
ordinary-invite mint-on-copy changes, dependencies/lockfiles, state cleanup,
deployment flags, release/publish, and wholesale removal of ordinary invite APIs.

## Compatibility decisions

- Accept existing supported v1/v2 agent links and shortcuts. Keep wire names,
  schema identifiers, API routes, and manifest versions. “Tool” is product copy,
  not a new protocol.
- Preserve existing ordinary replicas, bindings, credentials, offline edits,
  and normal read/write/sync. Do not convert or reauthorize them.
- Keep explicit `tonk --space NAME join` recovery of an already persisted
  ordinary import journal, labeled as legacy recovery. It cannot be initiated
  by supplying a new ordinary URL. This compatibility path does not repair an
  already broken authority chain; report such failures without deleting data.
- Retain ordinary mint/library functionality. CLI `invite` documentation must
  identify its output as a person invite to open in the browser.
- Retain `Agent connection confirmed` as the exact CLI compatibility marker
  in this increment, because frozen prompts wait for it. Browser product copy
  may say `tool connected`. Retiring that marker needs a separate decision.
- Older binaries can still accept user links. Document this deliberate narrowing
  without claiming every installed version already follows the new contract.

## Checkpoint 1: Prove tool-only new-import routing

Change `join::prepare` / `join_command` so a successful NEW import preparation
can produce only validated tool authority. Prefer a tool-specific prepared
return type instead of retaining an ordinary success variant that a future
caller might accidentally dispatch. Keep legacy journals/resume separately.

Recognize ordinary invitation input enough to return a dedicated redacted error,
without consulting account state or claiming it. Preserve malformed, mixed,
and unknown-version errors; do not call arbitrary URLs valid user invitations.
Resolve shortcuts once and preserve full grant/trusted-discovery validation.
Payload markers alone never authorize access.

Wrong-kind copy:

```text
This link invites a person to the space.
To connect the CLI, ask for a link from "connect a tool" in Tonk.
```

Keep `--name`; reject URL plus `--space`; do not infer resume from `TONK_SPACE`.
Update help and routing tests together. Replace the ordinary executable import
success assertion with rejection/no-mutation tests, but retain ordinary library
tests. Construct an already-persisted ordinary fixture to prove legacy resume.

Verify, expecting exit 0:

```sh
cargo test -p tonk-cli --locked --test join_routing
cargo test -p tonk-cli --locked --test connection_commands
```

Cover full/short v1/v2 tool links; ordinary open and targeted rejection;
resolve-once; mixed/invalid/unknown versions; secret-free errors; and unchanged
account/registry/binding state after rejection. Review this increment separately.

## Checkpoint 2: Prove account independence and isolation

Extend `connections` and `connection_commands` fixtures before changing storage.
Import and resume with absent, same-account, unrelated, and malformed legacy
account state. Assert account bytes unchanged, imported DID equal to the invite
recipient, and no new onboarding account or human member.

In one registry prove two issuing accounts / two spaces, then independent
invitations / the same space. Assert separate keys, grants, receipts, upstreams,
and collision-safe aliases. Exact-link retry reuses its connection; matching
only the space subject must not reuse a different connection. Reuse the access
service's standard revocation fixtures to show revoking one independent grant
does not deny the other. Remote denial must preserve offline data and edits.

Cover missing credentials on reopen, failed pull, failed receipt push, and
restart without the bearer. Never regenerate an identity or fall back to ambient
account authority. Success text is forbidden before confirmation succeeds.

Verify, expecting exit 0 for non-ignored tests:

```sh
cargo test -p tonk-cli --locked --test connections --test connection_commands --test handoff
cargo test -p tonk-access-service --locked --features helpers --test connections
```

Report ignored production-Wasm and old-executable checks separately. Normal
suite success is not evidence those checks ran.

## Checkpoint 3: Expose explicit browser actions

In the share menu, label the existing person-share action `invite someone` and
add `connect a tool`. Dispatch each to its own existing handler. Do not hide the
distinction in help text or infer it from account/CLI state.

Open a small app-owned tool-connection surface using existing FAB components.
Explain `give a tool access to this space under your account`. Reuse existing
agent issuance, state, and refusal handling. Offer `copy link` for direct use
with `tonk join <link>` instructions; keep `copy agent prompt` as a secondary
convenience. Both copy the same scoped invitation, not separately minted keys.
Agent-specific playground instructions can stay on the playground surface.

First prove the smallest vertical slice on a fresh AND previously seeded space:
open action, mint scoped link, copy, import with the test CLI, confirm. Prefer
current app-owned chrome over overwriting frozen or user-authored views. If this
cannot fit the allowed files, stop and propose the smallest scope extension.

Preserve issuer-side account/activation/sync/feature-disabled refusals. The
receiving CLI needs no browser flow; the issuing browser still needs authority.
Switching account or space must clear or replace transient displayed credentials
before copying; never label an old link as belonging to a newly selected account.

Keep the privacy warning, expiry/revocation, connection management, and receipt
meaning. A confirmation is a tool connection associated with the issuer, not a
new member or proof of online presence. Do not derive account authority from
mutable display labels.

Verify:

```sh
cargo test -p tonk-worker --locked --features connection-invites --test standard_library
cargo test -p tonk-fab --locked --lib
nix develop --accept-flake-config .#ci --command test:e2e -E 'test(tool_connection) | test(it_keeps_copied_agent_grants_independent_of_cli_accounts)'
```

Name new real-browser tests with `tool_connection`: fresh/returning-space actions,
ordinary-link rejection, account/space switching, and successful confirmation.
Expected: exit 0 with the named tests executed, not zero matches/quarantined skips.
Follow `account_flow.rs` fixtures with the harness-built CLI, not downloaded npm.
Ensure its browser artifact enables `connection-invites`; separately verify the
disabled artifact's refusal without changing deployment configuration. Inspect
rendered controls, keyboard operation, and narrow layout; source tests alone do
not verify the UI. Browser person invites must still create ordinary membership.

## Checkpoint 4: Document and run integration gates

Update CLI README/help and maintained prompt templates consistently. Explain
space-scoped tool access, multiple issuing accounts, browser-only new person
joins, shared-link identity, and legacy recovery. Keep the exact compatibility
success marker in prompts. Do not describe working-directory bindings as copies
of centrally stored replicas.

Update relevant Storybook journeys and verification items, distinguishing Drafted
from Verified. Follow its AGENTS.md, README, goal, and applicable source documents
before editing. Generate artifacts with its scripts, not hand edits.

Run at this integration checkpoint, expecting exit 0:

```sh
cargo fmt --all -- --check
cargo test -p tonk-cli --locked
cargo test -p tonk-invite --locked
cargo clippy -p tonk-cli --all-targets --all-features --locked -- -D warnings
python3 docs/storybook/scripts/build.py --check
python3 docs/storybook/scripts/check-links.py docs/storybook
git diff --check
git status --short
```

Commands are grounded in manifests, repository Nix tasks, and established gates;
none ran during planning. Use the repository Nix environment for prerequisites.
Report environment failures at the actual boundary rather than changing product
behavior for them. Review status for modifications outside scope.

## Acceptance matrix

| Scenario | Required result |
| --- | --- |
| Valid supported tool link, empty CLI | Invitation identity imported; confirmed sync; no account/member creation |
| Same link with same/other/malformed legacy account | Same authority; legacy account bytes unchanged |
| Ordinary open/targeted link, full/short | Wrong-kind error; no replica, account, membership, or binding writes |
| Mixed/unknown/malformed/wrong-audience/expired tool link | Redacted error; no ordinary fallback |
| Revoked tool grant | Remote denial; local data retained |
| Different issuers, including same space | Separate credentials and grant sets; neither merged nor replaced |
| Exact retry / explicit resume | Retained identity; no duplicate account/member |
| Pull or receipt-push failure | No success marker; safe state and recovery command |
| Existing ordinary replica / pending import | Data preserved; explicit legacy recovery, never silent conversion |
| Browser person invite | Existing separate-member flow unchanged |
| Old supported agent link / previously seeded space | Link works; current tool action available or explicit refusal |
| Issuing account/space changes while open | No stale or mislabeled copied link |

## Done criteria and execution record

- [x] Checkpoint 1 passed, including wrong-kind no-mutation tests.
- [x] Checkpoint 2 passed: account independence, same-space isolation,
  independent revocation, and failure/restart tests.
- [x] Checkpoint 3 passed in fresh and returning spaces; browser person
  membership still works and tool connection adds no human member. The
  feature-enabled archive executed the named browser journeys.
- [x] Checkpoint 4 passed for Plan 005; help, docs, maintained prompts, and the
  packaged E2E artifacts agree. Storybook's repository-wide generated-data and
  link checks still stop on pre-existing removed source paths recorded below.
- [x] Skips and unrun old-binary, Safari/device, production-Wasm, deployment, and
  hosted checks are explicitly recorded, not inferred from native tests.
- [x] No dependencies, wire schemas, account storage, or unrelated files changed.
- [x] This plan and its index record the uncommitted source state and fresh evidence.

### Execution evidence — 2026-09-22

Source remains the uncommitted worktree based at `d4003e8d6`; no commit, push,
publish, deployment, real bearer redemption, or state cleanup was requested.

- Checkpoint 1: `join_routing` (4) and `connection_commands` (5) passed. The
  complete `tonk-cli` suite also passed, including executable wrong-kind
  no-mutation and explicit legacy-journal recovery coverage.
- Checkpoint 2: `connections` (8), `handoff` (11), and the access-service
  connection suite (4 passed, 1 ignored production-worker restart case) passed.
  The full `tonk-invite` suite and CLI Clippy with `-D warnings` passed.
- Checkpoint 3 native evidence: FAB library tests (119), worker standard-library
  tests with `connection-invites` (47), exact app-owned target/cross-space
  refusal (1), and feature-disabled explicit refusal/no-bearer (1) passed.
  `tonk-fab` checks for `wasm32-unknown-unknown`; the feature-enabled `tonk-ui`
  all-target code, including the authored `tool_connection` browser test,
  checks natively.
- Checkpoint 3 browser evidence: the approved `flake.nix` extension now builds
  `tests-e2e` with `integration-tests,connection-invites`, builds the CLI and
  preview UI together, and serves that preview to the harness. The exact plan
  filter executed 2 tests from the packaged archive: both passed, with 92
  skipped. `it_keeps_copied_agent_grants_independent_of_cli_accounts` covered
  fresh issuance, account isolation, restart secrecy, receipts, and independent
  revocation; `tool_connection_rejects_person_links_and_confirms_the_cli`
  covered wrong-kind rejection, shared copy identity, accountless confirmation,
  returning-space rotation, and space-switch clearing. The existing
  `it_signs_up_to_share_and_hands_over_the_link` person-membership journey also
  passed from the same archive (1 passed, 93 skipped).
- Checkpoint 4: `cargo fmt --all -- --check` and `git diff --check` passed.
  Maintained prompt assets were regenerated from their source. Storybook data
  was regenerated before `build.py --check` and the link checker; both stop on
  the pre-existing WEB-07–WEB-13 references to removed
  `rust/tonk-workspace/src/ui_account_settings.{html,rs}`, not on a changed
  Plan 005 path.

The first exact `test:e2e` attempt correctly exposed that the archive omitted
`connection-invites` and therefore selected zero tests. After explicit approval,
the narrow `flake.nix` change enabled the feature in the archive and selected
`tonk-ui-preview` as the served artifact. The fresh packaged rerun selected and
passed both required tests. Rendered 390px layout, keyboard focus, reduced
motion, and a profile switch while the surface is open remain unverified rather
than being inferred from the browser normal paths.

Also unrun: the ignored released-old-executable compatibility case, ignored
production-worker persisted-KV restart case, Safari/device testing,
production-Wasm runtime, deployment, registry, and hosted-service checks.

### Follow-up evidence — 2026-09-23

Source remains uncommitted on `fix/cli-device-invitations` at `0ad1c07176`; no
push, publish, deployment, registry, or hosted-service operation was performed.

- `tonk join URL --via ORIGIN` now selects an explicit trusted deployment
  origin. It overrides `TONK_CONNECTION_ORIGIN`, must match the signed `/ucan/`
  route, and still requires successful `/.well-known/tonk` discovery without
  redirects. It is unavailable for URL-less resume mode.
- Staging and local browser prompts now copy `--via` with their page origin;
  production prompts remain concise and omit the option.
- The complete `cargo test -p tonk-cli --locked` suite passed (with its existing
  explicitly ignored compatibility/schema cases). The focused
  `connection_commands` integration suite passed all 5 tests with host access,
  including invalid-origin rejection, route-mismatch rejection, and CLI-option
  precedence over a deliberately wrong environment fallback.
- The focused FAB prompt test passed; the `tonk-ui` all-target check with
  `integration-tests,connection-invites` passed; and the maintained worker
  standard-library prompt test passed.
- The packaged browser test
  `tool_connection_rejects_person_links_and_confirms_the_cli` passed (1 passed,
  93 skipped) after 547.647 seconds, exercising the copied non-production
  command and packaged CLI together. Its first invocation stopped before the
  build because the sandbox could not write Nix's fetcher cache; the unchanged
  host-access rerun passed.
- `cargo fmt --all -- --check`, `git diff --check`, and the rendered
  `tonk join --help` command surface passed after the last source change.

Still unrun in this follow-up: full repository suites, Safari/device testing,
production deployment, registry publication, and hosted-service verification.

## Stop conditions and maintenance

Stop and report if implementation requires widening authority, changing the
invitation identity, using an unverified account label, merging same-space grants,
deleting legacy data, changing deployed formats, or touching an out-of-scope
area. Also stop for unresolved conflicting edits, a fixture proving tool imports
require ambient account authority, or frozen-view migration beyond scope. Show
the failing test and hypothesis before changing direction.

Guard against restoring ordinary claim as a fallback: retain the tool-only typed
entry point and executable rejection tests. Future copy changes must distinguish
product labels from stable protocol names and frozen prompt contracts. New
account management, generic device pairing, ordinary mint-on-copy redesign, and
retirement of the old success marker require separate decisions.

Keep each checkpoint independently reviewable. Make separate conventional
commits only if authorized. Do not push, open a PR, or deploy without a request.
