# Verification protocol

The feature documents were drafted from code and existing tests. This directory
turns their claims into observable checks against a built product. A source
assertion or old green check is not a result for this pass.

## What is here

| File | Covers |
| --- | --- |
| [accounts.md](accounts.md) | `accounts/*`, account portions of `foundations/*`, and hybrid browser/CLI flows. |
| [cli-spaces-ui.md](cli-spaces-ui.md) | `cli/*`, `spaces/*`, `ui/*`, and shared failure/recovery behavior. |

Each row has a stable ID, priority, required condition, one observable claim,
precise setup and steps, expected result, and a Result cell. Priorities are:

- **P1:** account/authority/data hot path, a state-model foundation, suspected
  bug, or destructive/concurrency/restart boundary;
- **P2:** ordinary supported behavior and recoverable error;
- **P3:** exact copy, geometry, timing, responsive layout, or platform detail.

Use `pass`, `fail`, or `blocked` in Result. Add a short reason after anything
other than `pass`. A fail is a mismatch with the document, not automatically a
production bug; update [bug triage](../bug-triage.md) with the decision.

## Before a pass

1. Run `git rev-parse --short HEAD` and record it. The current documents describe
   `a3f8670b1`; a different build is a drift check, not direct confirmation.
2. Use disposable local services or a dedicated destructive-test deployment.
   Never run delete/revoke fixtures against a real personal or production
   account.
3. Build the CLI used by browser integration. The repository's automated E2E
   entry point is:

   ```sh
   nix develop --accept-flake-config .#ci --command test:e2e
   ```

   It builds `tonk-cli` and runs `tonk-ui` integration tests serially. Record
   the exact binary path when running process/manual checks.
4. Create isolated browser profiles and CLI homes. Record every state override
   (`HOME`, XDG paths, Tonk state paths, offline/update/telemetry variables) so
   a later pass is reproducible.
5. Configure a virtual authenticator with PRF for the normal path and a second
   condition without PRF where the fallback is supported. Record credential
   counts before and after every account-create/add-passkey test.
6. Seed fixtures with stable subjects/DIDs and at least: two accounts, two
   devices, one local-only space, two owned spaces, one joined space, one
   revoked invite, duplicate display names on distinct subjects, and an
   unrelated profile.
7. Install deterministic fault controls at named checkpoints. Arbitrary sleeps
   are not acceptable evidence for response loss or process interruption.

The audit did not find a documented long-lived manual launcher for the full
account stack. Until one is identified, use the existing `TestEnvironment` for
automated browser checks and record manual destructive items as `blocked`, not
as implicit passes.

## Conditions

- **fresh-browser:** empty isolated browser profile, new virtual authenticator,
  service worker not yet controlling the first navigation.
- **returning-browser:** same profile/authenticator after page and browser
  restart.
- **second-browser:** independent browser profile and device DID; a second tab
  is not a second device.
- **cli:** built binary with a fully isolated native profile/account/space store.
- **hybrid:** built CLI plus real Chrome; loopback callback is real networking.
- **offline:** fail the named service connection. Browser DevTools offline does
  not necessarily interrupt a request already accepted by a service.
- **fault:** deterministic service/store failpoint before or after a named
  commit, including dropped response after acceptance.
- **restart:** close/kill after a readiness barrier, then inspect with a new
  page/process. Do not infer restart behavior from an in-memory object.
- **two-actor:** separate stores/devices/services where appropriate; not two
  async tasks sharing one actor.
- **TTY / pipe:** a real pseudoterminal versus ordinary piped streams. A
  `--no-color` flag is not the same as a pipe.
- **locked/full/corrupt:** an isolated fixture with a held OS lock, bounded or
  read-only storage, malformed state, or unsupported version. Do not damage a
  real user store.

## Running a pass

1. Work through all P1 items across both files first.
2. Read the linked feature section before each row; the document is the claim
   and the row is its executable summary.
3. Observe visible output and durable state. Account checks record selected
   profile DID, device/root DID, provider, attachment, customer and account
   repository status. Space checks record subject, site, registry, binding,
   owner, upstream, and branch heads.
4. For shared facts, confirm from a second actor after explicit sync.
5. For any mutation failure, rerun status and the documented recovery in a new
   process/page. A matching first-screen error alone is not a pass.
6. File every fail under the existing triage root cause or add a new entry with
   the checklist ID. Record screenshots/logs outside this description only if
   they contain no credentials, UCANs, callbacks, or passkey material.
7. A document becomes `verified` only after all of its P1 and P2 rows have
   passed or been filed with an accepted disposition.

## Automated implementation rule

When turning a row into a regression test:

- prove the invariant at the lowest deterministic layer;
- prove the hot-path experience once through the whole browser/process stack;
- force the test to fail before the fix or with the fault enabled;
- assert durable state from a fresh actor after the operation; and
- include the checklist/journey ID in the test comment or name mapping.

For Rust cross-target tests, use the repository test harness described in
`.claude/skills/testing/SKILL.md` unless the test is deliberately native
process-only. Real-browser tests use the repository `TestEnvironment` and run
serially where shared listeners/locks require it.

## Results so far

### Visual inventory pass — 2026-08-27

At visual commit `49a873a23`, the repository web stack built and served at
`http://127.0.0.1:8080/` after removing an inherited `NO_COLOR=1` value that
mdBook rejects as a boolean. An isolated headless Chrome profile at 1440 by 960
captured `WEB-01` through `WEB-06` from the running product: empty Hub, populated
Hub, space home, expanded space actions, account-required share, and invalid
space route. These captures prove those visible states only; they do not pass
the linked account, authority, sync, interruption, or restart checks.

`WEB-07` through `WEB-15` were captured from the production-source fixture,
which fetched the checked-in account/activation HTML and CSS and populated
documented test values. The fixture did not run WebAuthn, account services, or
the state transition into those screens.

The current `tonk` binary was built with `cargo build -p tonk-cli --bin tonk`.
Eleven CLI screen families were captured from exact command help or the empty
space-list result using isolated XDG, spaces, telemetry, and update paths; no
real account/profile command ran and `HOME` was not replaced. The captures prove
the displayed output, not mutation, recovery, remote, or TTY behavior.

The resulting explorer was checked in isolated Chrome at desktop and 390 by 844
viewports. Overview, Screens, Flows, Gaps, filters, search, detail routes,
screen-to-flow navigation, the compact navigation drawer, local assets, console,
and network requests were inspected. The final reload had no console messages
or failed asset requests.

### Earlier source and test pass — 2026-08-26

No checklist pass has run and no document is verified. Fresh evidence at commit
`a3f8670b1` on 2026-08-26 is limited to:

- static inventory: 308 CLI integration test attributes and 56 `tonk-ui` test
  attributes;
- `cargo test -p tonk-cli --features integration-tests --test
  account_interrupt -- --list`: compiled and discovered the expected 2 tests;
- the same CLI binary executed serially: the sandboxed attempt failed because
  the loopback callback could not bind, then the unchanged run with localhost
  access passed 2/2 in 0.81 s; and
- `cargo test -p tonk-ui --features integration-tests -- --list`: compiled and
  discovered 23 native tests—19 real-browser account flows, 2 API tests, 1
  deployment test, and 1 route-shell unit test.

The CLI execution verifies its existing pre-approval Ctrl-C and fresh-callback
assertions only; the broader `HANDOFF-04`/`HANDOFF-05` rows also require durable
state and late-callback checks, so their Result cells remain `—`. No UI test was
executed, and neither `test:web:debug` nor `test:web:release` was run. Static
counts, compilation, and discovery are not pass results for the 102 checklist
items.
