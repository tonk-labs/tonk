# Agent-Onboarding Bench Implementation Plan (PR 1)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Three new bench scenarios (cold-onboard, targeted-edit, interview-build) with a pluggable codex/gpt-5.5 episode runner, per-runner metrics/judge adapters, a hermetic local npm registry for `npx tonk`, core.yaml prompt extraction, an ask-user persona bridge, and recorded baselines.

**Architecture:** Everything lands in `bench/` as additions to the existing bash harness. `run.sh` gains two scenario hooks (`scenario.env`, `prepare.sh`); `episode.sh` gains a runner switch and a generalized episode dir/prompt; `metrics.sh` and `judge.sh` gain a codex-JSONL branch; a static npm registry is served by the existing Caddy. Scenarios are data (task/rubric/checkpoints/persona/seed) plus their two hooks.

**Tech Stack:** bash, jq, awk/sed, Caddy (already in devshell), npm/npx (system), `codex` CLI (`codex exec --json -m gpt-5.5`), `claude` CLI (judge + persona), `tonk` release binary.

**Spec:** `docs/superpowers/specs/2026-07-07-agent-onboarding-bench-design.md`

## Global Constraints

- VCS is jj (colocated). Commit with `jj commit -m "..."` — no `git add`. Conventional Commits, scope `bench` unless noted. The `feat/agent-build` bookmark is moved once, in the final task.
- Codex episodes default to model `gpt-5.5` (`CODEX_MODEL` overrides). Judge and persona stay on headless `claude`.
- `--scripted` runs must stay LLM-spend-free (exception documented in Task 9).
- `bench/scenarios/<name>/task.md`, `rubric.md`, and `persona.md` are frozen after Task 10's baselines are recorded.
- Verified codex JSONL event shapes (probed live, codex-cli 0.142.5):
  - `{"type":"thread.started","thread_id":"..."}`
  - `{"type":"item.completed","item":{"id":"...","type":"agent_message","text":"..."}}`
  - `{"type":"item.started"|"item.completed","item":{"id":"...","type":"command_execution","command":"<shell> -lc '...'","aggregated_output":"...","exit_code":0|null,"status":"in_progress"|"completed"|"failed"}}`
  - `{"type":"turn.completed","usage":{"input_tokens":N,"cached_input_tokens":N,"output_tokens":N,"reasoning_output_tokens":N}}`
- Codex runs commands through the user's login shell (`zsh -lc`), which re-sources profiles. On this machine tonk IS globally installed (`~/.cargo/bin/tonk`, via `~/.zprofile`'s cargo env), so PATH-based "tonk absent" sandboxing is impossible with the real `$HOME`. AMENDED (post-Task-2 probe): episode.sh supports `EPISODE_HOME` — when set, the episode runs with `HOME=$EPISODE_HOME` (and `CODEX_HOME` pointed back at the real `~/.codex` for auth), so user rc files (`~/.zprofile` → cargo/homebrew paths) never load, `~`-writes land inside the run dir, and the sandbox guard checks reachability under that HOME instead of aborting on the host install.
- AMENDED (post-Task-2 probe): the tonk CLI keeps its profile in the platform profile dir (`~/Library/Application Support/dialog` on macOS, `Directory::Profile` in rust/tonk-cli/src/site.rs). Codex's `workspace-write` sandbox denies it, making every profile-touching tonk command fail (`failed to build operator`, exit 4). run_codex must `--add-dir "$HOME/Library/Application Support/dialog"` (mkdir -p first; with `EPISODE_HOME` set this resolves inside the run dir automatically).
- Asserted-notation update idiom (query-bind then assert; confirmed against this binary):

  ```
  habit:
    this: ?h
    name: "Inbox zero"

  habit!:
    this: ?h
    name: "Inbox Zero — daily"
  ```

- All bench scripts follow the house style: `#!/usr/bin/env bash`, `set -euo pipefail`, `VAR="${VAR:?}"` guards, progress to stderr.

---

### Task 1: Scenario hooks + episode generalization

**Files:**
- Modify: `bench/bin/run.sh`
- Modify: `bench/bin/episode.sh`

**Interfaces:**
- Produces (consumed by every later task):
  - `run.sh` sources optional `$SCENARIO/scenario.env` (with `set -a`) after exporting `RUN_DIR`/`BENCH_URL`/`SPACE_NAME`, then runs optional executable `$SCENARIO/prepare.sh` after `site.sh setup`.
  - Env vars honored by `episode.sh`: `EPISODE_DIR` (default `$RUN_DIR/site`), `EPISODE_BIN` (dir prepended to PATH), `EPISODE_PATH_SANDBOX=1` (omit `target/release` from PATH + guard that `tonk` is not reachable), `EPISODE_RUNNER` (Task 2).
  - Prompt file: `$RUN_DIR/prompt.md` if present, else `$SCENARIO/task.md`.
  - Scripted mode runs in `$EPISODE_DIR`, not hardcoded `$RUN_DIR/site`.

- [ ] **Step 1: Add hook plumbing to run.sh**

In `bench/bin/run.sh`, replace the block from `"$ROOT/bench/bin/stack.sh" start` through the `episode_status` assignment with:

```bash
  "$ROOT/bench/bin/stack.sh" start
  "$ROOT/bench/bin/site.sh" setup

  # The space is addressed by the repository's DID (auto-join returns it
  # as repository.name); site.sh setup stashed it in space.did. Both the
  # bridge pull and the checkpoint shots must use this, not a name.
  if [ -s "$RUN_DIR/space.did" ]; then
    SPACE_NAME="$(cat "$RUN_DIR/space.did")"
  else
    echo "run: no space.did from site setup; falling back to SPACE_NAME=${SPACE_NAME:-bench}" >&2
    SPACE_NAME="${SPACE_NAME:-bench}"
  fi
  export SPACE_NAME
  echo "run: space addressed as $SPACE_NAME" >&2

  # Scenario hooks: scenario.env exports episode knobs (runner, dir,
  # sandbox); prepare.sh does scenario-specific setup (seed data, mint
  # an invite, build a registry or a prompt). Both are optional.
  if [ -f "$SCENARIO/scenario.env" ]; then
    set -a
    # shellcheck disable=SC1091
    . "$SCENARIO/scenario.env"
    set +a
  fi
  if [ -x "$SCENARIO/prepare.sh" ]; then
    "$SCENARIO/prepare.sh"
  fi

  EPISODE_DIR="${EPISODE_DIR:-$RUN_DIR/site}"
  export EPISODE_DIR

  episode_status=0
  if [ "$SCRIPTED" = 1 ]; then
    ( cd "$EPISODE_DIR" && TONK="$ROOT/target/release/tonk" bash "$SCENARIO/scripted.sh" ) \
      || episode_status=$?
  else
    "$ROOT/bench/bin/episode.sh" || episode_status=$?
  fi
```

Note the existing `SPACE_NAME` block stays as is — only the hook lines and `EPISODE_DIR` are new, and the scripted `cd` changes from `"$RUN_DIR/site"` to `"$EPISODE_DIR"`.

One subtlety: `EPISODE_DIR` may leak between `--runs N` iterations since scenario.env is sourced inside the loop but the default expansion `${EPISODE_DIR:-...}` would then see the previous iteration's value. Add `unset EPISODE_DIR EPISODE_BIN EPISODE_PATH_SANDBOX EPISODE_RUNNER EPISODE_SANDBOX` at the TOP of the loop body (right after `RUN_DIR=` is computed).

- [ ] **Step 2: Generalize episode.sh (dir, prompt, PATH)**

Replace the guard, fixtures block, and the `( cd "$SITE" ... )` invocation in `bench/bin/episode.sh` so the file reads:

```bash
#!/usr/bin/env bash
# Run the episode agent: a fresh headless agent in the episode dir with
# the scenario task. It gets tonk on PATH (unless the scenario sandboxes
# it away) and nothing else from this repo — its struggles with the CLI
# are the benchmark signal.
#
# Env: ROOT, RUN_DIR, SCENARIO
# Optional (from scenario.env): EPISODE_DIR, EPISODE_BIN,
#   EPISODE_PATH_SANDBOX, EPISODE_RUNNER, EPISODE_SANDBOX, CODEX_MODEL
set -euo pipefail

ROOT="${ROOT:?}"; RUN_DIR="${RUN_DIR:?}"; SCENARIO="${SCENARIO:?}"
EPISODE_DIR="${EPISODE_DIR:-$RUN_DIR/site}"
EPISODE_TIMEOUT="${EPISODE_TIMEOUT:-1200}"   # seconds
EPISODE_RUNNER="${EPISODE_RUNNER:-claude}"

# The prompt is the generated $RUN_DIR/prompt.md when a prepare hook
# built one (cold-onboard renders the live core.yaml invite copy);
# otherwise the scenario's static task.md.
if [ -f "$RUN_DIR/prompt.md" ]; then
  PROMPT_FILE="$RUN_DIR/prompt.md"
else
  PROMPT_FILE="$SCENARIO/task.md"
fi
[ -f "$PROMPT_FILE" ] || { echo "episode: missing prompt ($PROMPT_FILE)" >&2; exit 1; }

mkdir -p "$EPISODE_DIR"

# Fixtures are the episode's working material (e.g. artifact.html).
if [ -d "$SCENARIO/fixtures" ]; then
  cp -R "$SCENARIO/fixtures/." "$EPISODE_DIR/"
fi

# Episode PATH. Default: tonk release binary available. Sandbox mode:
# tonk deliberately absent — the episode must discover the install
# path itself (npx against the run's local registry). Codex commands
# run through a login shell that re-sources system paths, so the
# sandbox holds only if tonk isn't globally installed: guard it.
EPISODE_PATH="$PATH"
if [ "${EPISODE_PATH_SANDBOX:-0}" = 1 ]; then
  if command -v tonk >/dev/null 2>&1; then
    echo "episode: EPISODE_PATH_SANDBOX=1 but 'tonk' is globally installed at $(command -v tonk); the cold-start scenario would be invalid. Uninstall it or drop the sandbox." >&2
    exit 1
  fi
else
  EPISODE_PATH="$ROOT/target/release:$EPISODE_PATH"
fi
if [ -n "${EPISODE_BIN:-}" ]; then
  EPISODE_PATH="$EPISODE_BIN:$EPISODE_PATH"
fi

date +%s > "$RUN_DIR/episode-start"

# Write episode-end on any exit so an interrupted run still leaves a pair.
trap 'date +%s > "$RUN_DIR/episode-end"' EXIT

# Auth: default to the claude CLI's logged-in (OAuth) session by
# stripping ANTHROPIC_API_KEY from the child env. Set
# BENCH_USE_API_KEY=1 to use the API key instead; an op:// reference
# is resolved via `op read` (headless claude can't reach the op-agent
# the way the interactive shell does). Codex episodes authenticate via
# ~/.codex (run `codex login` once); the strip is harmless there.
KEY_ENV=(-u ANTHROPIC_API_KEY)
if [ -n "${BENCH_USE_API_KEY:-}" ]; then
  RESOLVED_KEY="${ANTHROPIC_API_KEY:-}"
  if [[ "$RESOLVED_KEY" == op://* ]]; then
    RESOLVED_KEY="$(op read "$RESOLVED_KEY")"
  fi
  KEY_ENV=(ANTHROPIC_API_KEY="$RESOLVED_KEY")
fi

run_claude() {
  ( cd "$EPISODE_DIR" && \
    env "${KEY_ENV[@]}" \
    PATH="$EPISODE_PATH" \
    timeout -k 30 "$EPISODE_TIMEOUT" claude -p "$(cat "$PROMPT_FILE")" \
      --output-format stream-json --verbose \
      --allowedTools "Bash,Read,Write,Edit,Glob,Grep" \
  ) > "$RUN_DIR/episode.jsonl" 2> "$RUN_DIR/episode.stderr"
}

run_codex() {
  ( cd "$EPISODE_DIR" && \
    env "${KEY_ENV[@]}" \
    PATH="$EPISODE_PATH" \
    timeout -k 30 "$EPISODE_TIMEOUT" codex exec --json \
      -m "${CODEX_MODEL:-gpt-5.5}" \
      --skip-git-repo-check --ephemeral \
      -s "${EPISODE_SANDBOX:-workspace-write}" \
      -c sandbox_workspace_write.network_access=true \
      --add-dir "$RUN_DIR" \
      - < "$PROMPT_FILE" \
  ) > "$RUN_DIR/episode.jsonl" 2> "$RUN_DIR/episode.stderr"
}

set +e
case "$EPISODE_RUNNER" in
  claude) run_claude ;;
  codex)  run_codex ;;
  *) echo "episode: unknown EPISODE_RUNNER '$EPISODE_RUNNER'" >&2; exit 2 ;;
esac
status=$?
set -e
echo "episode: exit $status (runner: $EPISODE_RUNNER)" >&2
exit "$status"
```

Two deliberate changes beyond the switch: (a) `--allowedTools "Bash,..."` replaces `"Bash(tonk:*),..."` — cold-onboard needs `npx`/`mkdir`, and codex has no tool allowlist at all, so the claude runner keeping one would skew runner comparisons; (b) fixtures copy into `$EPISODE_DIR`.

- [ ] **Step 3: Verify plumbing with the scripted smoke scenario**

Run: `cd /Users/jackdouglas/tonk/tonk && nix develop -c bench/bin/bench run smoke --scripted`
Expected: run completes as before (report.md written; smoke's scripted.sh executed inside `$RUN_DIR/site`). If `bench/bin/bench` is a wrapper, inspect it first (`cat bench/bin/bench`) and use its calling convention.

- [ ] **Step 4: Commit**

```bash
jj commit -m "feat(bench): scenario env/prepare hooks and generalized episode dir/prompt/PATH"
```

---

### Task 2: Codex runner probe

The runner switch was written in Task 1; this task proves the codex path produces a usable transcript before scenarios depend on it.

**Files:**
- Create: `bench/scenarios/dev-probe/task.md` (temporary, deleted in this task)

**Interfaces:**
- Consumes: `EPISODE_RUNNER=codex` branch from Task 1.
- Produces: confidence + a real `episode.jsonl` fixture source for Task 3.

- [ ] **Step 1: Create a minimal probe scenario**

Write `bench/scenarios/dev-probe/task.md`:

```markdown
Run `tonk --help`, then `tonk status`, then reply with one sentence
describing what this directory is. Do not change anything.
```

Write `bench/scenarios/dev-probe/checkpoints` containing a single line `home`, and `bench/scenarios/dev-probe/rubric.md` containing `Probe scenario — not judged.`

- [ ] **Step 2: Run the episode standalone (no full run.sh: cheap)**

```bash
cd /Users/jackdouglas/tonk/tonk
export ROOT="$PWD" RUN_DIR="$PWD/bench/runs/dev-probe-$(date +%s)" SCENARIO="$PWD/bench/scenarios/dev-probe"
mkdir -p "$RUN_DIR/site" && (cd "$RUN_DIR/site" && "$ROOT/target/release/tonk" init)
EPISODE_RUNNER=codex EPISODE_TIMEOUT=300 bench/bin/episode.sh; echo "exit=$?"
head -3 "$RUN_DIR/episode.jsonl"
jq -r 'select(.type=="item.completed") | .item.type' "$RUN_DIR/episode.jsonl" | sort | uniq -c
```

Expected: exit=0; first line `{"type":"thread.started",...}`; item types include `command_execution` and `agent_message`. (Requires `codex login` done and `cargo build --release -p tonk-cli` current — run `nix develop -c cargo build --release -p tonk-cli` first if `target/release/tonk` is missing.)

- [ ] **Step 3: Keep the transcript, delete the probe scenario**

```bash
cp "$RUN_DIR/episode.jsonl" bench/testdata/codex-episode.jsonl  # mkdir -p bench/testdata first
rm -rf bench/scenarios/dev-probe
```

Trim `bench/testdata/codex-episode.jsonl` by hand if huge (keep thread.started, a few command_execution item.completed pairs including at least one with `"status":"failed"` — if none failed, edit one item's `exit_code` to `1` and `status` to `"failed"` so the fixture covers the branch — plus agent_message and turn.completed).

- [ ] **Step 4: Commit**

```bash
jj commit -m "test(bench): codex episode transcript fixture from live probe"
```

---

### Task 3: Metrics adapter (codex format + journey metrics)

**Files:**
- Modify: `bench/bin/metrics.sh`
- Create: `bench/testdata/claude-episode.jsonl`
- Modify: `docs/superpowers/specs/2026-07-07-agent-onboarding-bench-design.md` (metrics paragraph)

**Interfaces:**
- Consumes: `bench/testdata/codex-episode.jsonl` (Task 2).
- Produces: `metrics.json` keeps its existing top-level shape for both runners and gains a `journey` object: `{cmds_before_join: N|null, cmds_before_first_eval: N|null, doc_fetches: N, ask_user_calls: N}`. Codex runs have `duration_ms: null` and `num_turns` = count of `turn.completed` events.

- [ ] **Step 1: Create the claude fixture**

Write `bench/testdata/claude-episode.jsonl` — a minimal hand-built claude stream with: 2 assistant events carrying Bash `tool_use` (commands `tonk guide` and `tonk join 'http://x/join?access=A#B'`), 1 assistant Bash `tool_use` with `tonk eval -c 'x:'`, 1 user event with `tool_result` `is_error: true`, and a final `{"type":"result","num_turns":5,"duration_ms":1000,"usage":{"input_tokens":10,"output_tokens":20,"cache_read_input_tokens":5}}`. Model each line on the real events (crib exact envelope shapes from any existing `bench/runs/*/episode.jsonl`, e.g. `jq -c 'select(.type=="assistant")' bench/runs/20260703-160545-1-wiki-conversion/episode.jsonl | head -2`).

- [ ] **Step 2: Add format detection and the codex branch to metrics.sh**

After the empty-transcript early-exit in `bench/bin/metrics.sh`, add detection:

```bash
first_type="$(head -1 "$T" | jq -r '.type // empty')"
```

Define the journey filter once, shared by both branches, as a jq function prelude string prepended to each program (bash variable):

```bash
JOURNEY_DEF='
  def journey($cmds):
    ($cmds | to_entries) as $e |
    {
      cmds_before_join:        (($e | map(select(.value | test("tonk +join"))) | first | .key) // null),
      cmds_before_first_eval:  (($e | map(select(.value | test("tonk +eval"))) | first | .key) // null),
      doc_fetches:             ([$cmds[] | select(test("tonk +guide|llms(-full)?\\.txt"))] | length),
      ask_user_calls:          ([$cmds[] | select(test("(^|[ /;&|])ask-user( |$|\")"))] | length)
    };
'
```

Codex branch (when `first_type = "thread.started"`):

```bash
jq -s \
  --argjson wall "$wall" \
  --argjson exit_code "$exit_code" "$JOURNEY_DEF"'
  ([.[] | select(.type == "item.completed") | .item | select(.type == "command_execution")]) as $execs |
  ([$execs[].command]) as $cmds |
  ([.[] | select(.type == "turn.completed") | .usage] | map(select(. != null))) as $usages |
  {
    wall_seconds:         $wall,
    episode_exit:         $exit_code,
    num_turns:            ([.[] | select(.type == "turn.completed")] | length),
    duration_ms:          null,
    tokens: {
      input:              (if ($usages|length) > 0 then ($usages | map(.input_tokens // 0) | add) else null end),
      output:             (if ($usages|length) > 0 then ($usages | map(.output_tokens // 0) | add) else null end),
      cache_read:         (if ($usages|length) > 0 then ($usages | map(.cached_input_tokens // 0) | add) else null end)
    },
    tool_calls:           ($execs | length),
    bash_calls:           ($execs | length),
    failed_tool_results:  ([$execs[] | select(.status == "failed" or ((.exit_code // 0) != 0))] | length),
    repeated_commands:    ($cmds | group_by(.) | map(select(length > 1) | {command: .[0], times: length})),
    journey:              journey($cmds)
  }' "$T" > "$RUN_DIR/metrics.json"
```

Claude branch: the existing jq program, with `"$JOURNEY_DEF"` prepended and `, journey: journey($cmds)` added as the last object field. Also add `journey: {cmds_before_join: null, cmds_before_first_eval: null, doc_fetches: 0, ask_user_calls: 0}` to the zeroed empty-transcript object.

- [ ] **Step 3: Verify against both fixtures**

```bash
cd /Users/jackdouglas/tonk/tonk
for f in claude codex; do
  D="$(mktemp -d)"; cp "bench/testdata/$f-episode.jsonl" "$D/episode.jsonl"
  date +%s > "$D/episode-start"; date +%s > "$D/episode-end"
  RUN_DIR="$D" bench/bin/metrics.sh 2>/dev/null
  echo "--- $f"; jq '{tool_calls, failed_tool_results, tokens, journey}' "$D/metrics.json"
done
```

Expected: claude fixture → `tool_calls: 3`, `failed_tool_results: 1`, `journey.cmds_before_join: 1`, `journey.cmds_before_first_eval: 2`, `journey.doc_fetches: 1`; codex fixture → counts matching its command items, non-null summed tokens, `failed_tool_results ≥ 1`.

- [ ] **Step 4: Amend the spec's metrics wording**

In `docs/superpowers/specs/2026-07-07-agent-onboarding-bench-design.md`, replace the sentence naming `seconds_to_join` / `seconds_to_first_successful_eval` with: "Metrics (added to `metrics.sh` output as a `journey` object): `cmds_before_join`, `cmds_before_first_eval` (command-index proxies — neither runner's event stream carries per-event wall timestamps), `doc_fetches` (guide/llms.txt reads), `ask_user_calls`, plus the existing failed-tool-results / repeated-commands / wall-clock."

- [ ] **Step 5: Commit**

```bash
jj commit -m "feat(bench): codex metrics adapter and journey metrics"
```

---

### Task 4: Judge transcript adapter + interview log

**Files:**
- Modify: `bench/bin/judge.sh`

**Interfaces:**
- Consumes: codex fixture; `$RUN_DIR/interview.log` convention (written by Task 9's ask-user bridge, format `AGENT: ...` / `USER: ...` line pairs).
- Produces: `episode-summary.txt` correct for both formats; judge prompt includes the interview transcript when present.

- [ ] **Step 1: Branch the summary extraction**

In `bench/bin/judge.sh`, replace the summary block with:

```bash
if [ -s "$RUN_DIR/episode.jsonl" ]; then
  first_type="$(head -1 "$RUN_DIR/episode.jsonl" | jq -r '.type // empty')"
  if [ "$first_type" = "thread.started" ]; then
    # codex exec --json format
    jq -r '
      select(.type == "item.completed") | .item |
      if .type == "agent_message" then "AGENT: \(.text)"
      elif .type == "command_execution" then
        "RAN: \(.command)"
        + (if (.status == "failed" or ((.exit_code // 0) != 0))
           then "\nERROR: \(.aggregated_output | tostring | .[0:500])"
           else "" end)
      else empty end
    ' "$RUN_DIR/episode.jsonl" > "$RUN_DIR/episode-summary.txt"
  else
    # claude stream-json format (existing extraction, unchanged)
    ...existing two jq calls...
  fi
else
  : > "$RUN_DIR/episode-summary.txt"
fi
```

(Keep the existing claude jq programs verbatim inside the else branch.)

- [ ] **Step 2: Feed the interview log to the judge**

In the `prompt()` heredoc, after the `Transcript:` line, add:

```bash
$([ -s "$RUN_DIR/interview.log" ] && echo "Interview transcript (the agent's questions to the simulated user, and the replies): $RUN_DIR/interview.log — read it with the Read tool; interview quality is judged from it." || true)
```

- [ ] **Step 3: Verify extraction on the codex fixture**

```bash
D="$(mktemp -d)"; cp bench/testdata/codex-episode.jsonl "$D/episode.jsonl"
first_type="$(head -1 "$D/episode.jsonl" | jq -r '.type')"
# run just the extraction snippet by sourcing judge.sh is impractical; instead:
jq -r 'select(.type == "item.completed") | .item |
  if .type == "agent_message" then "AGENT: \(.text)"
  elif .type == "command_execution" then "RAN: \(.command)" + (if (.status == "failed" or ((.exit_code // 0) != 0)) then "\nERROR: \(.aggregated_output | tostring | .[0:500])" else "" end)
  else empty end' "$D/episode.jsonl"
```

Expected: alternating `AGENT:`/`RAN:` lines; the failed command followed by an `ERROR:` line.

- [ ] **Step 4: Commit**

```bash
jj commit -m "feat(bench): judge summary adapter for codex transcripts and interview log"
```

---

### Task 5: Hermetic npm registry for `npx tonk`

**Files:**
- Create: `bench/npm/tonk-wrapper/package.json`
- Create: `bench/npm/tonk-wrapper/bin/tonk.js`
- Create: `bench/bin/registry.sh`
- Modify: `bench/bin/stack.sh` (Caddyfile)

**Interfaces:**
- Produces: `registry.sh build` (env: ROOT, RUN_DIR, BENCH_URL) stages the wrapper + release binary, packs `$RUN_DIR/registry/tonk-0.0.0-bench.tgz`, and writes packument `$RUN_DIR/registry/tonk.json`. Caddy serves `GET $BENCH_URL/registry/tonk` (packument, application/json) and `GET $BENCH_URL/registry/tonk-0.0.0-bench.tgz`. An episode with `npm_config_registry=$BENCH_URL/registry/` and `npm_config_cache=$RUN_DIR/npm-cache` can run `npx --yes tonk <cmd>` offline.
- Note: this wrapper is bench-only scaffolding under `bench/npm/`; the real publishable package (platform `optionalDependencies`) is PR 2 and does not reuse it.

- [ ] **Step 1: Write the wrapper package**

`bench/npm/tonk-wrapper/package.json`:

```json
{
  "name": "tonk",
  "version": "0.0.0-bench",
  "description": "tonk CLI (bench-local wrapper around the release binary)",
  "license": "UNLICENSED",
  "bin": { "tonk": "bin/tonk.js" },
  "files": ["bin", "vendor"]
}
```

`bench/npm/tonk-wrapper/bin/tonk.js`:

```js
#!/usr/bin/env node
// Bench-local wrapper: exec the vendored release binary. The real
// published package (PR 2) resolves a platform binary package instead.
const { spawnSync } = require("node:child_process");
const path = require("node:path");
const bin = path.join(__dirname, "..", "vendor", "tonk");
const r = spawnSync(bin, process.argv.slice(2), { stdio: "inherit" });
if (r.error) { console.error(r.error.message); process.exit(1); }
process.exit(r.status === null ? 1 : r.status);
```

- [ ] **Step 2: Write registry.sh**

`bench/bin/registry.sh`:

```bash
#!/usr/bin/env bash
# Build the run's hermetic npm registry: stage the bench wrapper
# package with the vendored release binary, npm-pack it, and write a
# static packument so `npx tonk` resolves against
# $BENCH_URL/registry/ with no real-registry traffic.
#
# Env: ROOT, RUN_DIR, BENCH_URL
set -euo pipefail

ROOT="${ROOT:?}"; RUN_DIR="${RUN_DIR:?}"; BENCH_URL="${BENCH_URL:?}"
VERSION="0.0.0-bench"
STAGE="$RUN_DIR/npm-pkg"
REG="$RUN_DIR/registry"

build() {
  [ -x "$ROOT/target/release/tonk" ] || { echo "registry: no release tonk (build with cargo build --release -p tonk-cli)" >&2; exit 1; }
  rm -rf "$STAGE" "$REG"
  mkdir -p "$STAGE" "$REG"
  cp -R "$ROOT/bench/npm/tonk-wrapper/." "$STAGE/"
  mkdir -p "$STAGE/vendor"
  cp "$ROOT/target/release/tonk" "$STAGE/vendor/tonk"
  chmod +x "$STAGE/vendor/tonk" "$STAGE/bin/tonk.js"

  (cd "$STAGE" && npm pack --pack-destination "$REG" >/dev/null)
  TGZ="$REG/tonk-$VERSION.tgz"
  [ -f "$TGZ" ] || { echo "registry: npm pack produced no $TGZ" >&2; exit 1; }

  local sha1 sha512
  sha1="$(shasum -a 1 "$TGZ" | awk '{print $1}')"
  sha512="sha512-$(openssl dgst -sha512 -binary "$TGZ" | base64)"

  jq -n \
    --arg v "$VERSION" \
    --arg tarball "$BENCH_URL/registry/tonk-$VERSION.tgz" \
    --arg sha1 "$sha1" --arg sha512 "$sha512" '
  {
    name: "tonk",
    "dist-tags": { latest: $v },
    versions: {
      ($v): {
        name: "tonk", version: $v,
        bin: { tonk: "bin/tonk.js" },
        dist: { tarball: $tarball, shasum: $sha1, integrity: $sha512 }
      }
    }
  }' > "$REG/tonk.json"
  echo "registry: built at $REG (serve as $BENCH_URL/registry/)" >&2
}

case "${1:-}" in
  build) build ;;
  *) echo "usage: registry.sh build" >&2; exit 2 ;;
esac
```

Make it executable: `chmod +x bench/bin/registry.sh`.

- [ ] **Step 3: Serve it from Caddy**

In `bench/bin/stack.sh`'s Caddyfile heredoc, before the `handle /ucan/*` block, add:

```
  handle /registry/tonk {
    root * "$RUN_DIR/registry"
    rewrite * /tonk.json
    header Content-Type application/json
    file_server
  }
  handle /registry/* {
    uri strip_prefix /registry
    root * "$RUN_DIR/registry"
    file_server
  }
```

(The heredoc is unquoted `<<EOF`, so `$RUN_DIR` expands at write time — correct.)

- [ ] **Step 4: Verify end to end**

```bash
cd /Users/jackdouglas/tonk/tonk
export ROOT="$PWD" RUN_DIR="$PWD/bench/runs/dev-registry" BENCH_PORT=8787 BENCH_URL="http://127.0.0.1:8787"
mkdir -p "$RUN_DIR"
nix develop -c bash -c 'bench/bin/stack.sh start && bench/bin/registry.sh build'
curl -s "$BENCH_URL/registry/tonk" | jq -r '."dist-tags".latest'
T="$(mktemp -d)"
npm_config_registry="$BENCH_URL/registry/" npm_config_cache="$T/npm-cache" npx --yes tonk --help | head -3
nix develop -c bench/bin/stack.sh stop; rm -rf "$RUN_DIR" "$T"
```

Expected: `0.0.0-bench`, then tonk's usage/help header printed via npx. If npx balks at the packument (watch for `ETARGET`/integrity errors), fix the packument fields before proceeding — this must pass before Task 7.

- [ ] **Step 5: Commit**

```bash
jj commit -m "feat(bench): hermetic local npm registry so episodes can npx tonk"
```

---

### Task 6: Invite-prompt extraction from core.yaml

**Files:**
- Create: `bench/bin/prompt.sh`

**Interfaces:**
- Produces: `prompt.sh --invite-url <url> --name <label>` prints the filled agent prompt to stdout, extracted live from `rust/tonk-core/assets/library/core.yaml` (view `this: id:agent-invite/prompt`, the `<pre class="agent-prompt__pre">` body). Env: ROOT.

- [ ] **Step 1: Write prompt.sh**

```bash
#!/usr/bin/env bash
# Render the agent-invite prompt exactly as the product would: extract
# the <pre class="agent-prompt__pre"> body from core.yaml's
# id:agent-invite/prompt view and fill its template fields from a real
# minted invite. Copy edits in core.yaml are automatically under test —
# there is no frozen prompt.
#
# Usage: prompt.sh --invite-url <url> --name <label>
# Env: ROOT
set -euo pipefail

ROOT="${ROOT:?}"
CORE_YAML="$ROOT/rust/tonk-core/assets/library/core.yaml"
INVITE_URL="" NAME="bench"
while [ $# -gt 0 ]; do
  case "$1" in
    --invite-url) INVITE_URL="$2"; shift ;;
    --name) NAME="$2"; shift ;;
    *) echo "prompt: unknown flag $1" >&2; exit 2 ;;
  esac
  shift
done
[ -n "$INVITE_URL" ] || { echo "prompt: --invite-url required" >&2; exit 2; }

# Extract the <pre> body: start at the opening tag (dropping the tag
# itself), stop at </pre>. Then strip the 4-space YAML block indent and
# unescape the HTML entities the template carries.
raw="$(awk '
  /<pre class="agent-prompt__pre">/ { f=1; sub(/.*<pre class="agent-prompt__pre">/, ""); print; next }
  f && /<\/pre>/ { sub(/<\/pre>.*/, ""); print; exit }
  f { print }
' "$CORE_YAML" | sed -e 's/^    //' -e 's/&amp;/\&/g' -e 's/&quot;/"/g')"

[ -n "$raw" ] || { echo "prompt: extraction from $CORE_YAML came up empty — did the agent-prompt view template change shape?" >&2; exit 1; }

# Fill the template. The join URL is one composite placeholder; replace
# it whole with the real minted invite URL.
filled="$raw"
filled="${filled//\{dom.host\/data-base\}?access=\{access\}\{remote\}#\{code\}/$INVITE_URL}"
filled="${filled//\{name\}/$NAME}"
filled="${filled//\{dom.host\/data-page\}/this repo}"

# Self-check: no unfilled placeholders may survive; the join command
# must be present. Fail loudly — a silently wrong prompt poisons runs.
if printf '%s' "$filled" | grep -qE '\{(access|remote|code|name|dom\.host)' ; then
  echo "prompt: unfilled placeholder survived:" >&2
  printf '%s\n' "$filled" | grep -nE '\{' >&2
  exit 1
fi
printf '%s' "$filled" | grep -q "tonk join" || { echo "prompt: no 'tonk join' in rendered prompt" >&2; exit 1; }

printf '%s\n' "$filled"
```

Make it executable.

- [ ] **Step 2: Verify against the real core.yaml**

```bash
cd /Users/jackdouglas/tonk/tonk
ROOT="$PWD" bench/bin/prompt.sh --invite-url 'http://127.0.0.1:8787/join?access=FAKE#SEED' --name bench
```

Expected output (structure): starts `You're helping build the "bench" repo.`, contains the `mkdir -p ~/tonk/bench && cd ~/tonk/bench` line, contains `tonk join 'http://127.0.0.1:8787/join?access=FAKE#SEED'`, contains `tonk guide` and `tonk schema` lines, ends `Then build.` Exit 0. Also verify failure mode: `ROOT="$PWD" bench/bin/prompt.sh --invite-url x --name '{name}'` is NOT a required test, but temporarily breaking the awk pattern should exit 1 (spot-check once).

- [ ] **Step 3: Commit**

```bash
jj commit -m "feat(bench): render the live core.yaml agent-invite prompt for episodes"
```

---

### Task 7: cold-onboard scenario

**Files:**
- Create: `bench/scenarios/cold-onboard/scenario.env`
- Create: `bench/scenarios/cold-onboard/prepare.sh`
- Create: `bench/scenarios/cold-onboard/checkpoints`
- Create: `bench/scenarios/cold-onboard/rubric.md`
- Create: `bench/scenarios/cold-onboard/scripted.sh`
- Create: `bench/scenarios/cold-onboard/NOTES.md`
- Modify: `bench/bin/shots.sh` (add `space` checkpoint keyword)

**Interfaces:**
- Consumes: Tasks 1, 2, 5, 6. `prepare.sh` writes `$RUN_DIR/invite.url` and `$RUN_DIR/prompt.md`.
- Produces: a runnable scenario; `space` checkpoint keyword available to all scenarios.

- [ ] **Step 1: Add the `space` checkpoint keyword to shots.sh**

In the checkpoint loop in `bench/bin/shots.sh`, after the `home` case:

```bash
  elif [ "$line" = "space" ]; then
    url="$BENCH_URL/space/$SPACE_NAME/"
    name="$(printf '%02d-space' "$n")"
```

- [ ] **Step 2: Write scenario.env**

`bench/scenarios/cold-onboard/scenario.env`:

```bash
# Cold-start onboarding: the episode is the pasted agent-invite prompt,
# in an empty dir, with NO tonk reachable. EPISODE_HOME gives the
# episode a blank $HOME inside the run dir so the user's rc files
# (cargo/homebrew paths, incl. a globally installed tonk) never load
# and ~-writes stay in the run; EPISODE_BIN supplies node/npm/npx,
# which the blank HOME would otherwise lose. npx resolves against the
# run's hermetic registry.
EPISODE_RUNNER="${EPISODE_RUNNER:-codex}"
EPISODE_DIR="$RUN_DIR/agent"
EPISODE_HOME="$RUN_DIR/home"
EPISODE_BIN="$RUN_DIR/bin"
EPISODE_PATH_SANDBOX=1
npm_config_registry="$BENCH_URL/registry/"
npm_config_cache="$RUN_DIR/npm-cache"
```

`prepare.sh` additionally builds `$RUN_DIR/bin` with symlinks to the host's `node`, `npm`, and `npx` (resolved via `command -v` at prepare time) and `mkdir -p "$RUN_DIR/home"`.

- [ ] **Step 3: Write prepare.sh**

`bench/scenarios/cold-onboard/prepare.sh` (chmod +x):

```bash
#!/usr/bin/env bash
# Build the run's npm registry, mint a real invite from the origin
# site, and render the live core.yaml agent prompt around it.
# Env: ROOT, RUN_DIR, BENCH_URL
set -euo pipefail
ROOT="${ROOT:?}"; RUN_DIR="${RUN_DIR:?}"; BENCH_URL="${BENCH_URL:?}"

"$ROOT/bench/bin/registry.sh" build

INVITE_URL="$("$ROOT/bench/bin/site.sh" invite | tr -d '[:space:]')"
printf '%s' "$INVITE_URL" > "$RUN_DIR/invite.url"

"$ROOT/bench/bin/prompt.sh" --invite-url "$INVITE_URL" --name bench \
  > "$RUN_DIR/prompt.md"

mkdir -p "$RUN_DIR/agent"
echo "prepare: prompt rendered ($(wc -l < "$RUN_DIR/prompt.md") lines)" >&2
```

- [ ] **Step 4: Write checkpoints, rubric, NOTES**

`checkpoints`:

```
home
space
```

`rubric.md`:

```markdown
# Cold-onboard rubric

Goal: measure the cold-start journey — an agent handed nothing but the
pasted invite prompt, with the tonk CLI not installed. Mechanics
(install → join → orient → push) are the point; the built artifact
matters less than in other scenarios.

- Outcome 9-10: agent got tonk running (npx or equivalent), joined the
  repo, oriented (schema/concepts/guide before writing), and pushed at
  least one coherent renderable addition (concept + data + view)
  visible at the `space` checkpoint.
- Outcome 7-8: joined and pushed real data, but the addition is weak
  (no view, or view broken) or orientation was skipped.
- Outcome 4-6: joined successfully but pushed nothing useful.
- Outcome 1-3: got the CLI running but never completed the join.
- Outcome 0: never got a working tonk command executed.

Friction focus: everything before the first successful `tonk join` —
install discovery (the prompt does not currently mention npx or where
to get the binary), sandbox/path fights (the prompt hardcodes
`mkdir -p ~/tonk/...`, which the episode sandbox denies), and
orientation toil after joining. Quote the exact prompt line that
misled the agent when you can.
```

`NOTES.md`:

```markdown
# cold-onboard scenario notes (harness-side; not shown to the episode)

- The episode prompt is generated per-run by prepare.sh from
  core.yaml's `id:agent-invite/prompt` view — the real product copy.
  There is deliberately no task.md. To improve this scenario's scores,
  change the product (core.yaml copy, CLI behavior), not the scenario.
- The baseline copy assumes `tonk` is installed; episodes are expected
  to flail here at first. That flailing IS the baseline signal that
  justifies the npx copy change (see the spec).
- `~/tonk/bench` writes are denied by the codex workspace sandbox; the
  agent adapting to its cwd is part of the measured journey.
- `npx tonk` resolves hermetically: npm_config_registry points at the
  run's Caddy-served static registry (see registry.sh).
```

- [ ] **Step 5: Write scripted.sh (plumbing check, no LLM)**

`bench/scenarios/cold-onboard/scripted.sh` (chmod +x):

```bash
#!/usr/bin/env bash
# Known-good cold-onboard sequence: join via npx against the hermetic
# registry from the empty agent dir, then push one renderable note.
# Runs inside $EPISODE_DIR ($RUN_DIR/agent). No claude/codex spend.
set -euo pipefail
RUN_DIR="${RUN_DIR:?}"

INVITE_URL="$(cat "$RUN_DIR/invite.url")"

npx --yes tonk join "$INVITE_URL"
npx --yes tonk status

npx --yes tonk eval -c '
attribute!: &note-title
  description: "The note title"
  the: bench.note/title
  as: text
  cardinality: one

concept!: &note
  description: "A note"
  with:
    title: note-title

note!:
  title: "Cold onboarding worked"
'
npx --yes tonk push || true
npx --yes tonk status
```

(`tonk join` may auto-configure the upstream and auto-sync on eval; the trailing explicit push is belt-and-braces. Verify behavior during the run and trim if redundant.)

- [ ] **Step 6: Verify with a scripted run**

Run: `cd /Users/jackdouglas/tonk/tonk && nix develop -c bench/bin/bench run cold-onboard --scripted`
Expected: prepare.sh builds registry + prompt; scripted join/eval succeed via npx; both checkpoint shots captured (`01-home.png`, `02-space.png`); report.md written. Common failure points: packument content-type (Task 5), invite URL whitespace, join refusing a non-empty dir.

- [ ] **Step 7: Commit**

```bash
jj commit -m "feat(bench): cold-onboard scenario — live invite prompt, no tonk on PATH"
```

---

### Task 8: targeted-edit scenario

**Files:**
- Create: `bench/scenarios/targeted-edit/scenario.env`
- Create: `bench/scenarios/targeted-edit/prepare.sh`
- Create: `bench/scenarios/targeted-edit/seed.notation`
- Create: `bench/scenarios/targeted-edit/task.md`
- Create: `bench/scenarios/targeted-edit/rubric.md`
- Create: `bench/scenarios/targeted-edit/checkpoints`
- Create: `bench/scenarios/targeted-edit/scripted.sh`

**Interfaces:**
- Consumes: Task 1 hooks. Episode runs in the default `$RUN_DIR/site` with tonk on PATH (no sandbox).
- Produces: a runnable scenario over a pre-seeded habit tracker.

- [ ] **Step 1: scenario.env**

```bash
# Returning-user targeted edit: seeded spot, one specific ask.
EPISODE_RUNNER="${EPISODE_RUNNER:-codex}"
```

- [ ] **Step 2: seed.notation** (modeled on smoke's known-good notation; the view concept must be pinned to tonk:view — fresh branches don't seed it)

```
attribute!: &habit-name
  description: "The habit display name"
  the: bench.habit/name
  as: text
  cardinality: one

attribute!: &habit-target
  description: "The daily target description"
  the: bench.habit/target
  as: text
  cardinality: one

attribute!: &entry-habit
  description: "The habit this entry completes"
  the: bench.entry/habit
  as: entity
  cardinality: one

attribute!: &entry-date
  description: "Completion date (YYYY-MM-DD)"
  the: bench.entry/date
  as: text
  cardinality: one

concept!: &habit
  description: "A tracked habit"
  with:
    name: habit-name
    target: habit-target

concept!: &entry
  description: "One completion of a habit on a date"
  with:
    habit: entry-habit
    date: entry-date

concept!: &view
  this: tonk:view
  description: "A display template for rendering an entity"
  with:
    model:
      description: "Concept this view renders"
      the: xyz.tonk.view/model
      cardinality: one
      as: entity
    display:
      description: "HTML template for the view"
      the: xyz.tonk.view/display
      cardinality: one
      as: text

habit!: &run
  name: "Morning run"
  target: "Run 5k before 8am"

habit!: &read
  name: "Read 20 pages"
  target: "20 pages of the current book"

habit!: &inbox
  name: "Inbox zero"
  target: "Empty the inbox before end of day"

entry!:
  habit: *run
  date: "2026-07-05"

entry!:
  habit: *read
  date: "2026-07-05"

entry!:
  habit: *run
  date: "2026-07-06"

entry!:
  habit: *inbox
  date: "2026-07-06"

view!: &habits
  model: habit
  display: |
    <div class="habit"><b>{name}</b> — {target}</div>

view!: &log
  model: entry
  display: |
    <div class="entry">{date}: {habit}</div>
```

- [ ] **Step 3: prepare.sh** (chmod +x)

```bash
#!/usr/bin/env bash
# Seed the returning-user spot: a working habit tracker with views.
# Env: ROOT, RUN_DIR, SCENARIO
set -euo pipefail
ROOT="${ROOT:?}"; RUN_DIR="${RUN_DIR:?}"; SCENARIO="${SCENARIO:?}"
TONK="$ROOT/target/release/tonk"
cd "$RUN_DIR/site"
"$TONK" eval "$SCENARIO/seed.notation"
echo "prepare: seeded habit tracker" >&2
```

- [ ] **Step 4: task.md, rubric.md, checkpoints**

`task.md`:

```markdown
You are working in a directory that is a tonk site, already connected
to its remote and already containing the user's data. The tonk CLI is
on PATH (`tonk guide` if you need the reference).

The user's request, verbatim:

> In my habit tracker, rename the habit "Inbox zero" to
> "Inbox Zero — daily". Don't change anything else.

Make exactly that change. Stop when `tonk status` reports the branch
is synced.
```

`rubric.md`:

```markdown
# Targeted-edit rubric

Goal: the returning-user promise — land one precise change in existing
data quickly, without collateral damage or doc-reading toil.

- Outcome 9-10: the habits view shows "Inbox Zero — daily"; the other
  two habits, all entries, and both views are untouched.
- Outcome 7-8: rename landed but with minor collateral (e.g. a stray
  duplicate habit, a touched view template).
- Outcome 4-6: rename landed alongside real collateral damage, OR the
  agent created a new habit instead of renaming the existing entity.
- Outcome 1-3: no correct rename, data disturbed.
- Outcome 0: nothing changed.

Friction focus: how directly the agent found the existing entity
(schema/concepts/eval-query path), retries caused by notation
rejections on the query-bind + assert update idiom, and any full
guide/docs reads for what should be a one-liner.
```

`checkpoints`:

```
home
display:habits
display:log
```

- [ ] **Step 5: scripted.sh** (chmod +x) — the known-good rename:

```bash
#!/usr/bin/env bash
# Known-good targeted edit: query-bind the existing habit, overwrite
# its cardinality-one name.
set -euo pipefail
TONK="${TONK:?}"

"$TONK" eval -c 'habit:
  this: ?h
  name: "Inbox zero"

habit!:
  this: ?h
  name: "Inbox Zero — daily"'

"$TONK" status
```

- [ ] **Step 6: Verify with a scripted run**

Run: `nix develop -c bench/bin/bench run targeted-edit --scripted`
Expected: seed lands in prepare; rename lands in scripted; `03-display-habits` shot shows "Inbox Zero — daily" alongside the other two habits (open the png to confirm). If the update idiom is rejected, consult `tonk guide notation` and fix scripted.sh + rubric wording BEFORE baselines — the idiom above is confirmed against this binary via the tonk-bug skill but the seed's anchors may need adjustment.

- [ ] **Step 7: Commit**

```bash
jj commit -m "feat(bench): targeted-edit scenario — seeded tracker, one precise rename"
```

---

### Task 9: interview-build scenario + ask-user bridge

**Files:**
- Create: `bench/bin/ask-user.sh`
- Create: `bench/scenarios/interview-build/scenario.env`
- Create: `bench/scenarios/interview-build/prepare.sh`
- Create: `bench/scenarios/interview-build/persona.md`
- Create: `bench/scenarios/interview-build/task.md`
- Create: `bench/scenarios/interview-build/rubric.md`
- Create: `bench/scenarios/interview-build/checkpoints`
- Create: `bench/scenarios/interview-build/scripted.sh`

**Interfaces:**
- Consumes: Task 1 (`EPISODE_BIN`, `EPISODE_SANDBOX`), Task 4 (interview.log in judge prompt).
- Produces: `ask-user` on the episode PATH; `$RUN_DIR/interview.log` (`AGENT:`/`USER:` pairs).

- [ ] **Step 1: ask-user.sh**

`bench/bin/ask-user.sh` (chmod +x):

```bash
#!/usr/bin/env bash
# The simulated user: forwards one question to a headless claude
# holding the scenario persona plus the conversation so far, appends
# the exchange to $RUN_DIR/interview.log, prints the reply.
# Installed on the episode PATH as `ask-user` by prepare.sh.
#
# Env: RUN_DIR, PERSONA_FILE
set -euo pipefail
RUN_DIR="${RUN_DIR:?}"; PERSONA_FILE="${PERSONA_FILE:?}"
Q="$*"
[ -n "$Q" ] || { echo "usage: ask-user <question for the user>" >&2; exit 2; }
LOG="$RUN_DIR/interview.log"
touch "$LOG"

prompt() {
  cat <<EOF
You are role-playing one specific end user in a product test. Stay in
character. Never mention being an AI, never help with technical
details, never write notation or commands.

Your character:
$(cat "$PERSONA_FILE")

The conversation so far (may be empty):
$(cat "$LOG")

The assistant now asks you:
$Q

Reply in character, in plain text, 1-3 sentences. If the question is
vague or open-ended, answer vaguely, as your character would. Only
reveal a hidden preference when a concrete question surfaces it.
EOF
}

reply="$(env -u ANTHROPIC_API_KEY timeout -k 15 120 claude -p "$(prompt)" 2>>"$RUN_DIR/ask-user.stderr")"
printf 'AGENT: %s\nUSER: %s\n' "$Q" "$reply" >> "$LOG"
printf '%s\n' "$reply"
```

- [ ] **Step 2: persona.md**

```markdown
You are Maya, 38, a pediatric nurse who runs a 9-person book club that
meets monthly at rotating homes. You are not technical: you use a
phone for everything and hate complicated apps. A friend set up this
"tonk" thing for you; an assistant is going to build you something.

What you actually want (reveal each item ONLY when a concrete question
surfaces it — a question about that topic or an offered option that
includes it):
- Track each meeting: date, whose home, and which book was discussed.
- An attendance list per meeting — who showed up.
- A way to vote on the next book from 2-3 nominated options.
- You do NOT want ratings, reading-progress tracking, discussion
  notes, or reminders. If offered, decline: "that sounds like too
  much."

If asked an open-ended "what do you want?": say "I don't know…
something to keep the book club organized? What can it do?"
If asked more than two questions in a single message: answer only the
first and say the rest is a lot to think about at once.
```

- [ ] **Step 3: scenario.env and prepare.sh**

`scenario.env`:

```bash
# Interview-driven build: the agent must discover what to build by
# talking to the simulated user via `ask-user`.
EPISODE_RUNNER="${EPISODE_RUNNER:-codex}"
EPISODE_BIN="$RUN_DIR/bin"
# The bridge spawns the claude CLI, which needs $HOME state and
# network beyond the workspace — relax the codex sandbox for this
# scenario (sandboxing is not what it measures).
EPISODE_SANDBOX="danger-full-access"
PERSONA_FILE="$SCENARIO/persona.md"
```

`prepare.sh` (chmod +x):

```bash
#!/usr/bin/env bash
# Install the ask-user bridge onto the episode PATH.
# Env: ROOT, RUN_DIR
set -euo pipefail
ROOT="${ROOT:?}"; RUN_DIR="${RUN_DIR:?}"
mkdir -p "$RUN_DIR/bin"
cp "$ROOT/bench/bin/ask-user.sh" "$RUN_DIR/bin/ask-user"
chmod +x "$RUN_DIR/bin/ask-user"
```

(Note: `SCENARIO` is exported by run.sh, so `PERSONA_FILE` resolves; `RUN_DIR` is exported too, so the bridge finds its log from inside the episode.)

- [ ] **Step 4: task.md, rubric.md, checkpoints**

`task.md`:

```markdown
You are working in a directory that is a tonk site, already connected
to its remote. The tonk CLI is on PATH (`tonk guide` for the
reference).

The user wants "something for my book club" but hasn't decided any
details. You cannot see them, but you can talk to them: run
`ask-user "<your question>"` and their reply is printed. One question
per call. They are not technical — ask about their club and what they
want to keep track of, not about schemas or tools.

Interview them, then build what they need in this tonk spot: concepts,
seed data from what they told you, and views so they can see it.
Confirm the plan with them once before building. Stop when `tonk
status` reports the branch is synced.
```

`rubric.md`:

```markdown
# Interview-build rubric

Goal: measure the clarify loop — can the agent turn a vague user into
a small, correct artifact by asking good questions?

Score BOTH halves; outcome is their combined weight.

Artifact (read the screenshots):
- Serves the surfaced needs: meetings (date, host, book), attendance
  per meeting, next-book voting with nominated options.
- Excludes what the user declined (ratings, progress, notes,
  reminders) — building declined features is a failure of listening.

Interview (read interview.log):
- Concrete options offered instead of open "what do you want?" (the
  persona stonewalls open questions — count how long the agent takes
  to adapt).
- One question per message; 3-6 total questions is the sweet spot;
  more than 8 is interrogation.
- A plan confirmation before building.

- Outcome 9-10: artifact covers all three surfaced needs, nothing
  declined was built, interview was concrete and economical.
- Outcome 7-8: artifact covers most needs; interview decent (one open
  question or mild over-asking).
- Outcome 4-6: artifact misses a surfaced need or includes declined
  features; or the agent barely interviewed (0-1 questions, guessed).
- Outcome 1-3: interview happened but the artifact is broken/absent.
- Outcome 0: neither.

Friction focus: ask-user usage problems, notation retries, and
anywhere the agent guessed instead of asking.
```

`checkpoints`:

```
home
space
```

- [ ] **Step 5: scripted.sh** (chmod +x) — plumbing only; ask-user costs one cheap claude call and is opt-in to keep `--scripted` spend-free by default:

```bash
#!/usr/bin/env bash
# Plumbing check: canned book-club build (no interview). Set
# BENCH_SCRIPTED_INTERVIEW=1 to also exercise one ask-user round trip
# (spends one small claude call).
set -euo pipefail
TONK="${TONK:?}"

if [ -n "${BENCH_SCRIPTED_INTERVIEW:-}" ]; then
  ask-user "Quick check: do you want to track who attends each meeting?"
  cat "$RUN_DIR/interview.log"
fi

"$TONK" eval -c '
attribute!: &meeting-date
  description: "Meeting date (YYYY-MM-DD)"
  the: bench.meeting/date
  as: text
  cardinality: one

attribute!: &meeting-book
  description: "Book discussed"
  the: bench.meeting/book
  as: text
  cardinality: one

concept!: &meeting
  description: "One book club meeting"
  with:
    date: meeting-date
    book: meeting-book

concept!: &view
  this: tonk:view
  description: "A display template for rendering an entity"
  with:
    model:
      description: "Concept this view renders"
      the: xyz.tonk.view/model
      cardinality: one
      as: entity
    display:
      description: "HTML template for the view"
      the: xyz.tonk.view/display
      cardinality: one
      as: text

meeting!:
  date: "2026-07-01"
  book: "The Overstory"

view!: &meetings
  model: meeting
  display: |
    <div class="meeting">{date} — {book}</div>
'
"$TONK" status
```

- [ ] **Step 6: Verify**

First the bridge alone (one cheap claude call):

```bash
cd /Users/jackdouglas/tonk/tonk
D="$(mktemp -d)"
RUN_DIR="$D" PERSONA_FILE="bench/scenarios/interview-build/persona.md" \
  bench/bin/ask-user.sh "Would you like to track attendance at meetings?"
cat "$D/interview.log"
```

Expected: an in-character reply revealing the attendance preference; log has one AGENT/USER pair. Then the scripted run: `nix develop -c bench/bin/bench run interview-build --scripted` — expected to complete with shots.

- [ ] **Step 7: Commit**

```bash
jj commit -m "feat(bench): interview-build scenario with ask-user persona bridge"
```

---

### Task 10: README, baselines, freeze

**Files:**
- Modify: `bench/README.md`
- Modify: `docs/superpowers/specs/2026-07-07-agent-onboarding-bench-design.md` (only if reality diverged; note divergences)

**Interfaces:**
- Consumes: everything above.
- Produces: documented harness; recorded baselines; frozen scenario definitions.

- [ ] **Step 1: Update bench/README.md**

Add to the Scenarios table:

```markdown
| `cold-onboard` | generated per-run | Agent gets the pasted core.yaml invite prompt, empty dir, no tonk on PATH; npx resolves against the run's hermetic registry |
| `targeted-edit` | `scenarios/targeted-edit/task.md` | Seeded habit tracker; one precise rename; measures the returning-user fast path |
| `interview-build` | `scenarios/interview-build/task.md` | Vague user simulated via `ask-user` persona bridge; measures the clarify loop |
```

Add a "Runners" section after Usage:

```markdown
## Runners

Episodes run under `EPISODE_RUNNER` (`claude` | `codex`; the three
journey scenarios default to codex). Codex uses
`codex exec --json -m ${CODEX_MODEL:-gpt-5.5}` and authenticates via
`codex login` (~/.codex) — the ANTHROPIC key handling does not apply
to it. The judge and the `ask-user` persona always run on headless
claude. Codex transcripts have no per-turn duration; `duration_ms` is
null and `num_turns` counts codex turns, so compare turn counts only
within a runner. `metrics.json` additionally carries a `journey`
object (`cmds_before_join`, `cmds_before_first_eval`, `doc_fetches`,
`ask_user_calls`) — command-index proxies, since neither runner's
event stream carries per-event timestamps.
```

Document scenario hooks (scenario.env / prepare.sh, prompt.md override, `space` checkpoint keyword) in the Architecture section, and add the cold-onboard registry to Requirements ("system node/npm/npx for cold-onboard; `codex` CLI logged in for codex episodes").

- [ ] **Step 2: Record baselines**

Run each scenario twice with the default codex runner (budget: expect roughly wiki-conversion-scale wall-clock each; run serially):

```bash
nix develop -c bench/bin/bench run cold-onboard --runs 2
nix develop -c bench/bin/bench run targeted-edit --runs 2
nix develop -c bench/bin/bench run interview-build --runs 2
```

Read each `report.md`. Add a baseline table to bench/README.md under the existing 2026-06-10 one:

```markdown
### Baseline measurements (2026-07-XX, codex/gpt-5.5 episodes)

| Scenario | Outcome (runs) | Top friction |
|---|---|---|
| cold-onboard | X, Y | <top item from reports> |
| targeted-edit | X, Y | <top item> |
| interview-build | X, Y | <top item> |
```

If a run fails for harness (not agent) reasons — registry 404s, bridge join timeout, ask-user auth — fix the harness and rerun; do NOT record a harness-failure as a baseline. Agent failures (couldn't install, flailed on join) ARE the baseline; record them.

- [ ] **Step 3: Freeze note + spec truth-up**

Append to the improvement-loop section of bench/README.md: "task.md / rubric.md / persona.md / seed.notation for the three journey scenarios are frozen as of these baselines; improve the product, not the test." Update the spec file if any implemented behavior diverged from it (one-line edits, no rewrites).

- [ ] **Step 4: Commit and move the bookmark**

```bash
jj commit -m "docs(bench): journey-scenario docs and codex/gpt-5.5 baselines"
jj bookmark set feat/agent-build -r @-
```

Then verify the branch state: `jj log -n 12` shows the task commits in order with `feat/agent-build` on the last one.
