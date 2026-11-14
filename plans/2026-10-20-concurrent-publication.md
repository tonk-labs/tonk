# Concurrent publication

## Evidence and scope

The frozen v8 participant 02 first valid research write committed locally but
returned HTTP 409 `SYNC_CONFLICT` from publication. Other participants were
publishing concurrently. The existing push route already pulls and merges after
typed upstream conflicts, bounded to four attempts. Native attempt logs are not
available, so the exact exhausted live boundary is unconfirmed.

Preserve v8, old spaces, and all unrelated work. Use disposable local services
and synthetic authority. Never repeat an uncertain evaluation.

## Checkpoints

1. Exercise actual AgentLinkWorker publication against two independent remote
   writers with a deterministic barrier at the conditional head write. Establish
   the current four-attempt failure before changing product behavior.
2. Fix only the demonstrated publication boundary, retaining exactly one local
   evaluation, bounded work, and pending state on unconfirmed outcomes.
3. Independently verify both writers' artifacts, same-entity convergence, burst
   behavior, and controls for unavailable or revoked publication. Run the focused
   regression and existing agent-link safety checks after the final change.

## Baseline

The actual native worker regression on unchanged product source failed after
four controlled conditional remote publication refusals, with three successful
pull/merge retries between them. It returned the exact v8 fixed error:
`Applied locally; remote push could not be confirmed. Do not repeat the write.
(HTTP 409; SYNC_CONFLICT)`. Test runtime 1.83 seconds after the build.
Retained log: `/private/tmp/tonk-publication-controlled-red.log` (0600).

This reproduces a concrete count-budget exhaustion mechanism, not the exact
unobserved native attempt sequence of frozen v8. Earlier fixture iterations
waited at an incorrect permit/object boundary; native traffic performs direct
UCAN operations, and the corrected barrier intercepts its conditional cell POST.

Next increment: give typed upstream-publication conflicts a bounded elapsed
retry window with capped jittered backoff. Evaluate remains outside this loop;
unknown/revoked outcomes still return immediately and preserve the journal.

## Implementation and verification scope

The shared push route now admits typed upstream-conflict retries for 30 seconds,
with capped jittered backoff. Reconciliation and its in-flight I/O can finish
after that window; no new conditional publication is admitted after it. This
is a retry-admission bound, not a cancellation deadline for an uncertain write.
Logs distinguish exhaustion before reconciliation from exhaustion after it,
and separate reconciliation failure from an ordinary failed push. Existing
evaluation logs now identify `transact=true/false` without submitted notation.

The exact extracted final production retry policy and unchanged unit assertion
passed a standalone native Rust test: one test, zero failures. This verifies
backoff caps and the post-reconciliation admission predicate without sleeping;
it does not stand in for network or full Cargo integration evidence.

Focused integration cases exercise actual native
worker processes and disposable independent remote writers: a finite burst
past four refusals, same-artifact convergence, sustained contention, refusal
before publication, and response loss after publication. Committing evaluation
counts come from actual `transact=true` route diagnostics, not call counts.

- Existing agent-link integration: 3 passed, 0 failed, 7.41 seconds, including
  real MCP clients, semantic rejection, revoked authority, and retained state.
- New publication integration: 5 passed, 0 failed, 37.60 seconds. Real conditional
  refusals were 4 for the finite burst, 1 for the same-artifact race, 3 before a
  successful publication whose response was lost, and 31 during the sustained
  conflict window. Unavailable publication made no conditional remote write.
- Logs: `/private/tmp/tonk-publication-green-final.log` and
  `/private/tmp/tonk-publication-cases-final.log`, both private.
- All fixture notation also passed an already-built native-host spike covering
  both schemas, four race write shapes, shared/disjoint reliability writes,
  and queries. Earlier new-fixture setup failures used unsupported `optional`
  notation and omitted required descriptions; they were not publication tests.

## Separate reliability receipt workload

The ignored test `reliability::emit_native_reliability_receipts` consumes a
registered JSON plan via `TONK_RELIABILITY_PLAN` and writes to a new directory
named by `TONK_RELIABILITY_OUTPUT`. It uses two independent native writer roots
and grant identities. Each writer has an owner queue. Barrier admission times
include queue latency; actual native execution intervals are also measured.
The burst schedules arrivals from the first barrier, independently of completion.

Fresh native readers verify each operation and both final observers; the two
same-artifact observers have distinct signer identities and storage roots. Raw
logs, source state, binary digest, barriers, and inventory accompany plan-bound
receipts with content hashes. No model session or live/frozen space is involved.
This workload is separate from model coordination quality.

Native attempt 02 completed in 10.28 seconds and emitted 30 actual receipts:
4 independent writes, 2 same-artifact writes, and 24 sustained arrivals. All were
independently read back, each had exactly one observed committing evaluation,
publication used at most two attempts, and none required recovery. Independent
scoring and raw-evidence review passed: 30/30 complete, zero replay, duplicate
records, or lost independent fields; both same-artifact observers converged.
Publication attempt totals were 5, 3, and 29 for the three cases. Evidence directory:
`/private/tmp/tonk-publication-reliability-v1-native-02`.
The directory includes `score.json` and `independent-review.json` alongside the
raw logs, source patch, binary digest, observations, and retained synthetic state.

Attempt 01 is retained separately. Its four independent operations completed,
but provisioning the next case hit the local helper's single-customer email
constraint. Attempt 02 uses a fresh local service per registered case. No
uncertain operation, old disposable scope, or frozen v8 state was replayed.
