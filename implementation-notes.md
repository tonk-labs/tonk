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
