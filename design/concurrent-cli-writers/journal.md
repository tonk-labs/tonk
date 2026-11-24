# Journal: concurrent CLI writers

## Reflective inquiry

|  | Understanding | Activity |
|---|---|---|
| **Status** | What the CLI does on a concurrent write is read from the code (see claims). Four separate places a write can fail are visible: eval's one-try commit, the lock file's immediate refusal, the pull-before moving the head, and the shared remote head. Nothing has been reproduced. | Diagnosing. No report beyond BUG-350 exists; Irakli believes the agents share one machine. |
| **Agenda** | Which of H1-H4 (below) produce the failures, at what concurrency, and whether any write is lost rather than refused. Whether the cold-start cost is a problem of its own or part of this one. | Diagnosing: run [research/concurrent-cli-writes](../../research/concurrent-cli-writes.md). |

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

## 2026-11-24, Diagnosing: no more evidence, so reproduce it

**Asked.** What did Gavin's agents see, and on one machine or several?
Irakli: nothing beyond the bugs already found; he believes one machine.

**Learned.** The symptom has to come from our own reproduction. "One
machine" (an opinion, in the ledger) narrows the field but does not remove
the remote: every committing eval still pulls and pushes, and other
members push to the same upstream. Hypotheses, written before any is
chased:

- **H1, one-try commit.** Another process commits between an eval opening
  and publishing; the eval fails with `Version mismatch`, exit 4.
- **H2, lock refusal.** Two publishes overlap on the head cell's lock file;
  the second fails at once with a storage error, exit 4.
- **H3, pull moves the head.** One process's pull-before or push-retry
  moves the local head under another's open eval (H1 by another path; needs
  an upstream).
- **H4, contention at open.** Start-up work that writes (repository
  upgrade, per-branch migration, registry, system key) fails or races
  before the eval's own transaction is reached.

Result template, filled by the research topic: for N concurrent evals
(N = 2, 4, 8, 16), each asserting a distinct fact, with `--no-sync` and
with a local upstream, how many succeed, how many fail by which error
(version mismatch, lock, other), whether every reported success is present
in the final head (no lost write), and the wall time of a cold eval. If
that table were filled we would know which of H1-H4 occur and how often.

**Decided.** Diagnose by experiment before weighing remedies. Phase moves
to Diagnosing.
