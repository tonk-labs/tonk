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

| criterion \ approach | A. Status quo: eval tries once | B. Retry: eval evaluates again over the new head at once, up to 64 tries | C. Backoff: as B, after a jittered pause doubling from 2 to 128 ms | D. Queue: every eval holds a machine-wide lock on the space from read to publish | E. Hybrid: evaluate without the lock, publish under it; after one loss, queued | F. Owner process: one process keeps the space open, takes every write in order, syncs on its own |
|---|---|---|---|---|---|---|
| a write lands without the agent trying again (the objective: zero failures) | 🔴 15-85% of evals fail at 2-16 writers | ? | ? | ? | ? | ? not built |
| a long write lands against frequent short ones | 🔴 fails | ? | ? | ? | ? | ? |
| time a small write takes, alone and with 16 writers | 25-32 ms alone; ~100 ms at 16 | ? | ? | ? | ? | ? warm, under 5 ms in the browser (reported) |
| what the response says | what was matched and committed, or a failure | the last try's matches, which may differ from the first's | as B | the matches at the head it committed on | as D | as D |
| writers that do not take the lock (sync, `Branch::transact` users, other tools) | meet the head as today | meet the head as today; B retries their wins | as B | 🟡 not ordered by the lock; can still move the head, which a queued eval then loses once | 🟡 as D | ordered only if they go through the owner |
| a writer that crashes holding the lock | no lock | no lock | no lock | the kernel releases an `flock` on exit | as D | 🟡 a new owner must take over (tonk-agent-peer does, on the next call) |
| needs a process that outlives the command | no | no | no | no | no | 🟡 yes: start, discovery, idle exit, upgrade |
| caches warm across commands | no | no | no | no | no | 🟢 yes |
| cost to build | none | 🟢 a loop in eval | 🟢 B and a pause | 🟢 a lock file in eval | 🟢 D and B | 🟡 lifecycle (sapling's modules, or tonk-agent-peer's owner) and a forwarding protocol |
