# Concurrent CLI writes on one replica

**Status: Building.** Decided 2026-11-25: a queue per branch, then a daemon. First round in: [results/2026-11-24](concurrent-cli-writes/results/2026-11-24.md). Answers the diagnosis of
[design/concurrent-cli-writers](../design/concurrent-cli-writers.md).

## The question

When N `tonk eval` processes write to one space's replica on one machine at
once, which writes fail, with which error, and is any write lost?

## Hypothesis

At N ≥ 2, some evals fail with exit 4. Without an upstream the failures are
`Version mismatch` (H1) and lock refusals (H2), in a ratio that shifts
toward lock refusals as N grows; with an upstream, pulls add H1 failures
(H3). No reported success is missing from the final head. Start-up work
(H4) is not a source.

## Measured against

`main` of tonk as it is (`c0ec36e`), with dialog at the revision tonk pins.
Each run starts from a fresh replica built from one seed.

## What settles it

This is a diagnosis, so there is no bar to clear; the outcome is which
hypotheses hold:

- **H1/H2 hold** if their errors appear in at least one of three runs at
  some N ≤ 16, each attributed by its error text.
- **A lost write** is any eval that exits 0 whose fact is absent from the
  final head. One is enough to make it the first problem.
- **H4 holds** if any failure occurs before the eval's transaction opens
  (an error not from the commit).
- **Nothing fails** at N = 16 in three runs: the symptom is not local, and
  the diagnosis moves to the remote (H3 and the shared upstream).

## Bars for the options (set 2026-11-25, before their results)

- **Success:** zero failed evals in every run: 2, 4, 8 and 16 writers,
  three rounds, and the 500-note eval against 8 back-to-back writers. Zero
  lost writes.
- **Failure:** any failed eval from a local overlap, in any run.
- **The middle**, for the maintainer: among options that reach zero, the
  time a small write takes at 16 writers and how long the long eval holds
  the others up.

## Report (2026-11-25)

Decided: a queue per branch for every local writer now, then a daemon
that stays up for a while ([design topic](../design/concurrent-cli-writers.md)).
The numbers that decided it, all on tonk `c0ec36e`, dev profile, 4 cores,
an empty space, three interleaved rounds per table
([results/2026-11-24](concurrent-cli-writes/results/2026-11-24.md),
[results/2026-11-25](concurrent-cli-writes/results/2026-11-25.md)):

| option | failed, 2-16 writers | a long eval against 8 back-to-back writers | ms per eval at 16, p50 / p95 | a counter, 8 increments |
|---|---|---|---|---|
| once (main) | 50-83% | fails | 125 / 147 | - |
| retry | 0 | never lands (64 tries) | 257 / 336 | - |
| backoff | 0 | never lands (64 tries) | 198 / 368 | - |
| queue | 0 | lands first try, ~0.3 s | 151-194 / 233-293 | 8 |
| hybrid | 0 | lands after one loss, ~0.6 s | 194 / 274-287 | - |
| age priority | 0 | lands, ~0.4 s (K = 500) | 186 / 285 | - |
| writer branches | 0 | lands, ~0.6 s | 164 / 188 | 1-2 |

On Chronicle the lock refusals and missing checkout entries go away and
nothing else measured changes. A commit in a process that stays open:
~8 ms, against ~36 ms in its own.

Left open: cold start on a space of realistic size; whether tonk's
notation can count stored facts; the remote head.
