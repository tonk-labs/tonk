# MCP observability

Measure the hosted MCP path end to end, so that its reliability (how often
each step succeeds) and its performance (how long each step takes) are
numbers, not impressions. The path starts when a client connects and a
person signs up or signs in through OAuth. It continues through the choice
of spaces and access on `/settings/agent`, the agent's reads and writes,
and the person seeing those writes. It ends with the person revoking the
agent's access.

This plan extends `plan/perf-gates.md`. That plan measures the web app:
guest boots, content removals and the browser worker's state lock
(`tonk_common::diagnostics`, #1242). None of its counters is on the MCP
path. This plan uses the same rules: gate on numbers that repeat, chart
time, measure before fixing.

## What exists today

| Stage | Code | Measured today | Missing |
| --- | --- | --- | --- |
| OAuth: metadata, `register`, `authorize`, `token` | `rust/tonk-access-service/src/oauth.rs`, `handlers/oauth.rs`, routed at `lib.rs:110` | Nothing. `AccessOperation` (`observability.rs:8`) has no OAuth operation | Funnel counts, failure kinds, client identity |
| Approval: sign-in, space and access choice, grants | `rust/tonk-worker/src/router/agent_session.rs`, page in `rust/tonk-core/assets/library/profile.yaml` | Step times as `log!` lines (`agent_session.rs:430-557`), in the browser console only. The passkey step emits `account_event` `authorize_agent` | Step times that leave the browser. `ProductAction::ConnectAgent` (`rust/tonk-analytics/src/product.rs:45`) is defined but nothing emits it |
| `/mcp` front: session, `tools/call` | `rust/tonk-worker/src/router/hosted/mcp.rs` (`serve` :265, `call_tool` :419, `access` :565) | Nothing. A failed tool returns `isError` (:825) and nothing is logged | Per-tool time and outcome. The `access()` round trip to the agent space's writer, which every non-home call makes, is not timed |
| Space writer Durable Object: pull, evaluate, push | `rust/tonk-host-worker/src/writer.rs`, `rust/tonk-worker/src/router/hosted/writer.rs` | Free-text step lines "space writer: X took N ms, now Y" (`writer.rs:258`), a 45 s `DEADLINE` that names the stuck step (:52), memory and recycle lines. `evaluate timing` line (`router/evaluate.rs:1006`) | Fields to aggregate on (space, tool, trace). Cold or warm open. No diagnostics read-out |
| Access service `/ucan/` pull and push | `rust/tonk-access-service/src/handlers/ucan.rs` | `Server-Timing` (:617) and a log line per request. `AccessFailureLog` on refusals. Metering records | The writer drops `Server-Timing`, so the MCP side cannot attribute its time |
| Write reaches the person | `ucan.rs:588` calls `live::changed`; `live.rs` fans out to sockets; the browser pulls and re-runs subscriptions | Fan-out count and time per change in `live.rs`. Browser `sync.rs` console lines | Time from the agent's write to the person's view. Nothing joins the two sides |
| Join from a link in chat | `rust/tonk-worker/src/router/hosted/join.rs`, `SpaceWriter::join_link` | Only the writer's `link` step line | Outcome and time per join |
| Revocation | `rust/tonk-worker/src/router/agents.rs:304` calls `/ucan/revoke` (`handlers/revoke.rs`), which writes the KV index and calls `live::revoked` | `log!` "access revoked" in the browser console. A refused chain is an `AccessFailureLog` `access_denied`, with no revocation kind | Time from revoke to the agent's first refused call. No test revokes an agent and checks its next call |

The end-to-end test `tests/e2e/hosted-agent/agent.test.mjs` runs on each
pull request against the preview deploy. It covers sign-up, OAuth, approval,
reads, concurrent writes, join from a link and live views. It does not cover
revocation. It prints some latencies with `console.log` and checks only
the approval upload budgets and a 30 s ceiling on live updates.

Every sink today is a free-text console line in Workers Logs or in a
browser. No request id joins one tool call across the front, the writer,
the access service and the live fan-out.

## What to measure

The kinds are the same as in `plan/perf-gates.md`. An invariant has a fixed
target. A cost has a baseline that the maintainer can move. A time is
charted and not gated on a pull request. Reliability is a success rate in
the field and pass or fail in a test.

| Question | Metric | Kind | Target |
| --- | --- | --- | --- |
| Does connecting work? | OAuth funnel: `register`, `authorize`, approval page shown, approved, denied, `token` issued; failures by kind (redirect, PKCE, expired request, expired code, client metadata fetch) | Reliability | Field rate per stage; e2e passes |
| How long is approval? | Approval steps (account open, targets checked, agent space made, grants written, pushed) | Time; uploads are a cost | Charted; upload budgets as today |
| Do tool calls work? | `tools/call` outcome by tool: ok, refused, tool error, timeout (504), transport error | Reliability | Field rate; e2e has no unexpected error |
| How long is a tool call? | Total, split into `access()` round trip, writer queue and lock wait, writer open (cold or warm), pull, evaluate, push | Time; round trips are a cost | Charted p50 and p95 |
| How many remote round trips per call? | `/ucan/` requests and bytes per tool call, by kind | Cost | Baseline, exact |
| Does an agent write reach the person? | Time from commit to the browser's view showing it | Time | Charted; 30 s ceiling in e2e |
| Does the person's view stay steady? | `display.entity.emptied` and `refilled` during an agent write | Invariant | 0 |
| Does revocation take effect? | Time from revoke to the first refused call, per grant kind (issued by this profile, from a share link) | Reliability | Pass within a budget, for example 10 s; field p95 charted |
| Do sessions stay alive? | Writer recycles, deadline hits, worker instances per hour | Cost | Baseline |

## Design

### 1. One trace id per request

The `/mcp` front makes an id for each request and sends it as a header to
the space writer, which sends it on to `/ucan/` and passes it to
`live::changed`. Each log line on the way carries it. The `/mcp` response
returns it in a header, so a client, the e2e test or a support report can
quote it. The approval page makes one id per approval and sends it with
each `/api/agent/*` call.

### 2. One event shape

Every stage writes one JSON line per event, with the same fields:

```json
{ "event": "mcp.tool", "trace": "…", "outcome": "ok", "kind": null,
  "ms": 412, "steps": { "access": 88, "lock": 0, "pull": 140, "eval": 31, "push": 150 },
  "tool": "tonk_evaluate", "space": "<short hash>", "client": "claude.ai" }
```

The lines are content-free: no notation, no space names, no email. Spaces and
agents appear as short hashes. The client is the host of its metadata
document. The event names go in one list next to `tonk_common::diagnostics::names`,
so that queries, tests and gates use the same names. Workers Logs can
already query JSON fields. Workers Analytics Engine is an option for
long-term rates (see open questions).

### 3. Each stage

- **OAuth.** Add `AccessOperation::OAuth` and emit one event per endpoint,
  with its outcome and failure kind. Count approval page loads that never
  reach `answer`.
- **Approval.** Change the `log!` steps in `agent_session.rs` into one timed
  record. Return it to the page, which sends it with `ConnectAgent` through
  `tonk-analytics` (content-free, the same as the existing account events).
- **`/mcp` front.** Time each `tools/call` and its `access()` round trip.
  Record the outcome, including tool errors that today go back only as
  `isError`. Send `Server-Timing` on the response.
- **Space writer.** Change `reach()` into step events with the trace id.
  Mark the open as cold or warm. Read `Server-Timing` from `/ucan/` and
  include its parts. Give the object a diagnostics registry
  (`tonk_common::diagnostics`) and an inspect route that the tests can read.
- **Access service.** Log the trace id with the existing `/ucan/` line. Add
  a `Revoked` failure kind, so that a refusal because of revocation can be
  told apart from other refusals.
- **Live updates.** Pass the trace id and the commit time with the change
  message. The browser records the time it pulled and the time the view
  showed the rows, through the page's diagnostics registry.
- **Revocation.** Record when `/ucan/revoke` is written, when `live::revoked`
  ends, and when the agent's first call is refused with `Revoked`.

### 4. Tests and the field

The general harness is the work of `plan/perf-gates.md`, which is under way:
the diagnostics registry and its read-outs, trace capture, the stability and
cost scenarios, the variance study, baselines, the delta report, and the
`tonk-perf` proxy and analyzer. This plan assumes that work and adds only
what is particular to the MCP routes:

- **End-to-end test.** Add a revoke step for both grant kinds: revoke, then
  call until refused, within the budget. Record each call's trace id and
  `Server-Timing` with the step times, through whatever artifact the
  baseline harness writes. Read `display.entity.*` from the person's page
  during the agent writes.
- **Gates.** The MCP's step counts and times go through the baseline's
  variance study, baselines and delta report, not a harness of their own.
  Request kinds for `/mcp`, `/oauth/*` and `/ucan/revoke` are added to
  `tonk-perf` only if the baseline's classification does not already
  separate them.
- **Canary.** A scheduled agent with its own test account connects, lists,
  reads, writes, reads back and revokes, against staging and production, a
  few times an hour. It gives a success rate and times over days, which the
  per-commit e2e test cannot.
- **Field.** Queries over the JSON lines for rates and p95 per stage, and
  an alert when the tool error rate or the deadline count goes above its
  usual level.

## Order of work

Each item is a separate pull request.

1. **Trace id and event shape** on the `/mcp` front and the space writer,
   with per-tool outcome and time. This gives the first field numbers.
2. **Access service and live updates:** trace id on `/ucan/`, `Revoked`
   failure kind, `Server-Timing` read by the writer, commit time in the
   change message.
3. **Revocation step in the e2e test**, for both grant kinds. Where today's
   code fails the budget (see the share-link note below), record today's
   value as a ratchet.
4. **OAuth and approval events**, including `ConnectAgent` with times.
5. **The MCP scenarios in the baseline gates:** the hosted-agent e2e
   steps and their step times, read through the perf-gates harness.
6. **Canary and field queries.**

Items 1 to 3 are the minimum before reliability or speed can be stated as
numbers. Items 4 to 6 turn the numbers into trends and alerts. Item 5 waits
for the perf-gates harness.

## Known gap that the measures will show

Revoking a grant that came from a share link (`agents.rs:328-353`) removes it
from the agent space and leaves the background sync to send the change. It
does not write the access service's revocation index. Item 3 will show how
long this takes. A fix for it is separate from this plan.

## Open questions

- Workers Logs only, or also Workers Analytics Engine for rates over months?
- Is the client's metadata host acceptable in logs and analytics? It is the
  only way to tell clients apart (Claude, ChatGPT, others).
- Where does the canary's account live, and who owns its alerts?
- What revocation budget is acceptable? The KV index is eventually
  consistent, so the answer sets whether the index is enough.
