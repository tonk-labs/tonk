# Validate hosted documents before dispatching a write

## Observed v4 failure

The fixed-build v4 trial stopped after wave 2 because the observer correctly
rejected retained state (`recovery.required=true`). Participant 02 first called
`tonk_evaluate` with exactly `tonk/repository: {name}` at 12:52:00.414 UTC on
2026-10-20. At 12:52:38.541 it received the native evaluation route's HTTP 400
with the unknown-outcome warning. An earlier `tonk_query` with the same document
had also returned HTTP 400. The later R-SRC-01 write was blocked by its journal.

The input is valid YAML, so PR 1128's parser guard correctly accepts it. The
ordinary analyzer rejects its null literal: `field "<scalar>" value null literal
isn't supported here`. An exact dry run in an existing disposable local space
returned exit 2; reads before and afterward showed the same full revision and
zero committed claims. No evaluation-space credentials or retained state were
used, repaired, or replayed.

## Narrow repair and uncertainty boundary

After authorized pull and only when there is no earlier pending marker, invoke
the ordinary evaluate route with `transact=false` before creating the marker.
This runs canonical validation in an uncommitted transaction. If it fails,
report that no write evaluation was dispatched regardless of its HTTP status.
Then persist the marker and dispatch the actual `transact=true` call once.
Every failure after actual dispatch keeps the marker, including a later
analysis rejection caused by a race. There is no automatic replay or blanket
HTTP 400 journal cleanup.

The ordinary interactive dry-run path drops its transaction and returns no
transients to the handler. This adds a read-only evaluation, not a second
committing invocation. Validation and dispatch may observe different state;
that is why preflight does not authorize clearing a later uncertain outcome.

## Focused proof

The real local access-service fixture submits the exact captured document and
checks a stable revision, no recovery flag, and successful corrected publication
read independently. The corrected write must begin at the preceding read's
revision and then advance it, so a committing preflight would fail the test.
Its publication must also be read back exactly once. Existing retained-journal and
revoked-push recovery controls remain in the same fixture.

The baseline failed at the intended diagnostic mismatch: it returned the
unknown-evaluation HTTP 400 warning rather than the explicit no-dispatch error.
The fixture removes its own temporary state on this expected baseline failure.
Local valid-write latency is sampled before and after the change as descriptive
fixture measurements, not hosted-performance or capacity evidence.

## Completed local evidence

- Red baseline: focused target 1 passed / 1 failed. The exact captured semantic
  document returned `Evaluation could not be confirmed. Do not repeat it; query
  the space first. (HTTP 400)` instead of the expected pre-dispatch rejection.
  Two additional runs of the compiled fixture reproduced the same mismatch.
- Final-source green: `cargo test --locked -p tonk-worker --features native-host
  --test agent_link -- --nocapture` passed both tests in 8.03 seconds. This checks
  unchanged revision/no recovery after rejection, no preflight commit before the
  corrected write, independent corrected publication, retained-state recovery,
  and revoked-push uncertainty. A further compiled-fixture run also passed.
- `cargo fmt --all --check` and `git diff --check` passed.
- Existing valid-write elapsed milliseconds in the same disposable fixture:
  baseline 174.351, 143.590, 145.883 (median 145.883); preflight 150.718, 152.950,
  145.422 (median 150.718). The observed median difference is +4.835 ms. These
  three samples per condition overlap and are descriptive local measurements,
  not a stable estimate of hosted latency or capacity. The deterministic added
  work is one extra read-only evaluation of each write document.

## Existing read-only contract

`router/evaluate.rs` returns from the interactive dry-run path before
`stage_and_publish`, with zero committed claims and `None` for transients. Its
HTTP handler invokes command dispatch only when returned transients are `Some`.
`tonk-evaluator/src/evaluate.rs` accumulates mutation and command facts in the
transaction; rule induction and durable publication happen at commit. The
existing `it_returns_transients_for_command_dispatch` regression explicitly
requires a dry run to return no transient dispatch. Preflight can perform normal
query reads and cache hydration, but cannot publish the submitted document or
hand its commands to the system/foreign command dispatcher.

Hosted behavior, deployment, and a new frozen trial need separate evidence.
The failed v4 trial and retained evaluation state remain untouched.
