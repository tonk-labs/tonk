# Concurrent CLI writes on one replica

**Status: Exploring.** First round in: [results/2026-11-24](concurrent-cli-writes/results/2026-11-24.md). Answers the diagnosis of
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
