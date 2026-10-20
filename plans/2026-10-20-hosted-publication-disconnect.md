# Hosted publication after client disconnect

## Observed boundaries

Pilot 4 stopped after seven Tonk sessions with unresolved publication. Its exact
container termination cause remains unknown. The pre-run hosted commit was
`e50689c877dd9417c4fa9cc2e928ca52478163db`, which is the squash merge of PR 1122:
bounded pull/merge/push after an upstream race was already present.

On 2026-10-20 the hosted build endpoint returned
`0115c2fcd89f6f15fa11a55cceb9721fd1ffa575`. GitHub ancestry comparison showed six
commits ahead and none behind the squash merge. Those commits contain no hosted
lifecycle repair. The local dirty checkout predates these commits; its existing
checkpoint relocation and diagnostic changes were preserved.

The pinned Cloudflare Containers 0.3.7 source counts in-flight HTTP requests and
renews the inactivity timer while any remain. A simple 20-minute inactivity
timeout is therefore not established as the cause of a live request disconnect.

## Executable local defect and repair

The HTTP server aborts a request when its response connection closes. Before this
repair the backend forwarded that cancellation into a dispatched native
evaluation. A test launching an actual disposable worker subprocess reproduced
`category=cancelled`, `signal=SIGTERM` and an unknown evaluation outcome. No
remote credentials, live space, or uncertain evaluation were used or replayed.

Once an evaluation is dispatched, client cancellation now leaves it running
under its existing two-minute native deadline, then checkpoints its closed
state. Queued calls and active queries remain cancellable. Explicit host force
cancellation remains available. Hosted graceful shutdown finishes its active
invocation and checkpoint and rejects queued calls instead of cancelling the
active write or draining the entire queue.

This closes a proven avoidable publication interruption. It does not prove the
original platform disconnect cause, prevent an arbitrary container kill, or
guarantee delivery of a response after its client disconnects. Callers must query
after an unknown outcome and must not repeat uncertain notation.

## Verification and deployment

The new subprocess regression failed before the change and passes afterward.
Focused controls cover query cancellation, queued evaluation cancellation,
checkpoint ordering, explicit force cancellation, and hosted graceful shutdown.
Fresh validation on isolated upstream commit
`0115c2fcd89f6f15fa11a55cceb9721fd1ffa575`: MCP tests 36/36, hosted tests 65/65,
scoped Prettier checks and `git diff --check` pass. A real MCP client read-only
probe of the old Pilot 4 grant returned recovery metadata, so no live write or
retained-state publication was attempted there. A separate fresh target query
succeeded without recovery metadata; that evaluation target was left unchanged.

The existing Deploy workflow creates an isolated MCP/provider/bucket preview
for a same-repository PR. Main push deploys agent-dev. Stage only reviewed
product hunks in a current isolated checkout, verify the preview's exact build
and a fresh disposable test target, then verify the deployed agent-dev build
before a new frozen trial. Preserve Pilot 4 and its pending grants; do not
recover or replay its uncertain writes as part of this repair.
