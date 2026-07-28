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

# Approved confirmation: exactly ten pairs, thirty episodes.
nix develop -c bench/bin/bench handoff --pairs 10 \
  --variant claim-handoff-confirmation
```

#### Pilot result

The three-pair `claim-handoff-pilot` passed every exact verifier:

- all three A agents completed the task, changed the claim revision on the
  repository DID, retained the opaque owner convention, and passed the hygiene
  checks;
- all three treatment and all three control B agents returned the exact label;
- no B agent changed its copied post-A spot.

Treatment reduced median total actions from `5` to `2` (`60%`), Tonk calls from
`4` to `1` (`75%`), orientation calls from `2` to `1` (`50%`), wall time from
`29s` to `19s` (`34.5%`), input tokens from `105142` to `61566` (`41.4%`), and
output tokens from `921` to `632` (`31.4%`). Every pair favored treatment by
exactly three actions and by `6–12s`.

This advances the treatment but does not establish significance. With three
pairs, all favoring treatment, the smallest possible one-sided paired
sign/randomization result is `p = 0.125`. Run at least ten pairs for
confirmation.

#### Confirmation decision rule

The ten-pair confirmation was approved after the pilot and before any
confirmation episode ran. Do not replace failed episodes or change prompts,
fixtures, model, metrics, or verification rules during the batch.

The treatment confirms only when all of these hold:

1. All ten A episodes pass the task, claim-identity, revision, retention, and
   hygiene checks, and every B spot remains unchanged.
2. At least nine treatment B episodes return the exact opaque owner label, and
   treatment exact-answer success is no worse than control.
3. At least eight pairs have exact answers in both arms and can be compared for
   efficiency.
4. Among those jointly successful pairs, treatment reduces median total actions
   by at least 25%.
5. A one-sided exact paired sign-flip test on the mean
   `control actions - treatment actions` difference reports `p < 0.05`.

Wall time, Tonk calls, orientation calls, and tokens are secondary explanatory
metrics. Report all episodes and protocol deviations even if the gate fails.

#### Confirmation result

The clean-revision `claim-handoff-confirmation` batch ran exactly thirty
episodes with no replacements. All ten A episodes passed every task, claim,
revision, retention, and hygiene check. All ten control B and all ten treatment
B episodes returned the exact opaque label, and every copied B spot remained
unchanged.

In numeric pair order, total-action savings were
`[4, 2, 3, 1, 3, 3, 1, 5, 4, 0]`: nine pairs favored treatment and one tied.
Median actions fell from `4.5` to `2` (`55.6%`), with mean paired savings of
`2.6` actions. The pre-registered one-sided exact paired sign-flip test has two
equally or more extreme assignments out of `1024`, so `p = 0.001953125`.
Every confirmation gate passes.

Secondary medians also favored treatment:

- shell commands: `3.5` to `1` (`71.4%` lower);
- Tonk calls: `3` to `1` (`66.7%` lower);
- orientation calls: `2` to `1` (`50%` lower);
- wall time: `35s` to `17s` (`51.4%` lower);
- input tokens: `113512` to `61074` (`46.2%` lower);
- output tokens: `988.5` to `469.5` (`52.5%` lower).

Four treatment agents answered directly from the loaded projection; six
re-read the live claim once. The tied pair used one orientation call in each
arm. Wall time favored treatment in eight pairs and control in two, so the
action result is stronger than the timing result. Randomization produced seven
treatment-first and three control-first pairs; this imbalance is a limitation,
not a protocol deviation.

The confirmation audit also found that the original B verifier's
`branch_unchanged` field compared only the agent-context claim revision. A
post-run bytewise comparison proved all twenty B site trees identical to their
frozen post-A origins, so the result still satisfies the guardrail. The harness
now checks both the claim revision and the complete site tree automatically.

The raw pass also exposed two reporting defects, corrected before aggregation:
Codex `file_change` events were absent from `tool_calls`, and a quoted `rg`
pattern containing `|tonk ...` was mistaken for a shell pipeline. Metrics were
recomputed from the preserved transcripts after both regressions were added.

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
