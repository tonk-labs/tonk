# CLI partial outcomes and remote deadlines implementation plan

**Goal:** Bound remote waits and ensure a durable local write is always reported before optional post-sync, while partial `space new` success names the safe state and exact convergent recovery command.
**Approach:** Add one shared remote-deadline wrapper around Dialog repository push/pull/fetch operations and expose auto-sync as a begin/local-commit/finish session. Delay the final `space new` receipt until account publication finishes; failures after local creation return a typed partial outcome whose message says local data is safe and directs the user to `tonk space link <name>`.
**Constraints:**
- A timed-out future must not roll back or hide a local commit; timeout copy must never tell the user to repeat a non-idempotent eval.
- Stdout remains machine-readable command output. Warnings, remote durability, and recovery guidance go to stderr unless a documented JSON envelope includes them.
- Pull-before failure remains a warning and local evaluation may proceed; push-after failure remains non-fatal after the local receipt.
- Default deadlines must be long enough for normal large spaces and configurable through one documented environment variable; tests inject short durations without global environment races.
- `space link` remains the single idempotent account-publication recovery path.

## File map
- `rust/tonk-cli/src/remote_deadline.rs`: shared timeout policy and typed timeout context.
- `rust/tonk-cli/src/lib.rs`: export the deadline module.
- `rust/tonk-cli/src/sync.rs`: apply deadlines to main/meta push, pull, and fetch/status.
- `rust/tonk-cli/src/remote.rs`: repair an interrupted exact remote registration without changing public `remote add` duplicate semantics.
- `rust/tonk-cli/src/auto_sync.rs`: `WriteSession` interface separating pull-before from push-after.
- `rust/tonk-cli/src/bin/tonk.rs`: flush eval receipt before post-sync and render `space new` partial outcomes.
- `rust/tonk-cli/src/space_link.rs`: expose/use stable stage names and recovery result without duplicating provisioning logic.
- `rust/tonk-cli/tests/sync.rs`: timeout and commit-before-push coverage.
- `rust/tonk-cli/tests/cli_space.rs`: final/partial `space new` receipt coverage.
- `docs/storybook/cli/command-surface.md`: stdout/stderr, deadline, and partial-success contract.
- `docs/storybook/verification/cli-spaces-ui.md`: fault/restart verification for `SPACE-08` and CLI output.

### Task 1: Bound every repository sync operation

**Files:**
- Create: `rust/tonk-cli/src/remote_deadline.rs`
- Modify: `rust/tonk-cli/src/lib.rs`
- Modify: `rust/tonk-cli/src/sync.rs:push,pull,status_with_hash`
- Test: `rust/tonk-cli/src/remote_deadline.rs`
- Test: `rust/tonk-cli/tests/sync.rs`

**Interfaces:**
- Consumes: operation label, target/upstream label, configured `Duration`, and a cancellable future.
- Produces: `remote_deadline::run(operation, target, future)` and `run_with(duration, ...)`; expiry maps to `SyncError::Timeout { operation, target, seconds }`.

- [x] Add a paused-time unit test around a never-resolving future; it must return the exact timeout variant at the injected duration and drop the future.
- [x] Add sync-level classification coverage for main push, metadata push, main pull, metadata pull, and status fetch; each timeout names the phase rather than a generic I/O failure.
- [x] Run the exact focused unit red; it ran one test and failed because the pass-through wrapper outlived the injected seven-second deadline.
- [x] Implement the repository-conventional policy: 120-second default (matching the worker's existing bounded operation) and 300-second cap (matching the callback's existing maximum wait), with `TONK_REMOTE_TIMEOUT_SECONDS` parsed as a positive bounded integer and invalid values rejected without echoing them.
- [x] Wrap each network-performing future separately so error copy identifies `push main`, `push metadata`, `pull main`, `pull metadata`, or `fetch status`.
- [x] Preserve `UpstreamNotConfigured`, forbidden/revoked, and divergence classifications; only actual expiry becomes `Timeout`.
- [x] Run focused deadline/sync tests: 3 deadline unit tests, 1 sync phase-classification unit test, and the exact hanging-remote process test all passed. The unfiltered `sync` integration target was not run; the changed process boundary is covered by its exact test.

### Task 2: Emit eval's local receipt before push-after

**Files:**
- Modify: `rust/tonk-cli/src/auto_sync.rs:run_eval,around_commit`
- Modify: `rust/tonk-cli/src/bin/tonk.rs:eval`
- Test: `rust/tonk-cli/tests/sync.rs`

**Interfaces:**
- Consumes: `TonkSite` and auto-sync enabled flag.
- Produces: `WriteSession::begin(site, enabled)`, `finish(committed) -> SyncReport`, while convenience `run_eval` remains available for callers without an external receipt boundary.

- [x] Add a behavioral test with a socket that accepts but never answers. Read the spawned CLI's stdout before expiry and assert the complete eval receipt/revision is already flushed.
- [x] Add assertions that the later stderr warning says the local write is saved, names `tonk push` as recovery, and explicitly says not to repeat the eval; process exit remains success because auto-sync is best-effort.
- [x] Run the focused process red; it ran one test and failed because stdout remained empty while the current pull/push path blocked.
- [x] Implement a deep `WriteSession`: `begin` performs warning-only pull-before; the caller performs/prints the local write; `finish` performs bounded push-after and account-directory recording.
- [x] In the CLI `eval` path, write and flush `Outcome.stdout` immediately after the local eval returns, then call `finish(outcome.committed)`. Preserve dry-run/no-sync behavior.
- [x] Keep data verb/library callers on a composed convenience path unless they have their own stdout boundary; avoid duplicating sync warning logic.
- [x] Re-run the exact hanging-remote process test (1 passed) and the complete `data_verbs` target (28 passed).

### Task 3: Report `space new` only when its actual boundary is clear

**Files:**
- Modify: `rust/tonk-cli/src/bin/tonk.rs:space_op SpaceCommand::New`
- Modify: `rust/tonk-cli/src/space_link.rs`
- Test: `rust/tonk-cli/tests/cli_space.rs`
- Test: `rust/tonk-cli/tests/space_link.rs`

**Interfaces:**
- Consumes: local `CreateOutcome`, optional active account, and named publication stages `founder`, `remote`, `upstream`, `push`, `accountDirectory`.
- Produces: final receipt only after all required stages; `CreateOutcome` plus typed `PublicationError { stage, source }` renders safe state plus `tonk space link <name>` without duplicating durable local fields inside the publication module.

- [x] Add child-scoped, integration-feature-gated fault tests after every signed-in post-create stage. Before the final stage, stdout must not start with `Registered space`; stderr must name the failed stage, local path/DID safety, and exact `tonk space link <name>` recovery.
- [x] Add a success test asserting the current receipt fields remain available after publication, and an existing-name retry message that points interrupted users to `space link` without claiming the old creation failed.
- [x] Run the focused process red; it ran one test and failed at the first `founder` case because the child ignored the injected stage and returned final success.
- [x] Extract one idempotent publication sequence used by both `space new` and `space link`. Exact remote address/subject is verified and missing metadata repaired; upstream wiring is replayed to repair a partial metadata boundary without replacing a different target.
- [x] Buffer the human final receipt until publication succeeds. On partial failure, return nonzero after printing only diagnostic/recovery text to stderr; the registry and local site remain untouched.
- [x] Ensure sync timeout uses the same partial outcome and `space link` guidance, retaining the typed remote-uncertainty message.
- [x] Run the complete `space_link` target (8 passed with loopback permission after the sandbox denied fixture startup) and the exact five-stage `cli_space` process test (1 passed after correcting its pre-canonical path expectation).

### Task 4: Document the output and recovery contract

**Files:**
- Modify: `docs/storybook/cli/command-surface.md`
- Modify: `docs/storybook/verification/cli-spaces-ui.md`

**Interfaces:**
- Consumes: timeout, eval receipt, and partial-space behavior.
- Produces: exact channel/exit/safe-state expectations for humans and agents.

- [x] Document `TONK_REMOTE_TIMEOUT_SECONDS`, default/cap, which operations it covers, and that local eval receipts precede optional remote durability.
- [x] Document final versus partial `space new` output, nonzero exit, retained local path, unknown stage outcomes, collision inspection, and `space link` convergence.
- [x] Add verification cases for a socket that accepts but never responds, SIGINT after local eval receipt, and faults after every account-publication stage.
- [x] Run `python3 docs/storybook/scripts/build.py`, `python3 docs/storybook/scripts/build.py --check`, and `python3 docs/storybook/scripts/check-links.py docs/storybook` (26 screens, 78 journeys, 116 verification items, 6 findings; 174 links valid).
- [x] Run final `cargo fmt --all -- --check`; focused sync/data/space-link checks and the complete lib target are green.

## TDD evidence and completed green verification

- RED — deadline: the exact `remote_deadline::tests::a_never_resolving_remote_is_dropped_at_the_injected_deadline` invocation ran 1 test (193 filtered) and failed on the outer guard because the initial wrapper did not finish at seven seconds.
- RED — eval ordering: the marker-verified copied CLI and exact `eval_flushes_its_local_receipt_before_a_timed_out_push_finishes` process test ran 1 test (14 filtered) and failed with empty stdout while the accepted connection still blocked.
- RED — partial creation: the marker-verified copied CLI and exact `every_partial_stage_keeps_the_local_space_and_names_one_recovery_path` test ran 1 test (65 filtered) and failed because the `founder` injection was ignored and final success was printed.
- Static after production changes, before the coordinated green window: direct `rustfmt --edition 2024` completed for every changed Rust file and `git diff --check` passed. Cargo green verification was deliberately deferred at that checkpoint.
- GREEN — deadline: `remote_deadline::tests` ran 3/3, including future cancellation and pure default/bounds parsing; sync deadline classification ran 1/1 across all five phase labels.
- GREEN — eval ordering: the freshly built, production-marker-verified copied CLI ran the exact hanging-remote process test 1/1. The receipt was readable while push-after remained in flight; the eventual zero-exit warning named local safety, remote uncertainty, `tonk push`, and no eval replay.
- GREEN — partial creation: the freshly built, production-marker-verified copied CLI ran the exact signed-in process test 1/1. Faults after `founder`, `remote`, `upstream`, `push`, and `accountDirectory` each retained the canonical site/DID/registry and converged through `space link`; final and same-name output contracts also passed.
- GREEN — regressions: `space_link` 8/8, `data_verbs` 28/28, and `tonk-cli --lib` 197/197. The first `space_link` and lib attempts were sandbox-only failures (`Operation not permitted` while binding loopback fixtures, before product behavior); unchanged commands passed with loopback permission.
- GREEN — final static/docs: `cargo fmt --all -- --check`, Storybook build and generated-data check (26 screens, 78 journeys, 116 verification items, 6 findings), 174/174 local link checks, and `git diff --check` passed.
