# Journal: concurrent CLI writers

## Reflective inquiry

|  | Understanding | Activity |
|---|---|---|
| **Status** | Reproduced without an upstream: H1 and H2 fail evals from N = 2, 81% at N = 16; no reported success is lost; H4 not seen; H5 (an extra head publish under contention, revisions missing from the checkout index) found. A cold eval on an empty space is 31 ms. | Directing: options built as switches and being measured. |
| **Agenda** | Whether G is kept, and if so how it reports conflicts. Whether age priority (H) is worth a column. Whether the per-branch lock is built before the owner process or only inside it. Whether G's silent conflicts and visibility lag are acceptable to agents. Whether agents reach saturation. Cold start on a realistic space. | Diagnosing: run [research/concurrent-cli-writes](../../research/concurrent-cli-writes.md). |

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

## 2026-11-24, Diagnosing: scope, and Chronicle as a second backend

**Asked.** Scope? Irakli: the local problem now; the remote one later and
separately, likely with work on the remote target itself. Irakli: measure
the same races on dialog-chronicle, which handles writers differently from
the block store and may argue against a daemon.

**Learned.** At tonk's pinned dialog, Chronicle waits on its file's turn
lock and reports a moved head as a version mismatch. Tonk can run on it by
swapping the one storage type (`experiment/chronicle-space`, a worktree
branch kept out of this one).

**Decided.** Scope: writers on one machine, one replica. The remote head
(H3, and BUG-350's swarm sync) is out of this topic.

## 2026-11-24, Diagnosing: sync apart from writes

**Asked.** Irakli: a separate process (not necessarily a daemon) could
reconcile local changes with the remote, so not every local write is
pushed at once and local writes do not wait on the remote.

**Learned.** Today every committing eval pulls and pushes around its
write, so sync sits on the local write path; taking it off is within the
local scope even though the remote's contention is not. The syncer still
writes the local head when it adopts remote changes, so it is one more
local writer and needs whatever local writers get. tonk-agent-peer's owner
already syncs on its own every 5 s.

**Decided.** Not decided. Candidate for the direction matrix: sync off the
write path, in a process of its own.

## 2026-11-24, Diagnosing: Chronicle on the same races

**Asked.** Does Chronicle, which waits on its file's lock, change the
races?

**Learned.** As predicted: no lock refusals and no missed checkout index
(H2 and H5's symptom gone), version mismatches as before (H1), so failures
at N = 16 fall from 85% to 67% and no further. Starvation under retry and
cold start on an empty space are the same on both. Chronicle answers the
refusal, not the race.

**Decided.** The local causes are now: (1) eval treats a lost race as a
failure, deliberately, so its response describes what it committed; (2)
the fs lock refuses instead of waiting (gone on Chronicle); (3) re-running
a long eval against frequent writers starves it; (4) every command starts
cold, lengthening each eval's window. Next: state the problem.

## 2026-11-25, Directing: try the options

**Asked.** State the problem? Irakli: go ahead and try various options.

**Learned.** Nothing yet; the runs are going.

**Decided.** The agent drafts the problem statement, marked as its own, so
the matrix has something to serve. Options B-E are built as one switch,
`TONK_EVAL_POLICY`, on an experiment branch; F (an owner process) is a
column without a build.

**Backtracked.** None; Delimiting was skipped on the maintainer's call,
and the statement is a draft until he takes it or rewrites it.

## 2026-11-25, Delimiting: the problem and the objective

**Asked.** Who writes the problem statement? Irakli: the agent does. The
objective: local errors to zero.

**Learned.** Zero is a different bar from fewer. A retry with a cap fails
once the cap runs out, which happened at saturation (64 of 64 lost), so
a capped retry cannot meet it on its own; an option meets it only if a
write that keeps losing eventually stops competing.

**Decided.** The problem statement in the topic stands, without the draft
marker. Objective: no local write fails because another overlapped it.
Not yet said: how long a write may wait instead of failing.

## 2026-11-25, Directing: the options measured

**Asked.** Which options reach zero local failures?

**Learned.** [Results](../../research/concurrent-cli-writes/results/2026-11-25.md).
Retry and backoff reach zero for small writes, but spend all 64 tries on a
long eval: they fail the objective. Queue and hybrid reach zero in every
run; queue was as fast or faster everywhere and never retried. A commit in
a process that stays open takes ~8 ms against ~36 ms cold, which bears
out the browser's warm-cache numbers.

**Decided.** Not decided by the maintainer. The matrix points at D, a
local queue, with F (an owner process) as the separate answer to
latency, built on D's ordering rather than instead of it. Open: the
writers that bypass D's lock.

## 2026-11-25, Directing: Irakli's direction

**Asked.** Take the queue? Must every writer go through it?

**Learned.** Irakli's answers, as given:

- Every local writer goes through the same queue, sync included.
- The queue is per branch of a repository, not one lock for everything.
- The queue should also back off.
- The owner process (F) is the right approach: it can schedule sync for
  when local writers recede, and interleave sync with them when they do
  not recede for long.
- Sync fetches without holding the queue, and rebases outside it too, so
  the lock is held only to swap the root.
- An alternative to assess by experiment (G): each writer writes to a
  branch of its own, so writers never meet, and a reconciler keeps merging
  those branches, so no writer waits. More unknowns.

Checked against the ledger:

- A sync that rebases outside the lock and swaps under it is an
  optimistic writer: if a local write lands between the rebase and the
  swap, the rebase is redone. Against busy local writers it can lose
  every time, which is the starvation measured on 2026-11-24.
  "Interleave if they do not recede for long" is the escape: after a
  bound, the sync rebases under the lock. That is hybrid (E) for the
  syncer, which measured well for one loss then queued.
- Backing off inside a queue needs a meaning. A writer blocked on a lock
  waits in line; one that polls the lock with backoff can lose it to
  newcomers every time, which is starvation again. Asked.

**Decided.** By Irakli: one queue per branch for every local writer;
direction F, an owner process that schedules sync around local writes and
holds the lock only for the root swap. G stays a column to measure.

## 2026-11-25, Directing: writer branches measured

**Asked.** Does G, a branch per writer and a reconciler, reach zero
without waiting?

**Learned.** It reaches zero, nothing went missing from main, and its
writers were a little faster than the queue's under load (p95 188 against
259 ms at 16). The long eval landed every time, slower than under the
queue (522-726 against 270-300 ms). The price is elsewhere: a write is on
main a reconcile pass later, and a pass over 16 branches took 282 ms;
concurrent writes to a one-valued field are both kept with no conflict
reported, so an agent cannot tell its write was contested; merges run no
rules; the shared checkout index collides across branches. The writers'
p50 is no better than the queue's: each eval still pays the cold start,
and its pull of main.

**Decided.** Not decided. G meets the objective and moves the cost from
waiting to visibility and conflict reporting.

## 2026-11-25, Directing: backoff, and two tradeoffs

**Asked.** What backoff means in a queue. Irakli: a pause after a queued
write loses to a writer outside the queue, and spacing sync while local
writers are busy; not polling the lock. Irakli: the queue and writer
branches are different tradeoffs. Did G set upstreams and push? How does
Dolt do this; what other prior art is there?

**Learned.** G used one-shot pulls both ways, no upstreams, no pushes; a
one-shot pull keeps a sync base, so each merge was incremental.

**Decided.** Backoff as above.

## 2026-11-25, Directing: prior art

**Learned.** [prior-art.md](prior-art.md). Dolt matches direction F
closely: a server the CLI finds through a file, a commit lock per branch,
merges computed outside the lock; but it fails a commit after 5 retries.
Dolt and jj both keep writer branches or heads apart and report conflicts
as data, which the G prototype does not. A third remedy for starvation
exists that was not measured: priority by age kept across retries
(Clojure, Greedy).

**Decided.** Not decided.
