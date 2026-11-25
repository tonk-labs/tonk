# Direction: concurrent CLI writers

Decision: how an eval on one machine handles another writer moving the
head while it runs. For: the problem statement in
[the topic](../concurrent-cli-writers.md); objective: zero local failures.

Scope: writers on one machine and one replica. The remote head is out
(journal, 2026-11-24). Storage (a file per block or Chronicle) is a
second axis: Chronicle removes lock refusals under every column and
changes nothing else measured.

Columns B to E are switches of one experimental build,
`TONK_EVAL_POLICY` ([patch](../../research/concurrent-cli-writes/harness/eval-policies.patch));
F is not built.

| criterion \ approach | A. Status quo: eval tries once | B. Retry: eval evaluates again over the new head at once, up to 64 tries | C. Backoff: as B, after a jittered pause doubling from 2 to 128 ms | D. Queue: every eval holds a machine-wide lock on the space from read to publish | E. Hybrid: evaluate without the lock, publish under it; after one loss, queued | F. Owner process: one process keeps the space open, takes every write in order through a queue per branch, and syncs when local writers recede (fetch and rebase outside the queue, swap the root under it) | G. Writer branches: each writer commits to a branch of its own; a reconciler merges them into main |
|---|---|---|---|---|---|---|---|
| a write lands without the agent trying again (the objective: zero failures) | 🔴 50-83% of evals fail at 2-16 writers ([results](../../research/concurrent-cli-writes/results/2026-11-25.md)) | none of 90 small evals failed, taking up to 17 tries | none failed, up to 8 tries | 🟢 none failed, each on its first try | none failed, at most 2 tries | ? not built; it would order writes as D does | 🟢 none of 90 failed, none missing from main ([results](../../research/concurrent-cli-writes/results/2026-11-25.md), "Writer branches") |
| a long write lands against frequent short ones | 🔴 fails | 🔴 a 500-note eval spent its 64 tries in 3 of 3 runs, ~32 s | 🔴 the same, ~23 s | 🟢 landed first try in 3 of 3, ~275 ms | landed in 3 of 3, 550-700 ms, after losing once | ? as D | landed first try in 3 of 3, 522-726 ms (queue 270-300 ms in the same sitting) |
| time a small write takes, alone and with 16 writers | ~36 ms alone; p50 125 ms at 16 (most failing) | p50 257, p95 336 ms at 16 | p50 198, p95 368 ms at 16 | p50 194, p95 293 ms at 16 | p50 194, p95 287 ms at 16 | 🟢 a commit in a process that stays open: ~8 ms after the first (measured), against ~36 ms in its own | p50 164, p95 188 ms at 16 (queue 178 / 259 in the same sitting); 🟡 reaching main takes a reconcile pass more, 282 ms p50 at 16 branches |
| what the response says | what was matched and committed, or a failure | the last try's matches, which may differ from the first's | as B | the matches at the head it committed on | as D | as D | 🟡 what it matched on its branch; main may hold a write it did not see, and a concurrent write to the same one-valued field is kept beside it with no conflict reported |
| writers that do not take the lock (sync, `Branch::transact` users, other tools) | meet the head as today | meet the head as today; B retries their wins | as B | 🟡 not ordered by the lock; can still move the head, which a queued eval then loses once | 🟡 as D | ordered only if they go through the owner | no lock; only the reconciler writes main, so sync must go through it |
| a writer that crashes holding the lock | no lock | no lock | no lock | the kernel releases an `flock` on exit | as D | 🟡 a new owner must take over (tonk-agent-peer does, on the next call) | no lock; its branch is left for the reconciler, or orphaned |
| needs a process that outlives the command | no | no | no | no | no | 🟡 yes: start, discovery, idle exit, upgrade | 🟡 yes: the reconciler |
| caches warm across commands | no | no | no | no | no | 🟢 yes | no, unless the writers are also in a process that stays open |
| cost to build | none | 🟢 a loop in eval | 🟢 B and a pause | 🟢 a lock file in eval | 🟢 D and B | 🟡 lifecycle (sapling's modules, or tonk-agent-peer's owner) and a forwarding protocol | 🟡 branch per writer (naming, cleanup), a reconciler, induction after merge, the shared checkout index |

Rows added from the runs:

- Only D and E meet the objective; D is as fast or faster than E on every
  measured row, with no retries.
- D and F are not alternatives: an owner process would take writes in
  order, which is D inside one process, and add warm caches. D can be
  built first; F answers the latency objective, separately.
- 🟡 D's gap is the writers that do not take its lock.

Decided 2026-11-25 (Irakli): direction F, with every local writer, sync
included, in one queue per branch. G is measured before it is ruled in or
out. See the [journal](journal.md).
