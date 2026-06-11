---
name: bench-iterate
description: One iteration of the bench improvement loop — run a scenario, pick the top friction item, fix it, rerun, compare, keep or revert. Designed to be driven by /loop (e.g. `/loop /bench-iterate from-scratch`).
---

# bench-iterate

One full improvement iteration against a bench scenario. Invoked with
the scenario name as argument (default: from-scratch). See
`bench/README.md` for how the harness works.

## Hard rules

- **One change per iteration.** A score delta must be attributable to
  exactly one fix. If the friction item needs multiple changes, do the
  smallest first and leave the rest for later iterations.
- **The benchmark is frozen.** Never edit `bench/scenarios/*/task.md`,
  `rubric.md`, `checkpoints`, or fixtures. If the benchmark itself
  seems wrong, stop and report to the human instead of changing it.
- Commit with jj (`jj commit -m "..."`), conventional-commit style.

## Procedure

1. Run the scenario: `nix develop -c bench/bin/bench run <scenario>`.
   If there is no prior scored run for this scenario in
   `bench/runs/index.jsonl`, this run is the baseline measurement —
   record it, promote its screenshots
   (`bench/bin/bench baseline <scenario> <run-dir>`), and stop.
2. Read the new run's `report.md` and `scores.json`. Compare with the
   previous run for the same scenario (`bench/bin/bench report`).
3. Pick the single highest-leverage friction item: prefer items that
   (a) appear in multiple runs, (b) caused failed tool results or
   repeated commands, (c) have a concrete `suggested_fix`.
4. Fix it in the product: slide CLI behavior or errors
   (`rust/slide`), the guide text (`rust/slide/src/guide*.md`), the
   notation/analyzer surface, or tonk-ui rendering. Rebuild what
   changed (`cargo build --release -p slide`; the stack rebuilds
   tonk-ui on the next run).
5. Rerun: `nix develop -c bench/bin/bench run <scenario>`.
6. Compare outcome, friction count, failed_tool_results, num_turns
   against the pre-fix run.
   - Improved, or equal outcome with less friction: commit the fix.
   - Worse: do NOT revert on your own (`jj abandon` requires asking
     the human) — describe the regression in the iteration summary
     and ask.
7. Write a 5-line iteration summary: what was fixed, score before →
   after, what the next-highest friction item is.

## Stopping

Stop the loop (tell the human) when outcome >= 9 on two consecutive
runs, or when the top friction item requires a design decision.

## Cost note

Each iteration spends two headless claude runs (episode + judge) plus
the fix work. Default to one run per iteration; use
`bench run <scenario> --runs N` only when variance on a specific
change matters.
