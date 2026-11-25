# Journal: concurrent CLI writes on one replica

## Where things stand

Building (2026-11-25): decided on a queue per branch, then a daemon; the
report is in the topic's file.

Exploring. No upstream: H1 and H2 hold from N = 2; at N = 16, 81% of
evals fail; no write that reported success was lost. An unexplained head
publish before the eval's own commit appears only under contention (H5).
Next: the same with a local upstream (H3), and a cold eval on a space of
realistic size.

Update 2026-11-25: of five policies, queue and hybrid reach zero failures
in every run; queue was as fast or faster. See results/2026-11-25.

## Datasets

None: each run builds a fresh replica with one concept and asserts one
note per writer. Space size is the smallest possible, which flatters the
cold-start time.

## 2026-11-24 — which concurrent evals fail, and how

**Question.** With N evals at once on one replica and no upstream, which
fail, with which error, and is any write lost?

**Method.** `harness/run.sh`, N = 2, 4, 8, 16, interleaved, three rounds;
tonk `c0ec36e`, dev profile, 4 cores.

**Results.** [results/2026-11-24](results/2026-11-24.md). Failed, % of N:
50, 67, 58, 81. 47 version mismatches, 17 head-lock refusals, 0 other, 0
lost. 22 "could not retain revision for checkout" warnings, 6 of them on
evals that succeeded. Sequential cold eval: median 31 ms.

**Decision.** H1 and H2 confirmed; H4 not seen (no failure outside the
commit); a write that reports success lands. New, H5: under contention
some processes publish a branch head before their own commit, and a
successful commit can miss the checkout index.

Commits: harness at `c0ec36e`-based branch.

## How to reproduce

```bash
cargo build -p tonk-cli --bin tonk
```

```bash
for r in 1 2 3; do for n in 2 4 8 16; do TONK=$PWD/target/debug/tonk research/concurrent-cli-writes/harness/run.sh $n r$r /tmp/runs; done; done
```

## 2026-11-25 — policies for a lost race

**Question.** Which of once, retry, backoff, queue, hybrid reach zero
failures, small writes and a long one?

**Method.** `harness/eval-policies.patch`; `run.sh` at N = 2-16 and
`starve.sh native 500 8`, three rounds, interleaved; `TONK_EVAL_WARM`
for a commit in a process that stays open. An earlier start was stopped
and discarded: a build ran during it.

**Results.** [results/2026-11-25](results/2026-11-25.md).

**Decision.** Queue and hybrid pass the bars; retry and backoff fail on
the long eval; once fails everywhere.
