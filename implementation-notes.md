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
- One baseline agent used JSON-formatted eval output. The classifier initially
  understood only notation envelopes, so it missed a verified write. JSON eval
  commits now count as writes when `commits.claims` is nonzero.
- First-use episodes run with a fresh home/profile under the run directory.
  Runner authentication remains available, but the developer's Tonk profile is
  outside the episode sandbox.

## Treatment 1: workflow-first live context

The ten-run baseline passed the verifier in every episode, with first-write
indices `12, 13, 11, 14, 10, 8, 9, 8, 6, 9` (median `9.5`). Every episode
spent most pre-write calls on orientation. Repeated failures were a guessed
`tonk context`, bare `tonk use`, and bare `tonk assert --help`; agents also
re-read after writes because assert printed stale pre-commit matches.

The first treatment therefore combines:

- bare `tonk` / `tonk context` as a bounded live workflow card;
- top-level help that shows `query -> assert -> query`, not a conceptual loop;
- generic and schema-derived assert help that both succeed;
- bare `tonk use` as transparent selection inspection;
- data reads without transaction-envelope noise;
- an assert response that performs a fresh read and prints current state.

The prototype context's mental-model essay and view inventory were rejected
for this treatment. The experiment says examples and executable flows are the
scarce resource. Existing guides remain available for compatibility but are no
longer the first-use recommendation; deleting or replacing them needs its own
transfer experiment.

Spot-local `AGENTS.md` is also held out of this treatment. It is a separate
hypothesis about automatic orientation and durable spot memory, so combining it
now would make any measured gain impossible to attribute.

### Result

The ten-run treatment passed the verifier in every episode. First-write indices
were `2, 3, 2, 3, 3, 2, 2, 2, 3, 2` (median `2.0`), with zero failed Tonk
calls. Against the baseline median of `9.5`, this is a `78.9%` improvement with
a bootstrap 95% interval of `[66.7%, 83.3%]`, Cliff improvement `1.000`, and an
exact one-sided permutation `p = 0.000568`. Mean judge outcome moved from
`9.50` to `9.10`; success stayed at `100%`. Every pre-registered gate passed,
so this treatment graduates.

Several judges claimed the agent trusted a silent write. Raw episode JSONL
shows otherwise: `tonk assert` returned `current state:` with the intended
entity and `done: true`. One judge later recognized that echo explicitly. This
is a judge-observability limitation, not a reason to add another verification
command.

The repeated auto-sync authorization warning is real but induced by the
experiment's isolated agent profile. The broken browser task route is also
real, but it predates and is independent of CLI orientation. Both should be
tested separately.

## Treatment 2 hypothesis: claim-backed spot AGENTS.md

Current Codex discovery is root-to-cwd only and happens once per run. That
means a filesystem file cannot be the durable object:

- canonical spots live under Application Support, where agents rarely start;
- an adopted `project/.tonk/AGENTS.md` is a child of the project cwd and is not
  discovered;
- cwd does not select a Tonk spot, so one project can use several spots and one
  static project file cannot safely claim which is active;
- a local file does not travel with synced spot data.

The durable object is instead a cardinality-one Dialog claim on the same stable
repository subject DID used by `tonk/repository`. The standard-library concept
is `tonk/agents`; its Markdown attribute is `xyz.tonk.repo/agents`. `TONK_SPOT`
selects the repository carrying the claim, independent of cwd.

`tonk agents` exports the raw Markdown, `tonk agents --json` exposes its subject
and observed revision, and `tonk agents set` imports a file or stdin. `tonk
context` includes the same claim with source and revision. The first experiment
asserts a trusted fixture, reads it back, verifies the subject, and only then
projects it into the episode cwd for Codex.

This corrects the earlier filesystem-first prototype. The projection is an
adapter, never authority. A real launcher must not silently overwrite unimported
local edits, and automatic loading is a trust boundary because any authorized
spot writer can change the claim. Cardinality one also means concurrent
whole-document updates can overwrite each other; a revision precondition or
merge flow is required before general multi-agent editing.

Durable memory still needs a two-episode handoff test: one agent records a
bounded durable fact in the claim, then a fresh agent receives a newly generated
projection and must use it. The current external episode budget is exhausted,
so only mechanical claim round-trips run until a new episode budget is approved.

### Mechanical result

The scripted `claim-agents-mechanics` run asserted the fixture on the repository
DID, read it back with source and revision metadata, produced a byte-identical
projection, and passed the unchanged two-task execution verifier. A focused sync
test also queried the typed `RepositoryAgents` claim from the upstream branch,
confirming the Markdown travels as content-branch data rather than checkout
state. No external agent episode was consumed.

The pre-registered handoff pilot uses three pairs. Episode A writes an opaque
spot convention into the claim; fresh treatment and control episode-B agents
share the post-A spot, but only treatment gets the updated pre-launch
projection. Control keeps the old projection and can still discover the current
claim through the CLI. This isolates automatic orientation from access to the
underlying data and costs nine external episodes before any confirmation run.

The handoff harness had two dry-run-only corrections before external episodes:
the typed `tonk query --json` verb returns a top-level result array rather than
an evaluator envelope, and the retention predicate initially matched `owner`
but not the fixture's `owned`. No model call occurred before all three scripted
pairs passed. The final dry path also opens each copied B spot under B's fresh
profile, so profile/delegation setup is exercised before the paid run.

### Handoff pilot result

The approved nine-episode pilot completed at
`bench/runs/20260728-144336-agents-handoff-claim-handoff-pilot`.

All three episode-A agents updated the repository-DID claim with the exact
opaque owner convention, preserved the unrelated workflows, omitted one-off
completion state and secrets, and completed the task correctly. All six fresh
episode-B agents returned the exact owner label and left their copied post-A
spots unchanged.

Corrected paired medians:

- total actions: control `5`, projected claim `2` (`60%` lower);
- Tonk calls: `4` vs `1` (`75%` lower);
- orientation calls: `2` vs `1` (`50%` lower);
- wall time: `29s` vs `19s` (`34.5%` lower);
- input tokens: `105142` vs `61566` (`41.4%` lower);
- output tokens: `921` vs `632` (`31.4%` lower).

Every pair saved exactly three actions; paired wall-time savings were `12s`,
`6s`, and `8s`. This passes the pilot gate but cannot pass a significance gate:
three same-direction pairs imply a minimum one-sided exact sign/randomization
`p = 0.125`. Confirmation needs at least ten pairs.

Two metrics defects were corrected and the preserved transcripts re-reported:
Codex `file_change` events now count as tool actions, and a quoted `rg` pattern
containing `|tonk ...` no longer looks like a shell-position Tonk executable.
Episode exit files are also written before metrics are computed. One A agent
ignored the prompt's no-filesystem-search instruction and read global Codex
memory, causing extra work and repeated commands; the opaque convention was not
present there, so retention and B-arm isolation remain valid.

### Handoff confirmation protocol

Fresh approval on 2026-07-28 raised the hard external budget from nine pilot
episodes to thirty confirmation episodes: ten independent
A/control-B/treatment-B pairs with no replacements. The decision rule was
frozen before launch in `bench/EXPERIMENTS.md`.

Correctness and claim hygiene remain guardrails. The confirmatory performance
statistic is the one-sided exact paired sign-flip test on mean total-action
savings among pairs where both B answers are exact. No treatment, prompt,
fixture, model, verifier, or metric change is permitted once the confirmation
batch begins.

### Handoff confirmation result

The approved batch completed at
`bench/runs/20260728-150040-agents-handoff-claim-handoff-confirmation`.
Its experiment metadata records clean revision
`c283e2fe7b9fbb905dadb94ea8b98f9dfa844717`, model `gpt-5.5`, ten planned
pairs, and an approved hard cap of thirty episodes. Exactly thirty episode exit
files exist and all record exit zero; no replacement episode ran.

All ten A verifiers and all twenty B verifiers passed. Every A claim retained
the opaque convention on the repository DID without task status or secrets.
Every B answer was exact and every B agent-context claim revision remained
unchanged; the later full-site audit is recorded below.

Control versus projected-claim medians were:

- total actions `4.5` vs `2`;
- shell commands `3.5` vs `1`;
- Tonk calls `3` vs `1`;
- orientation calls `2` vs `1`;
- wall time `35s` vs `17s`;
- input tokens `113512` vs `61074`;
- output tokens `988.5` vs `469.5`.

Numeric-pair action savings were `[4, 2, 3, 1, 3, 3, 1, 5, 4, 0]`. The mean
paired saving was `2.6`; enumerating all `1024` sign assignments found two at
least as large as the observed sum, for one-sided `p = 0.001953125`. The median
action reduction was `55.6%`, so every pre-registered confirmation gate passed.

Raw trajectories show four treatment agents wrote the answer without a Tonk
call and six re-read the claim once. Control agents needed between one and four
Tonk calls. Pair ten tied at two actions because both arms made one orientation
call. Treatment wall time was slower by two and three seconds in pairs four and
seven, but the wall median still fell by `51.4%`.

The initial command attempt stopped before batch creation because the sandbox
made Nix's fetcher cache read-only. The identical committed command then ran
with normal Nix cache access. There was no in-batch protocol deviation.
Randomized B order was imbalanced at seven treatment-first versus three
control-first pairs; report this when generalizing the result.

The raw audit caught a misleading verifier name: `branch_unchanged` compared
only the before/after `tonk agents --json` claim revision. A bytewise recursive
comparison of every control and treatment site against its frozen post-A origin
found all twenty identical, so the confirmation guardrail still passes. The
harness now records separate `claim_revision_unchanged` and `site_unchanged`
checks and requires both.
