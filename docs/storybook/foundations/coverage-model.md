# Coverage model

## Summary

Coverage is evidence that a user journey reaches the right durable state and
recovers safely from meaningful failures. Tonk needs several test layers
because no single layer can cheaply prove parsing, authority, browser timing,
remote contracts, restart behavior, and multi-device convergence.

## Evidence layers

| Layer | Proves | Does not prove |
| --- | --- | --- |
| Pure/unit | State transition, parser, validation, output shaping, idempotency logic. | Filesystem durability, browser behavior, service contracts. |
| Boundary contract | Every status/error envelope, timeout mapping, malformed response, authorization rule. | Whole-page timing or process restart. |
| Store/transaction | Atomic write, lock behavior, rollback, migration, corruption handling. | UI affordance or remote convergence. |
| DOM/component | Visible modes, focus, disabled controls, form arming, error text. | Real WebAuthn, service worker, browser navigation, callback networking. |
| CLI process | Exit code, stdout/stderr, TTY/stdin, signals, filesystem state after process exit. | Browser handoff unless paired with a browser. |
| Real browser | Routes, custom elements, service worker, WebAuthn, navigation, reload, accessibility. | Native CLI state unless paired with the built binary. |
| Hybrid E2E | Browser-authorized CLI and account/space convergence across surfaces. | Production deployment-specific configuration unless run there. |
| Fault/restart | Lost response, crash point, lock contention, duplicate request, retry safety. | Human comprehensibility unless output/UI is also asserted. |
| Deployment smoke | Production/staging routing, origins, credentials, and deployed service integration. | Exhaustive destructive and rare error cases. |

## Definition of covered

A journey is covered only when all applicable columns have executed evidence:

| Dimension | Required evidence |
| --- | --- |
| Resolve | Correct route/command, state selection, precedence, and validation before side effects. |
| Normal completion | Observable success and the durable state after restart. |
| Rejection | Invalid input, unauthorized target, wrong account/root, and already-completed target leave the promised state. |
| Service errors | Offline, timeout, relevant non-2xx codes, malformed body, and response lost after commit. |
| User interruption | Cancel/Back/Ctrl-C and competing action before and after the first side effect. |
| Process/page interruption | Reload, tab close, SIGTERM/crash, and restart at every non-atomic boundary. |
| Concurrency | Duplicate submit plus same target changed by another tab, process, or device. |
| Local durability | Locked, full/read-only, malformed, unsupported-version, and partial legacy state where applicable. |
| Recovery | Retry, resume, reconcile, or explicit manual recovery is safe and explained. |
| Output contract | Human output, JSON/notation, stdout/stderr, exit code, busy/disabled state, and actionable error agree. |
| Boundaries | Unrelated profiles, accounts, spaces, devices, joined data, and local replicas remain untouched. |

Line coverage is useful as a discovery signal, but there is no configured
Rust coverage gate in this worktree. A future line-coverage report should be
reported beside, not instead of, this journey matrix.

## Current static baseline

These counts were collected from source attributes at commit `a3f8670b1`; they
were not test executions.

| Surface | Static evidence | What it reveals |
| --- | --- | --- |
| CLI integration files | 308 test attributes in `rust/tonk-cli/tests/*.rs`. | Broad command and repository coverage, but not a systematic journey/error matrix. |
| CLI account process interruption | 2 tests in `account_interrupt.rs`. | Only Ctrl-C during callback wait and a fresh next callback are pinned. |
| CLI account session | 0 focused tests in `account_session.rs`. | Versioning, malformed state, locks, waiting/activating transitions, crash points, and detach retry are unproved at their owner. |
| UI total | 56 test attributes in `rust/tonk-ui/src`. | 27 account DOM/unit tests and 19 real-browser account flows dominate the suite. Native E2E discovery lists 23 tests; the remaining 33 attributes require the separate web/native target configurations. |
| UI native E2E discovery | 23 tests: 19 account browser flows, 2 API tests, 1 deployment test, and 1 route-shell test. | The repository E2E command does not itself execute all 56 static UI attributes; full target coverage also requires `test:web:*` and the ordinary native suite. |
| UI activation | 0 tests in `activate.rs`. | Missing/damaged/expired links, network failure, retry, duplicate activation, and receipt reporting are inferred only. |
| UI custody relay | 0 tests in `custody_relay.rs`. | Relay denial, malformed payload, timeout, and page lifecycle are unproved locally. |
| UI route shell | 1 unit test in `src/bin/ui.rs`. | Canonical account redirect is checked; actual route mounts and runtime failures are not comprehensively exercised. |

The repository E2E command builds the CLI and runs `tonk-ui` with
`integration-tests` serially. Fresh `--list` discovery at this commit found 23
native tests, including the 19 browser flows. That is a valuable whole-system
layer, but those browser tests are concentrated on successful account
lifecycles and a few authority outcomes. They do not enumerate network,
restart, malformed-state, or concurrent-actor failures. The 33 other static UI
attributes were not listed by this native E2E configuration and need the
repository's separate web/native target suites.

## Coverage ownership

| Journey kind | Minimum automated owner | Whole-journey evidence |
| --- | --- | --- |
| Account state transition | `tonk-account`/worker or `account_session` transition test. | Browser or CLI lifecycle test across restart. |
| Browser API error | Table-driven API/worker contract test. | One browser test per distinct visible recovery pattern. |
| Passkey ceremony | Identity-bridge validation tests. | Real Chrome virtual authenticator: success, cancel, wrong credential, PRF absent. |
| CLI invocation | Parser/output/state test using isolated home. | Spawned binary for exit code, streams, TTY/signal, and restart. |
| Sync/remote | Deterministic fake remote plus branch-state assertions. | Two repository actors for ahead/behind/diverged/revoked. |
| Destructive action | Plan/execute transaction tests with unrelated fixtures. | Browser/CLI confirmation and post-delete recovery. |
| Hybrid handoff | Callback payload and session transition tests. | Built CLI plus real browser, including decline, timeout, lost response, restart. |

## Priority

**P0** protects authority or data on a common path: account creation/login,
session durability, logout, device revocation, account/space deletion, space
ownership, and response-lost-after-commit.

**P1** protects common work and recovery: activation, add/switch profile,
account-backed space sync, invite claim/revocation, selection, write auto-sync,
and stable machine output.

**P2** covers less common commands and bounded failures. **P3** covers visual
geometry, copy, timing, and platform variants unless another document depends
on the exact value.

## First implementation slices

1. Add table-driven `account_session` tests for every state/event pair,
   including the latent waiting-logout save error and process contention.
2. Make CLI login crash points injectable and prove restart behavior before
   and after grant, root, provider, active-session, hydration, and push stages.
3. Add activation-page component and real-browser tests for every response
   class and a reload/duplicate-submit pass.
4. Add browser API contract tests so every non-2xx response yields the intended
   user-facing message rather than a JSON decoder error.
5. Add account-create checkpoints for duplicate email, passkey cancellation,
   local-save failure, remote commit with lost response, attachment failure,
   activation pending, and retry without extra authority.
6. Add destructive boundary tests for owned versus joined spaces, unrelated
   profiles/accounts, stale plans, duplicate execution, partial service failure,
   and restart.
7. Convert account-related direct `#[tokio::test]` integration tests to the
   repository's cross-target test harness where applicable, then verify the CI
   and native variants still discover them.

## Open questions and verification

- Static test counts should be replaced with discovered and executed test lists
  in the first verification pass.
- No coverage percentage is currently available; adding a threshold is a
  separate decision after excluding generated and platform-only code.
- The correct deployment smoke target and safe destructive fixture account
  still need to be chosen.

Source audit pinned to Tonk commit `a3f8670b1`.
