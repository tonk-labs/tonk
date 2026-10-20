# Reject malformed hosted notation before evaluation

The efficient v3 trial stopped after wave 2 because its observer received
`recovery.required=true`. Participant 02 first failed on `RES-SRC-D01` at
2026-10-20 11:06:43 UTC with the native evaluation route's HTTP 400. Its unquoted
`sources` scalar contained `scope boundary: ...`. Running the captured document
as a dry run against a new disposable local space reproduced the parser error
at line 6, column 53: mapping values are not allowed in this context.

The agent-link adapter persisted `pending-evaluation` before requesting the
ordinary evaluate route, so even syntax that could never be dispatched became
an unresolved write. The observer correctly refused to certify a snapshot of
retained state. The trial and its outstanding evaluation state remain untouched.
This failure is distinct from the client-disconnect lifecycle repair in PR 1127.

## Narrow repair

Parse the complete document with the same `tonk_notation::parse` used by the
ordinary evaluate route before opening replica storage or creating the journal.
Return only a fixed rejection and diagnostic line/column, without echoing the
submitted document or parser message. Schema analysis and evaluation still run
through the ordinary route. Failures after dispatch retain their recovery
journal; HTTP 400 alone is never permission to clear it or replay notation.

## Validation

- Reproduce the exact captured document in a disposable local dry run.
- Regression: the malformed scalar is rejected before replica/journal creation,
  and a corrected valid write remains possible for the same grant.
- Run existing agent-link tests covering retained-state recovery and revocation.
- Run Rust formatting and diff checks after the final edit.
- Hosted preview, deployment, and a new frozen trial require separate evidence.

## Baseline execution evidence

The pre-guard single-document regression compiled and ran against the old adapter.
It failed at the intended assertion after the adapter had opened/populated its
replica and attempted a remote pull. Its inspected temporary replica was removed.
The same no-storage regression now lives in the smaller integration target,
including a second malformed document after a valid first document.

```text
cargo test -p tonk-worker --features native-host --lib malformed_evaluation_is_rejected_before_replica_or_journal_creation -- --nocapture
Finished test profile [optimized] target(s) in 10m 37s
assertion left == right failed
left:  "Space pull failed; no evaluation was dispatched. (HTTP 503; SYNC_UNAVAILABLE)"
right: "Invalid notation (line 3, column 53); no evaluation was dispatched."
Test process then aborted while initiating its assertion panic (SIGABRT).
```
