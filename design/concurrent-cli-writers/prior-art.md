# Prior art: concurrent writers

Gathered 2026-11-25 from primary sources; every claim links to one.
Readings that are ours, not the source's, are marked *inference*.

## Dolt

- **Processes on one directory.** Storage takes an exclusive `flock` on
  `.dolt/noms/LOCK`, waiting up to 100 ms
  ([`file_manifest.go`](https://raw.githubusercontent.com/dolthub/dolt/main/go/store/nbs/file_manifest.go)).
  A process that does not get it opens the database read-only by default,
  and its writes fail; with `FailOnLockTimeout` it gets "the database is
  locked by another dolt process"
  ([`journal.go`](https://raw.githubusercontent.com/dolthub/dolt/main/go/store/nbs/journal.go),
  [`file.go`](https://raw.githubusercontent.com/dolthub/dolt/main/go/libraries/doltcore/dbfactory/file.go)).
- **The CLI goes through a running server.** Since Dolt 1.1, `dolt sql`
  (later `add`, `status`, `blame`) connects to a running `dolt sql-server`
  instead of opening storage
  ([blog, 2023-06-12](https://www.dolthub.com/blog/2023-06-12-cli-to-sql/)).
  The server writes `.dolt/sql-server.info` (`pid:port:secret`, mode 0600);
  the CLI searches upward for it, removes it if the pid is dead, and
  connects over loopback with the secret
  ([`creds.go`](https://raw.githubusercontent.com/dolthub/dolt/main/go/cmd/dolt/commands/sqlserver/creds.go),
  [`queryist_utils.go`](https://raw.githubusercontent.com/dolthub/dolt/main/go/cmd/dolt/commands/sqlserver/queryist_utils.go)).
- **Concurrent transactions on one branch merge at commit.** COMMIT runs a
  three-way, cell-level merge of the transaction's changes against the
  branch's current working set, the transaction's starting snapshot as the
  ancestor. Different cells merge; identical writes merge; different values
  in one cell conflict and the transaction rolls back with "serialization
  failure: this transaction conflicts with a committed transaction from
  another client, try restarting transaction" (SQLSTATE 40001); the client
  retries
  ([`transactions.go`](https://raw.githubusercontent.com/dolthub/dolt/main/go/libraries/doltcore/sqle/dsess/transactions.go),
  [blog, 2023-12-14](https://dolthub.com/blog/2023-12-14-concurrent-transaction-example/),
  [blog, 2021-05-19](https://dolthub.com/blog/2021-05-19-dolt-transactions/)).
- **Writes to the commit graph are serialized, per branch, with the merge
  computed outside the lock**
  ([blog, 2026-02-17](https://www.dolthub.com/blog/2026-02-17-dolt-concurrency/)).
  Commit retries an optimistic-lock failure up to 5 times with no pause,
  then fails (`maxTxCommitRetries`, `transactions.go`).
- **A branch per agent** is what DoltHub recommends for concurrent agents;
  each branch is its own unit of transaction concurrency, and work is
  merged after review
  ([2026-02-17](https://www.dolthub.com/blog/2026-02-17-dolt-concurrency/),
  [2025-10-31](https://www.dolthub.com/blog/2025-10-31-agentic-systems-need-version-control/),
  [2025-03-17](https://www.dolthub.com/blog/2025-03-17-dolt-agentic-workflows/)).
- **Merge conflicts are data.** `DOLT_MERGE()` returns `conflicts=1`;
  `dolt_conflicts_<table>` holds `base_`, `our_`, `their_` columns per
  conflicted row, resolved by editing those rows or
  `dolt_conflicts_resolve('--ours'|'--theirs')`
  ([docs](https://www.dolthub.com/docs/sql-reference/version-control/merges)).

## Jujutsu

No locks: each command records a new operation on the latest one, and
committing an operation cannot fail. The next command that finds several
operation heads merges their views three-way; a conflict, such as a
bookmark moved to two places, is recorded as a conflict, not refused
([docs](https://docs.jj-vcs.dev/latest/technical/concurrency/)).

## Git

Lock files are created `O_CREAT|O_EXCL` to "notice and fail if somebody
else has already locked the file"
([api-lockfile](https://git-scm.com/docs/api-lockfile)); ref locks retry
for 100 ms (`core.filesRefLockTimeout`), then fail
([git-config](https://git-scm.com/docs/git-config)).

## SQLite

- One writer at a time, in WAL mode too; readers see a snapshot and do not
  block it ([wal](https://www.sqlite.org/wal.html)).
- `busy_timeout` sleeps repeatedly until the time is spent, then returns
  `SQLITE_BUSY` ([busy_timeout](https://www.sqlite.org/c3ref/busy_timeout.html));
  the default handler's sleeps are 1, 2, 5, 10, 15, 20, 25, 25, 25, 50,
  50, 100 ms, then 100 ms each (`sqliteDefaultBusyCallback`,
  [`main.c`](https://raw.githubusercontent.com/sqlite/sqlite/master/src/main.c)).
  *Inference:* waiters poll; whoever wakes when the lock is free takes it;
  no order is kept.
- A `DEFERRED` transaction that read first and writes later gets
  `SQLITE_BUSY` if another connection wrote meanwhile, and the busy handler
  is skipped for it; `IMMEDIATE` takes the write lock before reading
  ([lang_transaction](https://www.sqlite.org/lang_transaction.html),
  [busy_handler](https://www.sqlite.org/c3ref/busy_handler.html)).
- `BEGIN CONCURRENT` (a branch) checks at commit whether any page it read
  changed and returns `SQLITE_BUSY_SNAPSHOT` if so; the client can only
  roll back; under heavy contention the doc suggests an application mutex
  ([begin_concurrent](https://www.sqlite.org/src/doc/begin-concurrent/doc/begin_concurrent.md)).

## Datomic

Every transaction is forwarded to one active transactor, which executes
them in order; each commit is a storage compare-and-swap on the previous
one; writes are strongly serializable because they are fully serialized
([Jepsen](https://jepsen.io/analyses/datomic-pro-1.0.7075),
[docs](https://docs.datomic.com/transactions/acid.html)). Transaction
functions run on the transactor, inside that order.

## LMDB

"Writes are fully serialized; only one write transaction may be active at
a time"; readers take no locks and neither blocks the other
([`lmdb.h`](https://raw.githubusercontent.com/LMDB/lmdb/mdb.master/libraries/liblmdb/lmdb.h)).

## Clojure refs

MVCC with automatic retry ([refs](https://clojure.org/reference/refs)).
From [`LockingTransaction.java`](https://raw.githubusercontent.com/clojure/clojure/master/src/jvm/clojure/lang/LockingTransaction.java):
a transaction may abort ("barge") another only if it has run more than
10 ms and started earlier; its start point is kept across retries, so
*(inference)* the oldest eventually wins; retries stop at 10,000. A
`commute` is not read-checked: at commit its function is applied again to
the latest value, so a commute-only transaction does not retry.

## Contention managers in software transactional memory

Always-wait and always-abort policies both risk starvation (Scherer and
Scott, [PODC 2005](https://www.cs.rochester.edu/u/scott/papers/2005_PODC_CM.pdf)).
Karma keeps a transaction's priority (work invested) across aborts;
Timestamp aborts newer enemies; Polka adds randomized exponential backoff
to Karma and was their suggested default. Greedy (Guerraoui, Herlihy,
Pochon, [EPFL](https://infoscience.epfl.ch/record/64477)) keeps a
timestamp across restarts and guarantees every transaction commits within
bounded time. GCC's libitm retries in hardware a few times, then falls
back to a serial, irrevocable mode
([`beginend.cc`](https://raw.githubusercontent.com/gcc-mirror/gcc/master/libitm/beginend.cc)).

## FoundationDB

Optimistic: a commit fails with `not_committed` (1020) if a write
committed since its read version touches a range it read; resolvers keep
5 s of writes; a transaction over 5 s fails `transaction_too_old`
([developer guide](https://apple.github.io/foundationdb/developer-guide.html),
[architecture](https://apple.github.io/foundationdb/architecture.html)).
`on_error` resets the transaction and backs off exponentially; retries are
unlimited by default ([C API](https://apple.github.io/foundationdb/api-c.html)).

## What it says about this topic (inference)

- Starvation of a long transaction under optimistic retry is the known
  failure of always-abort policies. Three remedies recur: wait instead of
  abort (SQLite `IMMEDIATE`, LMDB, Datomic: our queue); priority by age or
  work kept across retries (Clojure, Greedy, Karma: not measured here); a
  serial fallback after a few aborts (libitm: our hybrid).
- Dolt is the nearest system to direction F: a server the CLI discovers
  through a file and connects to, a commit lock per branch, the merge
  computed outside it. Its commit still fails after 5 retries and leaves
  conflicts to the client, so it would not meet a zero-failure objective
  as it is.
- Writer branches have two precedents, Dolt's branch per agent and jj's
  operation heads, and both report a conflict as data. Our G prototype
  elects one value and reports nothing: that is the gap to close if G is
  kept.
- Polling with backoff keeps no order (SQLite), which is why a queued
  writer here blocks rather than polls.
- `commute`, an update applied to the latest value at commit instead of
  re-run, is the "apply its claims as computed" half of the question left
  open on 2026-11-24, for writes that only add.
