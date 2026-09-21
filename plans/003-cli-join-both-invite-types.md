# Accept ordinary and agent invitations through `tonk join`

Status: DONE. Priority: P1. Effort: M/L. Risk: high at the credential boundary.
Planned at `70069be37`, 2026-09-21. Depends on the completed synced-name fix (002).

This is an implementation handoff, not authorization to push or deploy. Read it
fully, implement in independently tested increments, and update `plans/README.md`
with results and remaining verification. First run:

```sh
git status --short
git diff --stat 70069be37..HEAD -- rust/tonk-cli rust/tonk-invite rust/tonk-core/assets/library plans
```

Preserve existing work. Reconcile changed symbols against the current-state notes
below before editing; do not resurrect code from an older branch wholesale.

## Implementation results

Completed on 2026-09-21 in four reviewable commits:

- `01da82175` validates and routes both invitation formats after resolving a
  shortcut exactly once.
- `a4cdc366e` imports ordinary invitations through `tonk join`, retains
  authority-specific storage and receipts, and adds resumable pull/publication
  phases plus collision-safe naming and binding.
- `a487c503a` keeps the typed prepared-invitation dispatch compact under Clippy.
- `d755e9c5f` verifies failed required publication and interruption at the
  ordinary registry-publication barrier.

Fresh native evidence after the last source/test change:

- `cargo fmt --all -- --check`, `cargo test -p tonk-cli --locked`,
  `cargo clippy -p tonk-cli --all-targets --all-features --locked -- -D warnings`,
  and `git diff --check` all exited 0.
- The complete CLI suite includes seven `join_routing` and six
  `connection_commands` tests. The latter uses local S3 and access-service
  processes to prove an ordinary join pulls actual content and the synced
  repository name without asserting an agent receipt.
- `python3 scripts/build.py --check` reported 26 screens, 80 journeys, 120
  verification items, and 6 triage findings; `python3 scripts/check-links.py .`
  validated 178 local references.
- Two pre-existing tests remained intentionally ignored: released-executable
  compatibility requires `TONK_OLD_CLI`, and schema rendering still needs its
  post-#447 analyzer port.

No live browser notification check or external hosted-deployment check was run.
The integration evidence above is native and uses isolated local services. No
push or deployment was performed.

## Intended result

`tonk join INVITE [--name ALIAS]` accepts both ordinary sharing invitations and
agent invitations. The user need not specify `--agent`. Resolve a shortened link
once, select its parser, validate it, and dispatch using a typed validated result.
Malformed or unsupported agent invitations must never fall back to ordinary claims.

Both flows import a replica, pull its content, derive a collision-safe local alias
from its actual `RepositoryName`, and bind the original working directory. Explicit
aliases remain local and never rename the remote repository. Agent joins additionally
publish the existing grant-specific receipt that renders the connection toast.
Normal joins must not create or refresh that receipt.

This is shared command behavior, not a new invitation protocol. Retain each format's
authority semantics: ordinary invitations claim to an eligible local identity;
agent invitations retain their isolated scoped credentials. Do not widen scopes or
convert an ordinary invitation into agent authority merely to reuse code.

## Current state and reusable pieces

- `rust/tonk-cli/src/bin/tonk.rs`, `join_command` currently routes only
  `Some(url) if is_scoped_agent_link(&url)` and rejects all other URLs with
  `unsupported_agent_invitation`. Its classifier looks at the original fragment,
  before `connect_scoped_agent` resolves shortcuts. `--space NAME join` resumes only
  a scoped connection. `Join` help calls every input an agent invitation.
- `rust/tonk-cli/src/invite.rs`, `resolve_url`, `preflight`, and `claim` already
  implement ordinary invitations. `preflight` returns the resolved URL, invitation
  identity, and optional expected recipient root. `claim` stages storage beside
  the destination and refuses existing directories. It selects an existing local
  root or a locally custodied onboarding account, mounts delegated authority,
  retains `claimed-invitation`, configures remote/revocation relay, pulls, and
  records invitation provenance and membership. Its initial pull and roster push
  are currently best-effort; `ClaimOutcome.synced` distinguishes initial pull
  success, but there is no roster-push acknowledgement field.
- `rust/tonk-cli/src/connections.rs`, `inspect_link`, `validate_link`, `import_at`,
  and `open_bound` handle isolated scoped credentials and resumable publication.
  Preserve their marker checks and compatibility protections. A public marker
  selects a constructor; it never grants authority.
- `rust/tonk-cli/src/handoff.rs`, `synced_name` queries the repository subject's
  `RepositoryName`; `available_name` slugifies and suffixes collisions.
  `resolve_connection_name` replaces temporary `agent-<id>` aliases and updates
  directory bindings under the registry lock. `remember_connection_name` protects
  explicit names. `matching_invitation` supports ordinary claim replay detection.
  `confirm_scoped_connection` currently combines pull, receipt assertion, and push.
- `rust/tonk-cli/src/space.rs` supplies staged registration and registry write
  guards. `register_connection_bound` verifies alias/site/connection consistency;
  `register_existing_bound` handles ordinary sites. Preserve both constructors.
- `rust/tonk-core/assets/library/onboarding-agent.yaml` supplies the copied agent
  prompt, including acknowledgement and resume instructions. The receipt view
  itself is in `rust/tonk-core/assets/library/core.yaml` under `agent-connection`;
  it renders “agent setup confirmed”. No new toast implementation is needed.
- `rust/tonk-worker/src/router/join.rs` is the browser's ordinary claim path,
  useful as a semantic reference, not an implementation target for this change.
- Tests: `tests/connection_commands.rs` has isolated executable fixtures and a
  local S3/access-service integration test; `tests/handoff.rs` covers names and
  receipt rendering; `tests/join_profile.rs` covers accountless ordinary claims;
  `tests/invite_revocation.rs` covers ordinary invite authority. Match their
  `Result`-based tests, temporary stores, disabled telemetry/update checks, and
  secret-redaction assertions. Do not use the developer's real profile in tests.

The README describes spaces as centrally stored replicas with directory pointers.
`DESIGN.md` says CLI output follows existing CLI patterns. Plan 001's completed
browser approval flow is specifically local-space ownership attachment; do not
reuse it to add browser approval to invitation joining.

## Scope and decisions

Modify `rust/tonk-cli/src/{bin/tonk,invite,handoff,space,connections}.rs` as needed;
a dedicated `src/join.rs` coordinator and its `lib.rs` export are allowed. Modify
the existing CLI test files above, or add `tests/join_commands.rs` and register it
in `rust/tonk-cli/Cargo.toml` (`autotests = false`). Update directly related CLI
help/docs and the agent prompt if its instructions change. If changing library
assets, inspect `docs/storybook/AGENTS.md` and its documented generation commands
before refreshing affected generated artifacts. Do not hand-edit generated data.

Out of scope: account login/logout command restoration, browser account switching,
local-space ownership linking, worker authorization changes, new invitation wire
formats, dependencies/lockfiles, global storage cleanup, and toast redesign.

Resolve these behaviors as follows:

- Ordinary open invitations work without browser approval or CLI login, using
  existing claim identity rules. Ordinary recipient-targeted invitations require
  the matching locally available identity/authority. Reject a mismatch before
  importing or publishing a replica; do not silently switch accounts. Explain
  that the link targets another identity and request an eligible invitation.
- Keep accepting supported current agent formats. Unknown `tonk-agent-*` versions,
  mixed agent and ordinary payloads, malformed grants, and expired/revoked access
  fail explicitly. An agent-looking fragment is parser routing only, never proof
  of authority. Signed grants still determine subject, audience, and remote.
- Plain `tonk join` without a URL still requires explicit `--space NAME` to resume;
  ambient `TONK_SPACE` must not initiate an import. An input link plus `--space`
  remains an error. `--name` is an import-time override.
- Successful hosted joins require a successful initial pull. If the network fails,
  retain valid imported credentials/data under a printed resumable alias and
  return nonzero with a concrete resume command; do not report connected.
- Ordinary invitations without a remote remain usable for local import with an
  explicit “no sync remote” result. Prefer a synced record when available; otherwise
  use explicit alias, advisory invite name, then subject-derived fallback. Do not
  describe advisory metadata as a name fetched from the hub.
- Preserve ordinary membership/provenance publication. If that push fails, retain
  state and return a pending-publication error with resume instructions. Ordinary
  joins never emit an agent receipt, even on retries.
- Agent success still requires acknowledged receipt push. Normal success says
  `Joined space 'NAME'`; agent success retains `Agent connection confirmed`.
  Both print the chosen alias and a next command.

## Implementation checkpoints

### 1. Prove validated routing, including shortcuts

Add a coordinator entry point yielding a typed ordinary/agent prepared invitation.
Resolve shortcuts before classification, preserve fragments without logging them,
and reuse the resolved payload through validation/import rather than fetching twice.
Keep format detection separate from validation, and reject ambiguous mixed payloads.
Make this additive before switching the existing command dispatch.

Add tests for full ordinary and agent links, shortcuts to each, malformed ordinary
links, unsupported agent versions, mixed formats, expired grants, and errors that
never print the URL/seed. Count shortcut requests in the fixture to prove one fetch.

Verify: `cargo test -p tonk-cli --lib --locked` and the new routing integration
test target, if used, both exit 0. Commit this proven increment.

### 2. Expose the ordinary import adapter through the command

Use ordinary preflight and the existing staged claim machinery. Check targeted
recipient compatibility before state mutation. Do not call the agent constructor
for normal links. Use `matching_invitation` and verified subject/provenance to
recognize exact retries; matching only a space DID is insufficient to identify
an invitation or justify replacing credentials.

Expose structured pull/publication progress rather than inferring success from
printed warnings. Preserve existing library callers' documented behavior unless
intentionally updating them and their tests; the CLI can use a new strict adapter.
Retain minimal public recovery state if needed (kind, invitation identity, phase,
original cwd, explicit alias); never store bearer URLs or seeds in that state.

First executable spike: create an ordinary invitation with an isolated local
service, run `tonk join URL`, and prove upstream content is readable through the
selected local replica with no agent receipt. Also test local-only ordinary mint
and claim, valid matching targeted recipient, and rejected mismatched recipient.

Verify: `cargo test -p tonk-cli --test join_profile --test invite_revocation --test
connection_commands --locked` plus any newly registered test target all exit 0.
Update only the obsolete ordinary-link rejection expectations in
`removed_workflows_preserve_existing_local_state`; keep removed account commands
and unrelated workflows rejected. Commit this increment.

### 3. Share completion and resumable naming behavior

Extract common naming, alias publication, directory-binding, and output mechanics
from `finish_scoped_connection`. Keep authority-specific opening and ordinary
membership work in adapters. Split pull from agent receipt publication so naming
and local finalization do not fail for the first time after a toast was already
sent. Represent any incomplete binding/publication stage durably enough to resume.

Generalize temporary-alias handling without changing existing connection markers.
Preserve `connection-local-name`, legacy generated aliases, exact explicit aliases,
and collision suffixes. Allocate the final alias under a fresh registry write lock;
do not hold that lock over network or repository queries. Publish alias changes
and directory binding updates atomically where possible. Retain storage in place.

Route `tonk --space NAME join` by verified retained import kind. A generic unrelated
local space must not become an invitation import merely because it is selected.
Repeat attempts after pull, roster push, receipt push, or registry publication
failure must preserve authority and offline edits and must not create duplicates.

Verify: `cargo test -p tonk-cli --test handoff --test connection_commands --test
connections --test connection_compatibility --locked` plus ordinary command tests
all exit 0. Commit this increment.

### 4. Finish the public contract and integration checks

Update `Join` help and error messages to describe both invite types, no `--agent`
flag, and the real resume contract. Keep the agent prompt's existing page scope
and privacy guidance. Ensure docs distinguish importing a hosted space from
`tonk space link NAME`, which attaches an existing local space to an account.

Run final native gates once the integrated implementation is ready:

```sh
cargo fmt --all -- --check
cargo test -p tonk-cli --locked
cargo clippy -p tonk-cli --all-targets --all-features --locked -- -D warnings
git diff --check
```

All must exit 0. The current baseline has 16 passing handoff/connection-command
tests; the real local service fixture required execution outside the sandbox
after `Operation not permitted`. Retry that focused check with appropriate access
instead of changing source to accommodate the sandbox. Record any ignored tests,
including released-executable compatibility requiring `TONK_OLD_CLI`.

For the UI acceptance check, open the owning space in a browser, join with an
ordinary invitation and verify no new agent notification; repeat with an agent
invitation and verify its acknowledged receipt produces the existing toast.
Use isolated test spaces. Report browser and hosted checks separately from native
fixture results. A receipt/render unit test alone is not live browser evidence.

## Required regression matrix and done criteria

- [x] One command accepts full and shortened URLs for both supported invite kinds.
- [x] Invalid agent/mixed payloads cannot reach ordinary claim; errors redact secrets.
- [x] Ordinary open and eligible targeted invites work; audience mismatch has no
  replica, registry, account-switch, or directory-binding side effects.
- [x] Both hosted paths pull actual hub content/name; names, explicit aliases,
  collisions, and directory bindings survive restart and repeated links.
- [x] Existing `agent-<id>` entries and explicit-name markers remain compatible.
- [x] Failed pull, failed required push, and interruption around registry publication
  retain recoverable data and never report full completion prematurely.
- [x] Ordinary membership is published without any newly asserted agent receipt;
  agent receipt remains idempotent and tied to the validated grant identity.
- [x] Account state and unrelated local/offline data remain intact throughout.
- [x] No-remote invitations have honest local-only output and stable aliases.
- [x] Native gates pass, browser evidence or its absence is recorded, and the plan
  index records completion and any limitations.

## Stop conditions and maintenance notes

Stop and explain the concrete blocker if ordinary claim requires unavailable
account authority even for an open invitation, supported formats cannot be safely
distinguished, or the implementation appears to require changing signed scopes,
service authorization, or browser ownership semantics. Do not invent a new
redemption system or restore account management commands to get tests passing.

Future invitation versions must add explicit parser/validator support and routing
tests. Reviewers should scrutinize failure recovery, recipient checks, typed
dispatch, shortcut resolution count, and absence of agent receipts for ordinary
joins. Shared UX does not justify erasing format-specific authority checks.
