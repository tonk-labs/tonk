# Journal: concurrent CLI writers

## Reflective inquiry

|  | Understanding | Activity |
|---|---|---|
| **Status** | Reproduced without an upstream: H1 and H2 fail evals from N = 2, 81% at N = 16; no reported success is lost; H4 not seen; H5 (an extra head publish under contention, revisions missing from the checkout index) found. A cold eval on an empty space is 31 ms. | Diagnosing. No report beyond BUG-350 exists; Irakli believes the agents share one machine. |
| **Agenda** | Scope: one machine's writers only, or the remote head too. Whether agents reach saturation. H3 with an upstream. Cold start on a realistic space. | Diagnosing: run [research/concurrent-cli-writes](../../research/concurrent-cli-writes.md). |

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

## 2026-11-24, Diagnosing: the first reproduction

**Asked.** Which of H1-H4 occur on one replica with no upstream?

**Learned.** H1 and H2 both occur from two writers on
([results](../../research/concurrent-cli-writes/results/2026-11-24.md)).
Failures are refusals, not losses. Nothing fails before the commit (H4).
Something unexplained does: under contention a process publishes a head
before its own commit, and some committed revisions are not retained for
checkout (H5, which may bear on BUG-350's "a revision cannot be restored").
A cold eval of an empty space takes 31 ms, so the daemon's cold-start
argument has no evidence yet; it would need a realistic space to make it.

**Decided.** The local cause is that a lost race is a failure the agent
sees, whether lost at the version check or at the lock. Not decided: what
the eval should do instead, which turns on what its write means.

## 2026-11-24, Diagnosing: is 31 ms a lot, and does retry starve

**Asked.** Irakli: 31 ms is a lot. Compared to what? Not yet answered.
Irakli: a daemon serializes, but under retry a writer that commits more
may lose every time.

**Learned.** Of a cold eval's ~25 ms, ~4 ms is the process start that a
daemon's client still pays. The starvation hypothesis holds at saturation:
retried, a 220 ms eval never landed in 64 attempts against 8 back-to-back
writers; queued it landed first time. I had said the two meet "the same
lost race"; that was wrong, a queue guarantees progress and retry does
not. The queue that did it is a `flock` around the CLI: serializing does
not need a daemon. Under a queue on one machine no write loses a race, so
the apply-or-rerun question only remains for the remote head.

**Decided.** Not decided. Open: whether agents reach saturation; which
budget 25 ms is measured against; whether the remote head is in scope.

## 2026-11-24, Diagnosing: warm caches are wanted regardless

**Asked.** Irakli, on being asked what 31 ms is compared to: in the browser a
commit is 20-30 ms while the service worker is cold and under 5 ms once
its caches are warm; writes usually hit the same subtree; the CLI should
get the same. Whether that is how this issue is addressed is a separate
question.

**Learned.** The comparison was there all along: a warm commit. I had
argued cold start lies outside the race window; that is only half true,
since cold caches slow the evaluation inside the window as well, so warm
caches cut lost races, starvation and queue waits too. Re-litigating
whether warm caches help was not useful; the evidence for it is the
browser's.

**Decided.** Keeping caches warm across CLI commands is an objective of its
own (Irakli). This topic still decides how concurrent writes are handled;
warm caches are one input to it, not the answer to it by default.
