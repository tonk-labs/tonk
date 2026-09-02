# Tonk product Storybook

A visual, test-oriented description of every user journey that enters through
`tonk` or the Tonk browser shell. It pairs canonical screenshots with the
durable state changes, failure boundaries, and evidence needed to prove each
journey works.

## Open the explorer

From the repository development shell, run `dev:storybook`, then open
<http://127.0.0.1:4173/docs/storybook/app/>. The explorer is a local repository
tool and is not included in Tonk's deployed Cloudflare assets.

The static files are dependency-free. Search by user language or stable IDs,
open any screen to see its source owners and flows, and use the Gaps view to
separate unrun checks from known findings. The Markdown files remain the
reviewable source of truth; `screens.json` connects them to the visual layer.

## Purpose

Tonk is a local-first product spread across a command-line program, a browser
shell, a service worker, account and access services, and repositories that may
be local, hosted, or shared. From the user's point of view it is one state
machine. A browser account ceremony can authorize a waiting CLI; a CLI-created
space can later become account-owned; a local logout can leave all local spaces
usable while removing account services.

Those behaviors are currently defined implicitly across command dispatch,
custom elements, worker routes, account protocols, and tests. Raw test volume
does not show whether a whole journey survives cancellation, a lost response, a
second device, or a restart. This storybook is the outside-in source of truth
for that coverage.

The immediate objective is to make hot paths auditable, especially account
creation, activation, login, logout, browser/CLI handoff, device revocation,
space ownership, and deletion. A journey is not called covered merely because
one of its functions has a unit test.

### What this is not

- Not API documentation. Request and response types remain in the Rust crates.
- Not organized by crate. A behavior is described once where the user meets it,
  even when `tonk-ui`, `tonk-cli`, `tonk-worker`, and an account service all
  participate.
- Not a line-coverage target. Line coverage can help find dead regions, but the
  acceptance unit here is an observable journey and its recovery boundaries.
- Not proof that every journey works. Runtime captures prove the appearance of
  six reachable browser states at one commit; source fixtures and CLI captures
  prove authored output only. The recovery matrix remains unverified until its
  checklist is executed.

## Conventions

- Describe the experience first. Put implementation details in a block quote
  beginning `Technical note:` only when the mechanism changes the expected
  behavior.
- Use the [glossary](glossary.md) for `profile`, `device`, `root`, `account`,
  `attachment`, `space`, `provider`, and `customer`.
- Distinguish local identity, provider attachment, account-repository state,
  customer activation, space ownership, and sync state. “Signed in” is not a
  substitute for those six independent facts.
- Distinguish source evidence from executed evidence. `Drafted` means the code
  and existing tests were read. `Verified` requires the relevant P1 and P2
  checklist items to pass or to be filed in [bug triage](bug-triage.md).
- Every feature document ends with the source commit and its open questions.
  Surprising current behavior stays visible rather than being normalized.

## Interaction shape

Browser form lifecycles and CLI invocations use the same five phases:

1. **Resolve.** Select the profile, account, space, target, flags, route, and
   output mode; validate anything that can fail without side effects.
2. **Exit early.** Help, usage errors, no-ops, declined confirmation, already
   satisfied work, and unavailable prerequisites finish without crossing a
   durability or authority boundary.
3. **Cross a boundary.** The first passkey ceremony, local durable write,
   callback registration, remote mutation, or transaction commit makes aborting
   potentially non-free.
4. **Remain in flight.** Progress, retries, remote synchronization, callbacks,
   and concurrent changes can occur while the interaction is incomplete.
5. **Settle.** Commit or roll back, report an unambiguous result, and leave a
   state from which retry is safe.

Every feature document asks about the same interrupts in the same order:

1. Explicit abort: Cancel, Back, declined confirmation, or Ctrl-C.
2. Competing user action: navigate, switch profile or space, or run another
   command while the first action is unfinished.
3. Alternate completion: a callback, blur/Enter submit, or another actor
   completes the same target.
4. Service failure: offline, timeout, non-2xx response, malformed response,
   expired session, or passkey rejection.
5. Surface termination: reload, tab close, browser crash, terminal close,
   SIGTERM, or process crash.
6. Concurrent target change: another tab, process, or device edits, deletes,
   revokes, suspends, or replaces the target.
7. Input or context change: autofill, authenticator change, TTY becoming a pipe,
   stdin closing, current directory changing, or environment precedence.
8. Local durability failure: state is locked, read-only, full, missing,
   malformed, or only partly written.

Cross-cutting concerns appear in this order: identity and account authority;
local durability; remote service and sync; concurrency and multi-device;
output, errors, and recovery; accessibility, TTY, and machine output; privacy
and telemetry.

## Method

For each journey:

1. Read the user entry point and the state owners it calls.
2. Read all tests that touch any stage, then identify which stages are still
   inferred rather than exercised together.
3. Describe the normal path, variants, failure boundaries, interrupts, durable
   postconditions, and safe retry.
4. Add observable verification items. Every non-trivial interrupt and every
   suspected bug gets a P1 or P2 item.
5. Implement missing automated coverage at the lowest layer that proves the
   invariant, plus one whole-journey test for every hot path.
6. Run the checklist against the built product. File failures before marking a
   document verified.

## Scope decisions

- **Entry-point boundary.** The catalog includes every journey entered through
  `rust/tonk-cli` or `rust/tonk-ui`. Downstream worker, identity, account,
  access-service, portal, and guest behavior is in scope when it changes an
  observable outcome, but is not catalogued as a separate product.
- **Browser content.** Hub and rendered-space interactions are included at the
  routing, authority, navigation, and error boundaries owned by the browser
  shell. Component-specific editor or viewer behavior belongs in its own
  description unless the top-level route makes it unreachable.
- **Combinatorics.** “Every possible flow” means every meaningful equivalence
  class and state transition, not the Cartesian product of every flag and
  network status. Pairwise state coverage is required; triples are required
  when a known invariant spans three axes.
- **Source pins.** The written flow audit is pinned to commit `a3f8670b1`; the
  visual inventory is pinned separately to commit `49a873a23`. A screenshot at
  the visual commit does not silently upgrade the older recovery audit.
- **Environment boundary.** These are local source and runtime artifacts.
  Staging and production behavior must be verified separately rather than
  inferred from a local deployment.

## Structure

```text
README.md                              purpose, method, structure, and progress
goal.md                                standing drafting and verification rules
glossary.md                            shared product vocabulary
journey-catalog.md                     complete entry-point and journey inventory
bug-triage.md                          suspected defects found during the audit
screens.json                           screen inventory, provenance, and journey map
AGENTS.md / CLAUDE.md                  standing agent entry and update rules

app/
  index.html                           dependency-free visual explorer
  data.json / data.js                  deterministic generated product map
  screens/                             canonical browser and CLI screenshots

capture/
  README.md                            capture protocol and evidence labels
  fixture.html                         production-source and transcript renderer
  cli/                                 isolated, captured CLI transcripts

scripts/
  build.py                             inventory, coverage, and freshness validator
  check-links.py                       local documentation and asset link validator

foundations/
  state-model.md                       orthogonal account, space, and sync states
  coverage-model.md                    evidence layers and definition of coverage

accounts/
  lifecycle.md                         create, activate, login, switch, and logout
  browser-cli-handoff.md               browser approval of a waiting CLI
  authority-and-deletion.md            passkeys, devices, revocation, and deletion

spaces/
  lifecycle-and-collaboration.md       local, owned, joined, synced, and revoked spaces

cli/
  command-surface.md                   every command family and shared CLI behavior

ui/
  routing-and-runtime.md               boot, routes, account gate, and activation page

cross-cutting/
  failure-and-recovery.md              common failure injection and recovery rules

verification/
  README.md                            how to run and record a verification pass
  accounts.md                          account and hybrid account/CLI checklist
  cli-spaces-ui.md                     CLI, space, collaboration, and browser checklist
```

## Coverage

Status is one of `not started`, `drafted`, or `verified`. `Drafted` is source
coverage only; it does not mean the corresponding checks pass.

| Document | Status |
| --- | --- |
| `glossary.md` | drafted |
| `journey-catalog.md` | drafted |
| `bug-triage.md` | drafted |
| `foundations/state-model.md` | drafted |
| `foundations/coverage-model.md` | drafted |
| `accounts/lifecycle.md` | drafted |
| `accounts/browser-cli-handoff.md` | drafted |
| `accounts/authority-and-deletion.md` | drafted |
| `spaces/lifecycle-and-collaboration.md` | drafted |
| `cli/command-surface.md` | drafted |
| `ui/routing-and-runtime.md` | drafted |
| `cross-cutting/failure-and-recovery.md` | drafted |
| `verification/accounts.md` | drafted |
| `verification/cli-spaces-ui.md` | drafted |
| `screens.json` | drafted |
| `capture/README.md` | drafted |
| `app/index.html` | drafted |

No document is verified yet. The first pass should run the account P1 items,
then the hybrid browser/CLI items, then destructive space and account actions.
The current map contains 26 screen families, 78 stable journey IDs, 112
verification items, and 6 source-pinned triage findings. Fifteen browser screen
families have image evidence; the eleven CLI families use isolated transcripts
captured from the binary at the visual commit.

## Reference

The relevant source locations are:

- [`rust/tonk-cli/src/bin/tonk.rs`](../../rust/tonk-cli/src/bin/tonk.rs): CLI
  entry points, flags, help, and command dispatch.
- [`rust/tonk-cli/src/account_session.rs`](../../rust/tonk-cli/src/account_session.rs):
  native account-session state and locking.
- [`rust/tonk-cli/tests`](../../rust/tonk-cli/tests): process and integration
  tests for command behavior.
- [`rust/tonk-ui/src/bin/ui.rs`](../../rust/tonk-ui/src/bin/ui.rs): top-document
  route selection.
- [`rust/tonk-ui/src/account.rs`](../../rust/tonk-ui/src/account.rs): browser
  account lifecycle and settings interaction state.
- [`rust/tonk-ui/src/activate.rs`](../../rust/tonk-ui/src/activate.rs): emailed
  activation-link surface.
- [`rust/tonk-ui/src/account_flow.rs`](../../rust/tonk-ui/src/account_flow.rs):
  real-browser account and browser/CLI integration tests.
- [`rust/tonk-fab`](../../rust/tonk-fab): rendered-space navigation, share,
  switching, sync, and appearance controls.
- [`rust/tonk-portal`](../../rust/tonk-portal): rendered-space host and routing.
- [`flake.nix`](../../flake.nix): repository-defined E2E command.
