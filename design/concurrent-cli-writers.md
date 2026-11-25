# Agents running the CLI on one space at once fail one another's writes

**Phase: Designing.**

## Description

Gavin reports, through Irakli (2026-11-24), that several agents working on
one space with the same `tonk` CLI keep running into conflicts, "because the
head moves and so on". Irakli's reading is that concurrency is not managed,
and he brings three remedies to weigh: a daemon shared by the CLI's
processes, as sapling has, so they share caches and skip a cold start per
command; a queue that serializes concurrent writes; and optimistic
concurrency, where a transaction that loses its race runs again, which
dialog's stacks now offer.

BUG-350 in the tonk-team space ("Multi-agent write contention: shared heads, local
locks and checkpoint limitations", triage, filed 2026-11-21 from another
project's report, not reproduced while filing) says:

- writers on one machine: 6 of 8 concurrent `tonk eval --no-sync` calls
  failed with `revision.lock` held, exit 4;
- a swarm that pulls before and pushes after every eval races on the one
  remote head, with retries and pulls stuck mid-turn; the reporters worked
  around it by syncing only at checkpoints, and say even that collides at
  dozens of agents;
- the reporters keep their records add-only so that merges do not conflict.

What the code does today is in the [claims ledger](concurrent-cli-writers/claims.md).
In short: each `tonk` invocation opens the replica on its own, `tonk eval`
commits with one try, the head cell's lock file refuses a second writer at
once rather than waiting, nothing retries that refusal, and a committing eval
pulls and pushes around every write. A shared-process owner already exists
in tonk for MCP servers (`tonk-agent-peer`); the CLI does not use it.

## Problem

Agents on one machine, each running the tonk CLI against one replica,
need every write they make to land. When writes overlap, all but one
fail: eval treats a head that moved while it ran as an error, the file
store refuses a writer that finds the head locked instead of making it
wait, and a write that is run again against frequent short writes can
lose every time. With 16 writers at once, 85% of writes fail, and the
agent is left to retry by hand.

**Objective** (Irakli, 2026-11-25): no write on one machine fails because
another local writer overlapped it. Zero, not fewer.

## Approach

Decided 2026-11-25 (Irakli), at the level of strategy, from the options
measured in [direction.md](concurrent-cli-writers/direction.md):

- **A queue per branch, now.** Every local writer, sync included, takes
  its branch's turn from reading to publishing; writers on other branches
  never wait. A write that still loses to something outside the queue
  backs off and runs again. Reads are not queued: they read a pinned
  revision, as they do now.
- **A daemon that stays up for a while**, as sapling's does: started by
  the first command that needs it, gone after an idle period. It holds the
  space open, so caches stay warm across commands (a commit ~8 ms in a
  process that stays open, against ~36 ms in a process of its own), and
  takes local writes through the queue.
- **Sync in the background.** The daemon syncs when local writers are
  idle, and interleaves sync with them when no idle moment comes; it
  fetches and rebases outside the queue and holds it only to swap the
  root, falling back to rebasing in its turn when it keeps losing.

Not taken: writer branches with a reconciler (no failures, but they lose
read-modify-write updates and report no conflicts); retry and backoff
alone (a long write never lands); age priority (reaches zero, slower than
the queue everywhere).

Open for the design phase: where the daemon's lifecycle comes from
(sapling's modules, or tonk-agent-peer's owner), how the CLI finds and
talks to it, what a command does when no daemon runs (the CLI takes the
queue itself), the idle period, the bound after which sync interleaves,
and the shared checkout index that collides under concurrent writers.
