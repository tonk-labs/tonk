# Tonk CLI agent DX integration

Date: 2026-07-28
Branch: `feat/cli-agent-dx`
Base: `staging` at `e1fee7c65`

## Objective

Integrate the strongest ideas from `feat/cli-dx` and
`feat/cli-dx-codex`, but only graduate CLI behavior that measurably improves
held-out agent trajectories without reducing task correctness.

## Experiment-first sequence

1. Land measurement infrastructure and the frozen `first-use` scenario without
   changing CLI behavior.
2. Run a three-run baseline pilot to validate isolation, scoring, and trajectory
   classification.
3. Run at least ten baseline episodes once the pilot is sound.
4. Add one coherent treatment at a time.
5. Run the same pilot and confirmation sequence for each treatment.
6. Keep treatments only when they pass the pre-registered gate in
   `bench/EXPERIMENTS.md`.

## Controls

- Pin `TONK_SPOTS_STATE` and `TONK_SPOT`; cwd never selects Tonk data.
- Keep scenario task, seed, rubric, runner, and model unchanged across arms.
- Record the variant, Git revision, dirty state, runner, and model in every run.
- Judge rendered/final state separately from mechanical trajectory metrics.
- Use an execution verifier for exact data state; screenshots cannot prove row
  counts when the renderer falls back to a single default item.
- Do not treat the scripted three-command trajectory as agent evidence.
- Do not expose the developer's real Tonk profile to an episode.

## Decisions

- Use `first-use` for the primary discovery experiment. It supplies no command
  hint and asks for one precise update to seeded data.
- Use `targeted-edit` and `interview-build` as transfer checks before calling a
  treatment generally useful.
- Prefer the richer success-aware trajectory classifier from
  `feat/cli-dx-codex`; retain dry-run and command-class counts from
  `feat/cli-dx`.
- Compare arms with an exact one-sided permutation test when the sample is
  small enough, plus a bootstrap interval and a minimum practical effect.

## Open risks

- Agent and judge behavior are stochastic.
- Model service updates can invalidate comparisons spread across long periods.
- Sequential arms can be confounded by time; confirmation runs should be
  interleaved when the harness can select binaries without rebuilding.
- A command classified as a write can still be a semantic no-op. The execution
  outcome and judge score remain required guardrails.

## Deviations and findings

- The first scripted mechanics run rendered only one task and showed the
  existing per-item fallback banner. A screenshot could not prove the scenario's
  exact two-row invariant, so the harness gained a scenario verifier before any
  paid agent run.
- Successful `tonk eval` calls were initially classified as writes. They can be
  pure queries, so the classifier now requires different `revision-before` and
  `revision-after` values before treating an eval as a write.
- Subcommand matching initially scanned the full shell command, including eval
  source. A field named `status` could therefore misclassify an eval as
  orientation. Matching is now anchored to the actual token after `tonk`.
- The first real episode exposed two harness edge cases without losing its
  evidence: `query --help` was counted as a data read, and a quoted jq fallback
  stopped Markdown reporting. Help requests now count as orientation, and the
  completed episode can be re-reported from its preserved artifacts.
- First-use episodes run with a fresh home/profile under the run directory.
  Runner authentication remains available, but the developer's Tonk profile is
  outside the episode sandbox.
