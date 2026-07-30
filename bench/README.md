# bench — agent-episode benchmarks for the tonk ↔ tonk-ui flow

## What this is

An automated benchmark that measures how well an agent drives the tonk system
through the `tonk` CLI, judged by what actually renders in tonk-ui. Each run
spawns a fresh headless `claude -p` episode, no prior context, pointed at a
hermetic local stack and a temp tonk vault. After the episode the harness
screenshots checkpoint URLs in the real browser, runs a second headless judge
session against a rubric, and writes a scored report. The score and its
attached friction items feed an improvement loop (`/loop /bench-iterate
from-scratch`) that fixes the highest-leverage problem each iteration.

## Scenarios

| Scenario | Task | Description |
|---|---|---|
| `smoke` | `scenarios/smoke/task.md` | Plumbing check — scripted known-good asserts, no episode spend |
| `artifact-conversion` | `scenarios/artifact-conversion/task.md` | Agent converts a fixture HTML/JS artifact into tonk concepts+views; judged against `reference.png` |
| `from-scratch` | `scenarios/from-scratch/task.md` | Agent builds a habit tracker from nothing through tonk |
| `wiki-conversion` | `scenarios/wiki-conversion/task.md` | Agent converts the Grove wiki (page tree, block canvas, wikilinks, comments) into tonk concepts+views+components; judged against `reference.png` |
| `first-use` | `scenarios/first-use/task.md` | No CLI hints: measure the first successful live read and precise write |
| `targeted-edit` | `scenarios/targeted-edit/task.md` | Returning agent makes one precise change to seeded data |
| `interview-build` | `scenarios/interview-build/task.md` | Agent interviews a simulated user, then builds the result |
| `cold-onboard` | live invite prompt | Agent installs via npx, joins, orients, and builds |

### Baseline measurements (2026-06-10)

| Scenario | Outcome | Top friction |
|---|---|---|
| smoke | 7/10 | scripted; no episode judge |
| artifact-conversion | 9/10 | 4 friction items |
| from-scratch | 3/10 | `view` concept not built-in on a fresh branch (see Known friction below) |

### Baseline measurements (2026-07-08, codex/gpt-5.5 episodes)

The three agent-onboarding scenarios, run on the real codex/gpt-5.5 episode
runner. These are the "before" set for the agent-ergonomic-CLI work.

| Scenario | Outcome | Top friction |
|---|---|---|
| targeted-edit | 9/10 | reads the full guide + notation guide for a one-line edit (DSL learn-tax) |
| interview-build | 3/10 | strong interview, but the build never surfaces on the space home (render-gap) + DSL notation-validation rejections |
| cold-onboard | 7/10 | invite prompt gives no install hint; agent probes/filesystem-hunts for the CLI |

### Post-rename measurements (2026-07-13, codex/gpt-5.5 episodes)

Re-run of the two episode scenarios after the CLI data verbs went
dialog-native (`assert`/`retract`/`query`/`schema` replacing
`add`/`set`/`rm`/`list`/`describe`). Neither episode attempted an old
verb; both oriented via `tonk schema`/`tonk concepts` and drove every
write through `tonk eval` notation, accepted first try — the 07-08
notation-validation rejections did not recur.

| Scenario | Outcome | Top friction |
|---|---|---|
| targeted-edit | 9/10 | 12 commands of filesystem grepping for store-resident data before settling on the schema/eval path |
| interview-build | 3/10 | strong interview and clean notation, but the render-gap again: `tonk/space` home alias never repointed, so the build never surfaces on the space home |

### Post-authoring-verbs measurement (2026-07-13, codex/gpt-5.5 episode)

Re-run of interview-build after the authoring verbs landed (`tonk concept
add`, `tonk view add` with auto-surface, `tonk home` re-pointing the space
alias via the verified root-concept recipe).

| Scenario | Outcome | Top friction |
|---|---|---|
| interview-build | 3/10 | render-gap persists as a *discovery* problem: the episode issued zero `tonk home`/`tonk view add`/`tonk concept add` commands — it drove every write through `tonk eval` heredocs and never learned the alias-repointing verb exists, so the home screenshot still shows the empty launcher |

The capability gap was closed (the verbs exist and are verified end to end in
`rust/tonk-cli/tests/authoring.rs`) but the *orientation surfaces the episode
actually reads* — the invite prompt and `tonk guide` — still taught eval-first
and never mentioned `tonk home`.

### Post-discovery-fix measurement (2026-07-13, codex/gpt-5.5 episode)

One change per the improvement loop: the guide index's loop now leads with
the argument verbs and ends the build at the space home, and the
agent-invite prompt names the four-verb build path (`concept add` →
`assert` → `view add` → `home`).

| Scenario | Outcome | Top friction |
|---|---|---|
| interview-build | 9/10 | modeling back-reference cycles took two dry-run retries; a superseded anonymous view left a stale template live until hunted down and retracted |

The episode used the verbs heavily (8x `concept add`, 8x `view add`, 11x
`tonk home`, 11x `assert`, with `eval` for rules/effects) and the space-home
screenshot shows the full built dashboard — the render gap that capped this
scenario at 3/10 across three baselines is closed. Remaining blemishes for
follow-up: the "No view for <concept>; showing the default" banner over an
otherwise-correct render (the known reactor per-item-view fallback), a
dark-on-dark hero style against the shell theme, and `tonk assert --help`
(bare, no concept) rejecting the flag.

`cold-onboard` was 7/10 both before and after the seed-reaches-remote fix, but
the fix removed a confound: pre-fix the joined branch was barren (0
`xyz.tonk.view` attributes) because `tonk invite` never pushed the
`tonk init`-seeded standard library to the remote, so the agent had to
hand-author `tonk:view`. Post-fix the joined branch carries the full stdlib (19
`xyz.tonk.view` attributes), so the remaining 7/10 reflects real onboarding
friction (no install hint, orientation) rather than the harness artifact.

## Usage

```sh
# Full run: stack + episode + screenshots + judge + report
nix develop -c bench/bin/bench run <scenario>

# Plumbing check only (no claude spend)
nix develop -c bench/bin/bench run smoke --scripted

# Multiple runs (for variance on a specific change)
nix develop -c bench/bin/bench run <scenario> --runs N --variant <name>

# Trend over the last N runs (default 10)
nix develop -c bench/bin/bench report [N]

# Compare two frozen experiment arms
nix develop -c bench/bin/bench compare <scenario> <baseline> <treatment>

# Promote a run's screenshots to baselines for future visual diff
nix develop -c bench/bin/bench baseline <scenario> <run-dir>
```

Run artifacts land in
`bench/runs/<timestamp>-<n>-<scenario>-<variant>/` (gitignored):
`experiment.json`, `episode.jsonl`, `shots/`, `metrics.json`, `judge.json`,
`scores.json`, `report.md`, and `visual-diff.json`. The experiment protocol and
graduation gate live in [`EXPERIMENTS.md`](EXPERIMENTS.md).

### Improvement loop

```sh
/loop /bench-iterate from-scratch
```

Skill at `.claude/skills/bench-iterate/SKILL.md`. Each iteration: run the
scenario → read the report → pick the single top friction item → fix it →
rerun → compare → commit if improved. Hard rules: one change per iteration;
task.md/rubric.md are frozen.

## Architecture

**Local stack** — hermetic per-run:

- `tonk-access-local` binary: native access service with in-process S3
  (`LocalS3`), binds to a random port, writes `ACCESS_SERVICE_URL=...` to its
  log on startup.
- Caddy: serves `rust/tonk-ui/dist` (trunk-built) with `/ucan/*` proxied to
  the access service — same-origin layout as production.
- Tonk spot: registered by `site.sh setup` as `tonk spot new "$TONK_SPOT"
  --site "$RUN_DIR/site"`, fresh per run. Remote `origin` points at
  `$BENCH_URL/ucan/`.

**Spot pinning** — the CLI resolves a spot by `--spot`, then `TONK_SPOT`, then
a directory attachment (`tonk use <name> --here`), then the `tonk use`
selection. `cd`-ing into the site directory does nothing on its own — only an
explicit attachment makes a directory mean anything — and an unpinned `tonk`
call succeeds against whatever spot the developer happens to have selected
globally, silently, against the wrong repo. `TONK_SPOT` outranks attachments
precisely so the harness stays authoritative over whatever the developer has
bound locally. `run.sh` therefore exports both:

- `TONK_SPOTS_STATE="$RUN_DIR/spots-state"` — the registry and its canonical
  `spots/` root, so runs never see each other or the developer's own spots.
- `TONK_SPOT=bench` — the spot every harness and scenario `tonk` call resolves.

`episode.sh` passes both into the agent's environment too, overridable per
scenario as `EPISODE_SPOT` / `EPISODE_SPOTS_STATE`. A scenario where the agent
registers its own spot (cold-onboard joins from an invite) sets both — an empty
spot, which the CLI reads as unset, and a separate empty registry, so a
pre-join `tonk` reports no spots rather than silently resolving the origin site
the agent is supposed to be joining. `tonk join` selects the spot it registers,
so the agent's own governs from there.

**Episode** — headless `claude -p`, given only the scenario task and a
workspace pointing at the local stack. Bounded by `EPISODE_TIMEOUT` (default
1200 s). Transcript captured as streaming JSON for metrics.

**Browser** — headless Chrome via chromedriver/WebDriver-over-curl
(`bench/bin/browser.sh`). The only UI interaction is navigating to the minted
invite in `bridge.sh`: the join component auto-joins (claims the invite and
JS-navigates to `/space/<repository-DID>` with no form), so the bridge just
polls until the URL lands off `/join`, then fires the data pull.

**Checkpoints** (`scenarios/<name>/checkpoints` file):

- `home` → `$BENCH_URL/`
- `<path>` → `$BENCH_URL/space/$SPACE_NAME/<path>`
- `display:<view-name>` → resolved at capture time from the view's name, after
  a `tonk push`; `shots.sh` queries the view's model concept and builds the
  display URL as the model's directory route, `/space/$SPACE_NAME/<model>`

**Judge** — second headless `claude -p`, given rubric.md + checkpoint
screenshots (+ `reference.png` for artifact-conversion) + transcript. Returns
`{"outcome": N, "friction": [...], "notes": "..."}`. One retry on invalid JSON.

**Visual diff** — `imagemagick compare` pixel-diff % of shots against promoted
baselines in `bench/baselines/<scenario>/`. Informational; never failing — a
diff may be the improvement you just made.

**Metrics** — computed by `metrics.sh` from the transcript via `jq`:
wall-clock, tokens, tool calls, failed tool results, repeated commands, and the
command index of the first successful Tonk call, live-state read, instance-data
read, and content write. It also counts Tonk failures, orientation calls,
dry-runs, and broad command classes. Never from the judge.

**Index** — `bench/runs/index.jsonl`, one line per run with key metrics.
`bench report` reads it for trend display; `bench compare` applies the
pre-registered significance and correctness gate to two labelled variants.

## Requirements

- Repo devshell: `nix develop` provides caddy, trunk, jq, imagemagick, GNU
  coreutils `timeout`.
- `chromedriver` via `$CHROMEDRIVER` env var (set in the devshell).
- Chrome at `/Applications/Google Chrome.app` (macOS default).
- `claude` CLI on PATH, logged in. Episodes and the judge default to the
  CLI's OAuth session (`ANTHROPIC_API_KEY` is stripped from their env). Set
  `BENCH_USE_API_KEY=1` to bill against the API key instead; `op://`
  references are then resolved via `op read`, which requires an unlocked
  1Password session.
- `tonk` release binary — built automatically by `stack.sh start` via
  `cargo build --release -p tonk-cli` (no-ops when already up to date).

## Interpreting reports

**Outcome** (0–10): does the rendered UI achieve the task goal? For
artifact-conversion, how faithful is it to the reference screenshot? Rubrics
are per-scenario in `scenarios/<name>/rubric.md`.

**Friction items**: qualitative, with transcript evidence and a `suggested_fix`.
These are the improvement queue — prefer items with (a) evidence across
multiple runs, (b) failed tool results or repeated commands, (c) a concrete
fix.

**Visual diffs**: pixel-level change from the promoted baseline. Small drifts
(< 2%) are usually font rendering or timestamp variation. Large diffs may
indicate a regression or a real improvement — look at the shots side by side.

## Known friction

**tonk-created concepts have no Name claim** — `<tonk-display>`'s bare
`{model}/` directory route resolves names via the Name concept
(`dialog.name/referent`); concepts asserted through tonk only carry
`dialog.meta/name`, so name-addressed directory URLs may report "no concept
matched".

**`view` concept not seeded on fresh branches** (top item as of 2026-06-10,
from-scratch score 3/10): a fresh tonk branch has no built-in `view` concept,
so any `view!:` assertion fails immediately. The agent wastes turns probing,
greps prior runs for a definition, and the copied definition may not register
correctly. Fix: seed `tonk:view` on fresh branches, or document its canonical
definition in `tonk guide views` so the guide's examples work out of the box.

**Route shapes**: routing is data, not Leptos code — the service worker builds
a matchit router from the branch's seeded `route!` table
(`rust/tonk-core/assets/library/core.yaml`), which has four patterns:

```
/
/{*entity}@{*model}!{*view}
/{*entity}@{*model}
/{*model}
```

Checkpoint lines therefore use the directory form (`/{*model}`), which
`shots.sh` builds by resolving `display:<view-name>` to the view's model
concept. `!` only means something after an `@entity` segment, so the old bang
form (`note!tonk:view`) falls through to `/{*model}` with the whole literal
string as the model name and matches no concept — it renders a red "Model not
found" box. It was removed from every scenario's `checkpoints` file.

**Sync mechanism** — 20 s background tick in `sync_controller.rs`
(`TICK_INTERVAL_MS`). `bridge.sh` fires an explicit
`POST /api/repository/{name}/branch/main/sync/pull` and polls until confirmed,
so shots don't race the tick. The pull endpoint is handled by the service
worker, not Caddy.

**eval-async for SW-intercepted fetches** — synchronous XHR doesn't route
through async SW fetch handlers. `bridge.sh` uses WebDriver `execute/async` (the
`eval-async` command in `browser.sh`) for the pull poll; synchronous eval would
return before the SW response arrives.

**Notation anchors** — `attribute!:` and `concept!:` are how notation rows are
anchored in tonk. A bare field assertion without an anchor creates an
unresolvable row. The annotation `this:` for uniqueness via `FieldValue::Nested`
is not supported by the analyzer; use a content attribute instead.

**`--max-turns` unsupported** — the installed `claude` CLI has no `--max-turns`
flag. Episodes are bounded by `EPISODE_TIMEOUT` via `timeout(1)` from coreutils.

**`SPACE_NAME` is the repository DID** — the tonk-ui addresses a space by the
repository's subject DID (`did:key:…`), which the auto-join flow returns as
`repository.name` and the route parser reconstructs from the URL segment
(`did:key` is a droppable label; the id is re-prefixed). `site.sh setup`
captures the DID from `tonk spot new` into `$RUN_DIR/space.did`, and `run.sh`
exports it as `SPACE_NAME` for both the bridge pull
(`/api/repository/<DID>/…`) and the checkpoint shot URLs
(`/space/<DID>/…`). A hardcoded name like `bench` resolves to
`did:key:bench`, which doesn't exist — every view then 404s with
`Credential not found: key/self`. (Historical: the harness used to fill a
join form with a chosen name; the product removed that form in favor of
auto-join, so the harness now follows the DID.)
