# Claims: concurrent CLI writers

Line numbers: tonk at `c0ec36e`, dialog at `6955165` (tonk pins `875cc7f`,
an ancestor, so lines may differ slightly).

| claim | kind | backing | status |
|---|---|---|---|
| Every `tonk` invocation opens its own handle on the replica under `spaces/<name>/`, on the per-file fs provider | fact | `rust/tonk-cli/src/site.rs:27,168,200-260`; `space.rs:359-399` | holds |
| Nothing the CLI derives at start (profile, repository upgrade, reactor, per-branch migration, node cache) survives the process | fact | `site.rs:158-164,385-454`; dialog `repository/caches.rs:1-60` | holds |
| A cold start costs enough per command to matter | hypothesis | on an empty space a whole cold eval takes a median 31 ms (10 runs), which argues against; still open for a space of realistic size (rules, a deep tree) | open, weakened |
| `tonk eval` commits with the deprecated one-shot `commit().publish()`, so a head moved since it opened fails it with `Version mismatch`, exit 4 | fact | `rust/tonk-cli/src/eval.rs:154-155,198-205,255-267`; dialog `branch/transaction/batch.rs:592-628` | holds |
| Most other CLI writers use `Branch::transact`, which runs the body again on a lost race, up to 64 times | fact | `invite.rs:511`, `remote.rs:198`, `space.rs:1139` and others; dialog `branch/transaction/transact.rs:162-186`, `commit.rs:36` | holds |
| dialog's `Stack::transact` runs the caller's closure again on a lost race, up to 8 times, with no backoff | fact | dialog `stack/invoke.rs:33,188,452-498`; test `stack.rs:5589` | holds |
| A held head-cell lock file is refused at once, as `MemoryError::Storage`, not `VersionMismatch`, so neither retry loop retries it | fact | dialog `storage/provider/fs/native.rs:599-650` (`CLAIM_ATTEMPTS = 2`), `fs/memory.rs:34-37,113-160` | holds |
| BUG-350's `revision.lock` failures are that refusal | fact | reproduced: `…/memory/branch/main/revision.lock held`, exit 4, 17 of 64 failures ([results](../../research/concurrent-cli-writes/results/2026-11-24.md)) | confirmed |
| A committing eval pulls before and pushes after; the pull does not retry, the push retries a non-fast-forward up to 5 times, and failures of either are warnings | fact | `rust/tonk-cli/src/auto_sync.rs:1-27,245-276`; `sync.rs:263-311`; dialog `branch/pull.rs:45-56,463-464` | holds |
| One process's pull-before moving the local head fails another process's open eval | hypothesis | follows from the two facts above; confirmed by a two-process run with an upstream and the error each process reports | open |
| No dialog test runs two processes against one fs-provider branch; the single-file archive (opt-in, native) has such tests | fact | dialog `dialog-chronicle/tests/repository.rs:849,905`; no fs-provider equivalent found | holds |
| tonk already has a shared owner process for MCP servers: one takes a `flock` on `peer.lock`, keeps the worker open, syncs every 5 s, serves the others over `peer.sock`, and hands over on exit | fact | `rust/tonk-agent-peer/src/owner.rs:1-30`; `tonk-worker/src/router/local_peer.rs:461-480` | holds |
| Sapling's daemon both serializes (one commit turn per space) and retries a lost compare-and-swap (up to 8) | fact | sapling `rust/sapling-kernel/src/subject.rs:87,113`; test `rust/sapling/tests/transact.rs:319` | holds |
| Sapling's process-lifecycle modules (auto-start, instance lock, runtime dir, leases, idle exit) are not tied to its kernel model; its worker handover is | hypothesis | read of sapling `rust/sapling/src/commands/daemon/`; they live in a binary crate, so tonk would copy, not depend | open |
| What Gavin's agents saw is the same as BUG-350's | hypothesis | no further report exists (Irakli, 2026-11-24); settled only by reproducing it | open, research/concurrent-cli-writes |
| Gavin's agents run on one machine, sharing one replica directory | opinion | Irakli, 2026-11-24 ("I believe") | - |
| Agents on one machine still meet at the remote head: a committing eval pulls and pushes unless `--no-sync`, and other members' devices push to the same upstream | fact | `auto_sync.rs:1-27,245-276` | holds |
| On one replica with no upstream, concurrent evals fail from N = 2: 50, 67, 58, 81% at N = 2, 4, 8, 16 | fact | [results/2026-11-24](../../research/concurrent-cli-writes/results/2026-11-24.md) | holds |
| A concurrent eval that exits 0 has its write in the final head (no lost write) | fact | same results, 0 of 25 successes missing | holds, for this workload |
| Start-up work fails a concurrent eval before its commit (H4) | hypothesis | not seen: every failure was the commit's | refuted for this workload |
| Under contention some eval processes publish a branch head before their own commit, and a committed revision can miss the checkout index (H5) | hypothesis | 16 failed evals printed a retain warning, which follows only a successful head publish (`account_authority.rs:343-372`); 6 successful evals' revisions were not retained; sequentially one eval publishes one head | open |
