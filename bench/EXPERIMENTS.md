# Agent CLI experiments

This protocol measures whether a Tonk CLI treatment improves autonomous agent
performance, not whether a scripted ideal path works.

## Frozen first-use task

`bench/scenarios/first-use` contains a seeded task list and asks an agent with no
Tonk-specific instructions to change one existing field. Do not edit its task,
seed, or rubric between comparison arms. A prompt change creates a new
experiment.

## Run sequence

```sh
# Mechanics only. This does not count as agent evidence.
nix develop -c bench/bin/bench run first-use --scripted --variant mechanics

# Pilot each arm before paying for confirmation.
nix develop -c bench/bin/bench run first-use --runs 3 --variant baseline
nix develop -c bench/bin/bench run first-use --runs 3 --variant treatment

# Confirmation: at least ten independent runs per arm.
nix develop -c bench/bin/bench run first-use --runs 10 --variant baseline
nix develop -c bench/bin/bench run first-use --runs 10 --variant treatment

nix develop -c bench/bin/bench compare first-use baseline treatment
```

Keep the runner and model fixed. If confirmation runs cannot be interleaved,
finish both arms close together and record the timing limitation.

## Trusted spot AGENTS.md pilot

`--spot-agents` installs a trusted, scenario-owned `AGENTS.md` in the episode's
working directory and records that exposure in `experiment.json`. It is a
best-case test of whether automatically loaded spot-specific examples remove
the remaining context call:

```sh
nix develop -c bench/bin/bench run first-use --runs 3 \
  --variant spot-agents-pilot --spot-agents
```

Compare it to the graduated `workflow-context-pilot` arm. The expected direct
trajectory is `query -> assert`, so the primary metric can improve from two
commands before the write to one.

This pilot does not establish that an `AGENTS.md` inside every real spot is
discoverable. Codex searches from the project root down to its current working
directory, not into child directories. A file at `project/.tonk/AGENTS.md`
therefore does not load when the agent starts at `project/`. It also does not
test cross-device memory or untrusted collaboration: remotely supplied text
must never become automatic agent instructions without an explicit trust
decision.

A later durable-memory experiment must use two episodes: one agent records a
bounded durable fact, then a fresh agent uses that fact. Score both correct
retention and the absence of secrets, transient status, or instruction drift.

## Pre-registered decision rule

Primary metric: shell-command index before the first successful Tonk content
write, among episodes that both reached a write and passed the scenario's
execution-based final-state verifier. Lower is better. The judge outcome remains
a guardrail for UI quality and collateral that the structured verifier cannot
see.

A treatment graduates when all of these hold:

1. At least ten scored episodes and eight successful episodes exist in each
   arm.
2. Median commands before the first write improve by at least 25%.
3. An exact one-sided permutation test of that median reports `p < 0.05`.
4. Success rate is no more than five percentage points below baseline.
5. Mean outcome is no more than 0.5 points below baseline.

Secondary metrics explain the result but do not override the gate: first live
read, Tonk failures, orientation calls, total tool calls, wall time, output
tokens, repeated commands, and collateral mutations from the rubric.

## Interpretation

- The three-run pilot validates the harness and estimates variance. It cannot
  establish significance.
- A scripted run validates mechanics only.
- A faster failed run is not an improvement.
- Inspect raw transcripts even when the aggregate passes. A new failure mode
  can hide behind a better median.
- Validate a winning treatment on `targeted-edit` and `interview-build` before
  treating it as a general CLI improvement.
