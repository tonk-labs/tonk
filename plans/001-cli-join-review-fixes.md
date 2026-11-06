# Restore local-space linking and space-only status

Status: DONE. Planned at `ea730cc18`; implemented and verified 2026-09-18.
Priority: P1 linking, P2 status. Effort: L linking, S status. Risk: high at the ownership boundary, low for status presentation.

This plan addresses the two review comments on `tonk join`. The checkpoints below are retained as the implementation rationale and maintenance contract; the completed paths and fresh verification evidence are recorded here.

## Implementation result

- `rust/tonk-invite/src/local_space_link.rs` defines the signed request, browser consent, service binding, exact subject/account/recipient/correlation checks, expiry and scope validation, and replay protection.
- `rust/tonk-cli/src/space_link.rs` owns eligibility, loopback delivery, browser handoff, durable stage recovery, local founder/remote/upstream/push publication, and the retained space-only signer used after restart. `rust/tonk-cli/src/bin/tonk.rs` exposes only `tonk space link NAME [--no-open] [--via URL]`; public account commands and root `tonk link` remain rejected.
- `rust/tonk-worker/src/router/local_space_link.rs` implements approval, browser provisioning, and completion endpoints. `rust/tonk-workspace/src/ui_account_settings.rs` and `.html` provide explicit approve/decline UI while keeping account credentials in the browser.
- `rust/tonk-ui/src/register_dialog.rs` preserves a validated `/settings/link?intent=local-space-link&request=...` route across fresh account creation. `rust/tonk-ui/src/account_flow.rs` covers signed-in success and completion retry, cancellation, and fresh-account continuation in a real browser.
- `tonk status` now emits `tonk.status.v3` with selected-space sync and authority only; cached CLI account/session state is neither read into nor rendered by that report.

Fresh verification after the final source changes:

- Protocol and CLI authority: 4 `tonk-invite` plus 3 `tonk-cli` `local_space_link` tests passed.
- Parser and subprocess regressions: 19 parser tests, 5 connection-command tests, and the combined 62 `cli_space` + 5 connection-command + 7 context tests passed.
- Ordinary native CLI suite: `cargo test -p tonk-cli --locked` passed; the released-executable compatibility case remained ignored because `TONK_OLD_CLI` was not supplied.
- Service-backed cached-account compatibility: 8/8 `space_link` integration tests passed. This is compatibility evidence, not browser-handoff evidence.
- Real browser handoff: 3/3 filtered `local_space_link` tests passed against freshly rebuilt CLI and production UI artifacts, with 89 unrelated tests skipped.
- Quality gates: strict all-workspace Clippy, `cargo fmt --all -- --check`, and `git diff --check` passed.

## Intended behavior

- `tonk space new garden` continues to create a usable local space.
- `tonk space link garden` opens Tonk to select an account and explicitly approve attaching that exact local space. CLI login is unnecessary. Preserve its DID, content, site path, name, and directory bindings.
- Only a locally controlled, local-only space qualifies for a new link. Joining someone else's space never confers ownership. A completed link cannot transfer to another account.
- `tonk join INVITE_LINK` continues to import scoped access to an existing space, with no browser ceremony.
- `tonk status` and `tonk status --json` describe the selected space, its sync state, and its authority. Neither reports a cached CLI account session.

Use ordinary UCAN delegation, attenuation, expiry, and revocation. Handoff correlation and publication progress are delivery/recovery metadata, not a second authorization system. This local-space ownership transition is distinct from the older proposal for browser approval to access existing hosted spaces; do not resurrect that broader workflow.

## Current state and entry points

`rust/tonk-cli/src/bin/tonk.rs`:

- `SpaceCommand` (around line 520) has no `Link` variant. Its new-space help directs users to `join`.
- `account_spaces_parser_tests::account_management_and_browser_linking_are_not_cli_commands` explicitly rejects `vec!["tonk", "space", "link", "garden"]`. Other account commands and root `tonk link` should remain rejected.
- Update `space_op` dispatch and command telemetry classification when adding the exception.
- `status_op` already constructs `ScopedAuthorityReport { kind, subject, recipient, grant_ids }`, then separately reads `site.account_store.account()` for scoped sites and reports `signed_in: cached.is_some()`. Non-scoped sites call `account::status_in`. `StatusReport` requires an account section and uses `tonk.status.v2`.

`rust/tonk-cli/src/space_link.rs` retains useful ownership and publication logic, but is not a browser implementation. `prepare` does this:

```rust
let account = registry
    .account
    .clone()
    .context("no account is signed in; run `tonk account login` first")?;
```

It also calls `account::status_in`. Do not wire `Link` straight to this function. Reuse its preflight checks, founder/authority consistency, and idempotent publication stages: founder, remote, upstream, push, account directory. Its checks already reject scoped connection adoption.

Relevant existing tests and conventions:

- `rust/tonk-cli/tests/space_link.rs`: real account/access-service tests behind `integration-tests`; preserves identity and checks founder, upstream, and account directory. Its cached-account fixtures do not prove browser linking.
- `rust/tonk-cli/tests/connection_commands.rs`: isolated subprocess fixtures, explicit state directories, disabled telemetry/update checks. `removed_workflows_preserve_existing_local_state` currently treats space linking as removed. `connection_command_imports_bearer_restarts_and_keeps_account_state` verifies authority but does not assert absence of account output.
- `rust/tonk-cli/tests/cli_space.rs`: status JSON and offline fallback regression tests, including the v2 schema assertion.
- `rust/tonk-cli/src/connections.rs`: isolated invitation authority and binding validation; preserve it.
- `rust/tonk-invite/src/connection.rs`: ordinary scoped invitation validation and expiry bounds; reuse conventions without changing the invitation format.
- `rust/tonk-worker/src/router.rs`: API registration and existing repository/invitation test patterns. `rust/tonk-ui/src/register_dialog.rs` and `rust/tonk-ui/src/account_flow.rs` provide account UI and browser-test entry points. Residual `/settings/link` references do not establish that an approval route still works.
- `plan/space-linking.md` describes browser provisioning-before-attachment and account activation. Treat named handlers there as historical pointers to trace, not confirmed current symbols.

Match existing Rust `Result`/typed-error patterns and tempfile-based tests. Never log invite secrets, private keys, or callback credentials.

## Scope and preparation

Start with `git status --short` and:

```sh
git diff --stat ea730cc18..HEAD -- rust/tonk-cli rust/tonk-invite rust/tonk-ui rust/tonk-worker
```

Compare changed code against the current-state notes before executing. Preserve the three existing untracked `did:key:*` directories; they are not disposable fixtures.

Primary scope: CLI command/status implementation, `space_link.rs`, their tests, CLI README/help, and this plan/index. The linking slice may add a dedicated handoff protocol module and tests under `tonk-invite`, a dedicated browser approval component under `tonk-ui`, and a narrow worker route under `tonk-worker`. Record the exact new paths after checkpoint 1 identifies the current integration points.

Out of scope: account login/logout restoration, root `tonk link`, multi-space linking, ownership transfer, invitation redesign, dependency/lockfile changes, broad account-storage migration, and service-side redemption/activation tables. Legacy account storage remains available internally. Do not delete or rewrite existing local replicas.

No commit, push, or PR publication is requested. Keep each checkpoint reviewable; if commits are subsequently requested, commit each passing logical checkpoint separately.

## Checkpoint 1 — Prove the browser-to-local ownership handoff

Before exposing the command, trace current browser account selection, provisioning, account-directory publication, and local repository ownership proof. Specify the minimal request/response types in this document with exact module paths, then implement a tested protocol slice.

The slice must establish this sequence:

1. Resolve the named space independently of ambient selected-space state. Verify local ownership authority, absence of a founder/account ownership, absence of hosted remotes, and absence of scoped-import markers. Reject unreadable or inconsistent ownership state rather than treating it as local-only.
2. CLI starts a request bound to the existing space DID and a locally retained signing identity. Browser displays that space and lets the user select/authenticate an account and approve. Account credentials and private keys stay in the browser.
3. Bind the selected account, service, space, recipient, and consent to the exact request. Only after consent does the local owner issue the ordinary space authority needed by that account. The browser uses its own account authority for provisioning and directory publication; the CLI receives only the authority needed for this space.
4. Verify signatures, audience, subject, scope, expiry, trusted service routing, and request correlation before installing returned authority. A callback containing an account DID or a success flag alone is insufficient proof.

Use a loopback delivery channel only with loopback binding, an unpredictable correlation value, explicit origin validation, bounded payloads/timeouts, and request replay rejection. Supply a printable URL fallback when browser opening fails. Request state must not serve as authorization. Keep private key material out of URLs and telemetry.

**Verification:** introduce tests named with `local_space_link` in the protocol and CLI modules, then run:

```sh
cargo test -p tonk-invite --locked local_space_link
cargo test -p tonk-cli --lib --locked local_space_link
```

Both must execute nonzero tests and pass. Cover valid consent plus wrong subject, account, recipient, correlation, expired/overbroad grant, replay, and cancellation. No founder, remote, or account-directory publication may occur before approval. This is the first uncertainty gate: do not broaden the command/UI until the authority exchange is proven.

## Checkpoint 2 — Restore the command and finish publication

Add `SpaceCommand::Link { name, ... }`, help, dispatch, and telemetry classification. Support `--no-open` to print the approval URL for headless use. Keep account selection in Tonk; do not add CLI account-selection/login flags or reuse ambient cached accounts. Update the parser regression to allow only this specific linking exception.

Split reusable publication operations from the old cached-account adapter. Drive provisioning and account-directory work through the approved browser context, and local founder/remote/push work through verified space authority. Recheck ownership immediately before committing the transition so concurrent requests cannot choose different owners.

Preserve existing identity and data. Persist only the information needed to retry interrupted publication. An approved but incomplete link may resume only for the same space/account; a completed repeat is idempotent. Report the failed stage and retain local editability. Do not report success until content/metadata and the account directory are published. If the browser closes, give a recoverable outcome rather than falsely reporting completion.

Extend `space_link.rs` tests for eligibility, cancellation, concurrent conflicting approvals, partial-stage failures/retry, and identity preservation. Update `removed_workflows_preserve_existing_local_state` so invalid link attempts still preserve data without requiring that the parser reject the command.

**Verification:**

```sh
cargo test -p tonk-cli --bin tonk --locked account_spaces_parser_tests
cargo test -p tonk-cli --test connection_commands --locked
```

Both must pass, including positive parsing for `space link garden`, continued rejection of public account commands/root `link`, and no browser behavior added to `join`.

## Checkpoint 3 — Make status space-only

Remove account-session construction and rendering from `status_op` for both scoped and non-scoped spaces. Remove the `account` field from `StatusReport`; bump its schema to `tonk.status.v3` and document the deliberate removal. Keep shared `AccountContext` machinery wherever compatibility callers still need it.

Retain selected-space and sync fields. Scoped output uses the existing binding-backed authority fields: kind, subject, recipient, grant IDs. Local-only output identifies local access without inventing a grant or account. Preserve honest `legacy` classification for old remotes when no scoped binding exists. Never call a stored grant valid merely because it is present; retain the distinction between stored authority and remote validity checked when used.

Extend `connection_commands.rs` to compare text and JSON status for the same invitation with no cached account and with an unrelated cached account. Both succeed, expose identical authority, contain no account/session section, and preserve the cached record on disk. Missing/mismatched bindings must still fail closed. Update `cli_space.rs` schema assertions and local/offline/legacy status coverage.

**Verification:**

```sh
cargo test -p tonk-cli --test connection_commands --test cli_space --test context --locked
```

Require passing assertions for v3, absence of `account`/`signedIn` in status, unchanged scoped recipient/subject, and preserved offline sync fallback. Do not globally remove account-related words from ownership inventory or compatibility tests.

This checkpoint can be implemented independently if browser linking takes longer; it does not depend on introducing new handoff machinery.

## Checkpoint 4 — Browser proof and integration

Add a real-browser case in `rust/tonk-ui/src/account_flow.rs`, named with `local_space_link`, using the suite's existing CLI/browser fixture patterns. Start with a fresh CLI state and a local `garden` containing an identifiable fact. Approve in Tonk, verify the same DID and fact appear in the account, restart the CLI, and verify scoped remote operations without CLI login. Repeat with an unrelated legacy CLI account cached; browser selection must determine the owner. Test cancellation and account creation/sign-in continuation without losing the pending space.

Run the repository browser harness with the test filter:

```sh
nix develop --accept-flake-config .#ci --command test:e2e local_space_link
```

Require nonzero executed tests; check nextest filtering/quarantine if none execute. Add a browser retry case for interrupted publication. Existing signed-in `space_link` tests remain useful compatibility coverage but are not a substitute for this flow. Run those service-backed tests using the repository native integration harness after checking its fixture setup; an unconfigured standalone Cargo run is not service evidence.

Final native gates:

```sh
cargo fmt --all -- --check
cargo test -p tonk-cli --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
git diff --check
```

These commands must exit zero. Update CLI README/help with the two concrete examples (`space new` then `space link`, versus `join INVITE_LINK`) and the status v3 contract. Report browser, service-backed native, and ordinary native results separately; record infrastructure failures rather than calling unrun checks passed.

## Completion and stop conditions

Done requires passing focused authority, parser, subprocess status, and real-browser linking tests after the last source change; unchanged local identity/content/bindings; preserved legacy storage; and no public CLI account-session output from status. Update this plan and the index with results and any remaining limitations.

Stop the affected checkpoint and report if browser publication requires account-wide authority in the CLI, the existing space cannot delegate ownership authority safely, or safe retry requires weakening ownership checks. Also report if implementation requires service-side authorization state, ownership transfer, destructive migration, or dependency changes outside scope. Independent status work can proceed while a linking design blocker is resolved. For environment failures, identify the failing command and access boundary before changing source.

Maintenance: future invitation, custody, and roster changes must preserve the distinction between local ownership adoption and delegated access. Future status schema changes must keep account-session state separate from the authority actually selected for a space.
