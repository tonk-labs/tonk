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

`--spot-agents` asserts scenario-owned Markdown as a Dialog claim on the spot's
repository subject DID. The harness reads the claim back, verifies its entity,
and projects it into the episode cwd as `AGENTS.md` before launch. The claim is
the source of truth; the file is a runtime adapter. Both exposure and source are
recorded in `experiment.json`.

This is a best-case test of whether automatically loaded spot-specific examples
remove the remaining context call:

```sh
nix develop -c bench/bin/bench run first-use --runs 3 \
  --variant spot-agents-pilot --spot-agents
```

Compare it to the graduated `workflow-context-pilot` arm. The expected direct
trajectory is `query -> assert`, so the primary metric can improve from two
commands before the write to one.

This pilot validates pre-launch projection, not automatic placement in every
runtime. Codex searches from the project root down to its current working
directory, not into child directories, so a launcher must project the selected
spot's claim into a path Codex will read. Cwd still does not select the spot.

The fixture is trusted. In collaboration, an authorized spot writer can change
the claim, so automatic instruction loading needs an explicit trust decision
and must show the repository subject and revision. A projection must also avoid
silently overwriting unimported local edits.

A later durable-memory experiment must use two episodes: one agent records a
bounded durable fact with `tonk agents set`, then the harness projects the new
claim for a fresh agent using the same spot. Score correct retention, entity and
revision continuity, and the absence of secrets, transient status, instruction
drift, or lost concurrent edits.

### Two-episode handoff pilot

Use three independent paired runs. Each pair costs three external episodes:

1. Episode A receives a normal spot task plus one durable, unguessable
   convention: future security-review tasks are owned by a scenario-generated
   opaque team label. It must complete the task and preserve the convention for
   future agents.
2. Verify the task separately, then verify that the live `tonk agents --json`
   revision changed, still maps the repository DID, contains the exact
   convention, and does not contain the completed task's status, credentials,
   invite links, or unrelated prompt text.
3. Start two fresh episode-B agents against the same read-only post-A spot. The
   treatment cwd receives a projection of the live updated claim; the control
   cwd receives the frozen pre-A projection. Randomize their order.
4. Ask each B agent to write only the opaque owner label to `answer.txt`. The
   control may still discover the live claim through `tonk context` or `tonk
   agents`; it is not denied access to spot data.

The primary outcome is exact answer success. Among successes, compare tool
calls, Tonk orientation calls, output tokens, and wall time. The pilot advances
only if all three A claims pass the retention/hygiene verifier, every treatment
B succeeds, and treatment does not use more median tool calls than control. A
confirmation requires at least ten pairs and randomized B-arm order.

```sh
# Harness mechanics only; no model calls.
nix develop -c bench/bin/bench handoff --scripted --pairs 3 \
  --variant claim-handoff-mechanics

# Approved pilot: exactly three A/control-B/treatment-B pairs, nine episodes.
nix develop -c bench/bin/bench handoff --pairs 3 \
  --variant claim-handoff-pilot
```

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
