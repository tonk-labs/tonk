# Agent-onboarding bench: measuring and optimizing the tonk CLI for agents

Date: 2026-07-07
Status: approved design, pre-implementation

## Problem

Agents handed a tonk invite take many turns figuring out what the CLI is, how
to install it, how to join, and how to find the right claims once inside. The
agent-invite prompt in `rust/tonk-core/assets/library/core.yaml` assumes
`tonk` is on PATH and gives no pointer to the llms docs
(`tonk-site/llms`). Existing bench scenarios all start *after* join with a
fully specified task, so none of this friction is measured.

## Decisions (made during brainstorming)

- **Measurement first.** New bench scenarios define "optimized"; CLI and copy
  changes then land one at a time through the `/bench-iterate` loop with
  before/after evidence. No speculative CLI features up front.
- **Three journeys get scenarios**: cold invite onboarding, returning-user
  targeted edit, interview-driven build.
- **npx-first distribution.** A `tonk` npm wrapper package (esbuild/biome
  pattern) so the invite prompt can say `npx tonk join '<url>'` — install and
  join in one line. Bench stays hermetic via a local tarball; registry
  publishing is its own PR.
- **Episodes run on codex + GPT-5.5.** The episode runner becomes pluggable
  (`EPISODE_RUNNER=codex|claude`); codex/gpt-5.5 is the default for the new
  scenarios. A model with no tonk in training is a stronger test of the
  docs and copy. The judge and the interview persona stay on headless
  claude (score consistency with existing baselines; the persona is not the
  system under test).

## Scenario 1: `cold-onboard`

The episode prompt is the *actual* agent-invite copy, extracted from
`core.yaml`'s `id:agent-invite/prompt` view template at run time and filled
(`{name}`, `{access}`, `{remote}`, `{code}`, `{dom.host/data-base}`,
`{dom.host/data-page}`) from a real invite the harness mints against the
hermetic stack. Copy edits in `core.yaml` are therefore automatically under
test — the scenario has no frozen prompt of its own.

Setup:

- Fresh working dir, no `.tonk/`.
- PATH sandbox contains **no `tonk` binary**. It does contain node/npm/npx.
- The npm wrapper package is available hermetically: `npm pack` output plus a
  primed npm cache (or `npm_config_registry` pointed at a static local
  registry dir), so `npx tonk` resolves offline. No real-registry traffic.
- Invite minted by the harness via the release `tonk` binary (harness-side
  only, outside the episode PATH), same as `bridge.sh` does today.

Judged outcome: pasted prompt → joined → oriented → at least one useful,
pushed change visible at the checkpoint URL. Rubric penalizes install
flailing, guessed schema, unpushed work.

Metrics (added to `metrics.sh` output): `seconds_to_join`,
`seconds_to_first_successful_eval`, `doc_fetches` (guide/llms.txt reads),
plus the existing failed-tool-results / repeated-commands / wall-clock.

## Scenario 2: `targeted-edit`

Pre-seeded spot with realistic data (reuse an existing fixture — the wiki
seed or the habit tracker — frozen into the scenario). Task is one specific
returning-user request, e.g. "rename the 'Groceries' page to 'Shopping'" or
"add a due-date to tasks and set the report task due Friday".

Judged outcome: exactly that change landed, nothing else mutated. Rubric
penalizes collateral writes, bulk retractions, and doc-reading toil relative
to the size of the ask. Metrics emphasize turns, tool calls, and wall-clock
to the first correct committed write.

## Scenario 3: `interview-build`

The episode PATH gains an `ask-user` command — a bridge script; each
invocation forwards the question to a second headless `claude -p` session
holding a persona prompt plus an append-only conversation-state file, and
prints the persona's reply (same second-session pattern as the judge).

Persona: a non-technical user with a vague goal ("something to organize my
book club") and latent preferences (e.g. wants an attendance list and a
voting mechanism, hates clutter) that surface only when asked concrete
questions. The persona file is scenario data and frozen with the rubric.

Judged outcome: both the artifact (does the built thing serve the surfaced
preferences?) and interview quality per `agent-flows.md`'s own standard —
concrete 2–3-option questions, one at a time, no open "what do you want?",
no 20-question interrogation. Metrics: `ask_user_calls`, plus the standard
set.

## Harness changes

- **Runner abstraction** in `episode.sh`: `EPISODE_RUNNER=codex|claude`.
  - claude: current `claude -p` streaming-JSON path, unchanged.
  - codex: `codex exec --json -m gpt-5.5` (exact flags/sandbox options
    verified at implementation time), transcript captured as JSONL.
- **Per-runner metrics adapter** in `metrics.sh`: both transcript formats
  reduce to the same `metrics.json` shape; fields a runner can't provide are
  null (as `num_turns` already is in one wiki-conversion run).
- **Prompt extraction** for `cold-onboard`: a small script pulls the
  agent-prompt template out of `core.yaml` (yq/awk over the
  `id:agent-invite/prompt` display block, HTML entities unescaped) and fills
  the invite fields.
- **PATH sandbox** for `cold-onboard`: episode env gets a minimal PATH
  (node toolchain, coreutils, no tonk) — a directory of symlinks, not a
  container.
- **`ask-user` bridge** for `interview-build`: script + persona file +
  conversation state under the scenario dir; conversation log saved into the
  run dir for the judge, which scores interview quality from it.
- Auth: codex episodes need an OpenAI credential in the environment;
  document alongside the existing `claude` login requirement in
  `bench/README.md`.

## npm distribution (PR 2)

Wrapper package `tonk` (name/scope to confirm at publish time): a tiny JS
launcher resolving a per-platform binary from `optionalDependencies`
(`tonk-darwin-arm64`, `tonk-linux-x64`, …), binaries built by release CI.
`npx tonk <cmd>` works with no global install. Bench does not depend on the
registry — the scenario uses the packed tarball — so this PR can land in
parallel with baseline collection, but the invite-prompt copy change to
`npx tonk join` waits for real publishing.

## Improvement loop (PR 3+)

`/loop /bench-iterate <scenario>` with the existing rules: one change per
iteration, task.md / rubric.md / persona frozen once baselines are recorded.
Candidate levers the loop is expected to reach for (not commitments):

- **Invite prompt copy** (`core.yaml`): `npx tonk` form, a pointer to
  `https://tonk.xyz/llms.txt`, one-line intent fork ("user gave you a goal →
  build it; no goal → interview them; `tonk guide agent` is the playbook").
- **`tonk guide agent`**: bake the agent-flows playbook into the binary.
  Source-of-truth flips: `rust/tonk-cli/src/guide-agent.md` becomes
  canonical and `tonk-site/llms/docs/agent-flows.md` mirrors it (update the
  llms README, which currently declares agent-flows site-original).
- **`tonk join` orientation output**: after a successful join, print the
  concept names/counts, view names, and suggested next commands — what an
  agent currently burns 3 turns discovering.
- **Claim discovery**: whatever `targeted-edit` baselines show agents
  flailing on — likely a `tonk find <text>` value/name search or richer
  `tonk concepts` output. Decided by evidence, not now.

## Sequencing

1. **PR 1** — three scenarios + harness changes (runner abstraction, prompt
   extraction, PATH sandbox, local-tarball npx, ask-user bridge); baseline
   runs (2–3 per scenario) recorded in `bench/README.md`.
2. **PR 2** — npm wrapper package + publish CI.
3. **PR 3+** — one bench-iterate improvement per PR, each with a
   before/after run pair.

## Out of scope

- Web-UI onboarding beyond the invite-prompt copy (create flow, templates).
- Multi-branch / multi-repo CLI surface.
- Interview persona variations / difficulty tiers (one persona first).

## Open questions (resolve at implementation time)

- Exact codex headless flags: JSON event schema, sandbox mode, working-dir
  flag, timeout interaction with coreutils `timeout`.
- npm package name availability (`tonk` vs a scope).
- Whether `cold-onboard`'s "useful change" should be fixed (rubric names it)
  or left to the agent — fixed is more judgeable; current lean: fixed, a
  small seeded ask embedded in the `data-page` slot of the prompt.
