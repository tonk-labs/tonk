# Journal: concurrent CLI writers

## Reflective inquiry

|  | Understanding | Activity |
|---|---|---|
| **Status** | What the CLI does on a concurrent write is read from the code (see claims). Four separate places a write can fail are visible: eval's one-try commit, the lock file's immediate refusal, the pull-before moving the head, and the shared remote head. Nothing has been reproduced. | Describing. |
| **Agenda** | What Gavin's agents actually saw: the errors, and whether they share one machine and one replica. Whether the cold-start cost is a problem of its own or part of this one. | Get Gavin's observations; reproduce N concurrent evals on one replica; then diagnose. |

## 2026-11-24, Describing: what is reported and what the code does

**Asked.** Irakli brought the report and three remedies: a shared daemon,
serialized writes, optimistic retry.

**Learned.** The three remedies answer at least two different problems: the
daemon is argued from cold start (a cost), serializing and retrying from
failed writes (a correctness and liveness issue). The failed writes have at
least three local causes in the code, and the cheapest is that `tonk eval`
does not use the retrying transaction dialog already offers. BUG-350 is a
second report of the same symptom, from another project. Tonk already runs a
shared owner process for MCP servers.

**Decided.** Not decided. Next: what was observed, before any remedy is
weighed.
