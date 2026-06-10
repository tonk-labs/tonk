# bench — agent-episode benchmarks for the slide ↔ tonk-ui flow

## What this is

An automated benchmark that measures how well an agent drives the tonk system
through the `slide` CLI, judged by what actually renders in tonk-ui. Each run
spawns a fresh headless `claude -p` episode, no prior context, pointed at a
hermetic local stack and a temp slide vault. After the episode the harness
screenshots checkpoint URLs in the real browser, runs a second headless judge
session against a rubric, and writes a scored report. The score and its
attached friction items feed an improvement loop (`/loop /bench-iterate
from-scratch`) that fixes the highest-leverage problem each iteration.

## Scenarios

| Scenario | Task | Description |
|---|---|---|
| `smoke` | `scenarios/smoke/task.md` | Plumbing check — scripted known-good asserts, no episode spend |
| `artifact-conversion` | `scenarios/artifact-conversion/task.md` | Agent converts a fixture HTML/JS artifact into tonk concepts+views; judged against `reference.png` |
| `from-scratch` | `scenarios/from-scratch/task.md` | Agent builds a habit tracker from nothing through slide |

### Baseline measurements (2026-06-10)

| Scenario | Outcome | Top friction |
|---|---|---|
| smoke | 7/10 | scripted; no episode judge |
| artifact-conversion | 9/10 | 4 friction items |
| from-scratch | 3/10 | `view` concept not built-in on a fresh branch (see Known friction below) |

## Usage

```sh
# Full run: stack + episode + screenshots + judge + report
nix develop -c bench/bin/bench run <scenario>

# Plumbing check only (no claude spend)
nix develop -c bench/bin/bench run smoke --scripted

# Multiple runs (for variance on a specific change)
nix develop -c bench/bin/bench run <scenario> --runs N

# Trend over the last N runs (default 10)
nix develop -c bench/bin/bench report [N]

# Promote a run's screenshots to baselines for future visual diff
nix develop -c bench/bin/bench baseline <scenario> <run-dir>
```

Run artifacts land in `bench/runs/<timestamp>-<n>-<scenario>/` (gitignored):
`transcript.jsonl`, `shots/`, `metrics.json`, `judge.json`, `scores.json`,
`report.md`, `visual-diff.json`.

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
- Slide vault: `$RUN_DIR/site`, fresh per run. Remote `origin` points at
  `$BENCH_URL/ucan/`.

**Episode** — headless `claude -p`, given only the scenario task and a
workspace pointing at the local stack. Bounded by `EPISODE_TIMEOUT` (default
1200 s). Transcript captured as streaming JSON for metrics.

**Browser** — headless Chrome via chromedriver/WebDriver-over-curl
(`bench/bin/browser.sh`). The only UI interaction is the join-form submit in
`bridge.sh`: fill `wa-input[name="space-name"]`, call `form.requestSubmit()`,
poll until landed off `/join`.

**Checkpoints** (`scenarios/<name>/checkpoints` file):

- `home` → `$BENCH_URL/`
- `<path>` → `$BENCH_URL/space/$SPACE_NAME/<path>`
- `display:<view-name>` → resolved at capture time via `slide share display`;
  `shots.sh` queries the view's model concept and builds the display URL as
  `/space/$SPACE_NAME/<model>!tonk:view`

**Judge** — second headless `claude -p`, given rubric.md + checkpoint
screenshots (+ `reference.png` for artifact-conversion) + transcript. Returns
`{"outcome": N, "friction": [...], "notes": "..."}`. One retry on invalid JSON.

**Visual diff** — `imagemagick compare` pixel-diff % of shots against promoted
baselines in `bench/baselines/<scenario>/`. Informational; never failing — a
diff may be the improvement you just made.

**Metrics** — computed by `metrics.sh` from the transcript via `jq`:
wall-clock, tokens, tool calls, failed tool results, repeated commands. Never
from the judge.

**Index** — `bench/runs/index.jsonl`, one line per run with key metrics.
`bench report` reads it for trend display.

## Requirements

- Repo devshell: `nix develop` provides caddy, trunk, jq, imagemagick.
- `chromedriver` via `$CHROMEDRIVER` env var (set in the devshell).
- Chrome at `/Applications/Google Chrome.app` (macOS default).
- `claude` CLI on PATH.
- `ANTHROPIC_API_KEY` — `op://` references are resolved via `op read` before
  launching the headless claude subprocesses; requires an unlocked 1Password
  session.

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

**`view` concept not seeded on fresh branches** (top item as of 2026-06-10,
from-scratch score 3/10): a fresh slide branch has no built-in `view` concept,
so any `view!:` assertion fails immediately. The agent wastes turns probing,
greps prior runs for a definition, and the copied definition may not register
correctly. Fix: seed `tonk:view` on fresh branches, or document its canonical
definition in `slide guide views` so the guide's examples work out of the box.

**Route shapes** (post-#488): `/space/:space/concept/:source` renders
`TonkConceptView`; `/space/:space/*subject` (`display:<model>!tonk:view`) renders
`TonkDisplay`. The `*subject` wildcard is defined after the chromed parent route
so it doesn't shadow `concept`/`view`/`layout`/`board` — Leptos 0.8 matches in
definition order.

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
anchored in slide. A bare field assertion without an anchor creates an
unresolvable row. The annotation `this:` for uniqueness via `FieldValue::Nested`
is not supported by the analyzer; use a content attribute instead.

**`--max-turns` unsupported** — the installed `claude` CLI has no `--max-turns`
flag. Episodes are bounded by `EPISODE_TIMEOUT` via `timeout(1)` from coreutils.
