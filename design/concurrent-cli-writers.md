# Agents running the CLI on one space at once fail one another's writes

**Phase: Directing.**

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

Not chosen yet.
