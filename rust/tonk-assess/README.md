# tonk-assess

Benchmark harness for evaluating AI agent probe accuracy. Spawns `claude` CLI
as a subprocess, sends it probes, captures structured JSON output, and scores
results with keyword or LLM judges.

## Quick start

```sh
# List available probes
cargo run -p tonk-assess -- benchmark/probe --list

# List probes with details
cargo run -p tonk-assess -- benchmark/probe --list --persona marcus -v

# Run all lookup probes for marcus
cargo run -p tonk-assess -- benchmark/probe --persona marcus --tag lookup

# Run a single probe by ID
cargo run -p tonk-assess -- benchmark/probe --probe marcus-lookup-01

# Verbose output (shows raw claude CLI JSON on stderr)
cargo run -p tonk-assess -- benchmark/probe --persona marcus --tag lookup -v
```

## Probe format

Probes are YAML files following the [Dialog concept notation][domain-model].
Each file asserts a single `tonk.assess/Probe` concept instance. The concept
definition lives in [`concept/probe.yml`](concept/probe.yml).

[domain-model]: https://github.com/tonk-labs/rfc/blob/feat/domain-modeling/rfc/domain-model.md

### Directory layout

Probes are discovered by recursively walking the probe directory. The convention
is `{probe_dir}/{persona}/{tier}/{persona}-{tier}-NN.yml`:

```
benchmark/probe/
  marcus/
    lookup/
      marcus-lookup-01.yml
      marcus-lookup-02.yml
    synthesis/
      marcus-synthesis-01.yml
    inference/
      marcus-inference-01.yml
```

The probe ID is derived from the filename (e.g. `marcus-lookup-01.yml` becomes
`marcus-lookup-01`).

### Probe instance format

Each `.yml` file asserts one probe:

```yaml
tonk.assess/Probe:
  persona: marcus
  corpus: ../personas/marcus/artifacts
  tag: [lookup, base]
  prompt: "What test framework does the api-gateway project use?"
  judge:
    tonk.assess/Judge:
      keyword:
        tonk.assess/KeywordJudge:
          keyword:
            - term: vitest
              score: 2
            - term: jest
              score: 1
  source-file:
    - projects/api-gateway/CLAUDE.md
```

### Fields

| Field | Type | Required | Description |
|---|---|---|---|
| `persona` | string | yes | Persona name. Must match a directory under `benchmark/personas/`. |
| `tag` | string[] | yes | Arbitrary tags for filtering (e.g. `[lookup, base]`, `[synthesis, carry]`). |
| `prompt` | string | yes | The question sent to the agent. |
| `judge` | Judge | yes | Scoring config — either `keyword` or `llm` (see below). |
| `name` | string | no | Human-readable label shown in `--list` instead of the ID. |
| `corpus` | string | no | Relative path from probe dir to corpus files. Default: `../personas/<persona>/artifacts`. |
| `source-file` | string[] | no | Paths within corpus where the answer lives. Documentation for probe authors. |
| `allowed-tool` | string[] | no | Tools the agent may use. Default: `Read,Glob,Grep`. |
| `system-prompt` | string | no | Relative path from probe dir to a markdown file appended as system prompt. |
| `max-turns` | integer | no | Maximum agent turns (default: 10). |
| `mcp-config` | string | no | Path to an MCP server config JSON file. |

### Tags

Tags are arbitrary strings used for filtering with `--tag`. Common conventions:

- **Difficulty tier**: `lookup`, `synthesis`, `inference`
- **Condition**: `base` (file access), `carry` (tool access)
- **Custom**: anything useful for filtering (`fast`, `regression`, `api`, etc.)

## Judges

Judges score how well an agent's answer matches expectations. Each probe is
scored on a 0–10 scale:

| Score | Meaning |
|-------|---------|
| 0     | Completely wrong, no relevant information |
| 1–3   | Mostly wrong, significant gaps |
| 4–5   | Partially correct, gets the idea but misses key facts |
| 6–7   | Good, covers most key facts with some gaps |
| 8–9   | Strong to excellent, nearly complete |
| 10    | Perfect, fully correct and covers all key facts |

### Keyword judge

Best for lookup queries with clear factual answers. Each keyword has a score;
the agent's answer is checked for term presence.

```yaml
judge:
  tonk.assess/Judge:
    keyword:
      tonk.assess/KeywordJudge:
        keyword:
          - term: vitest
            score: 2
          - term: jest
            score: 1
        max-score: 3          # optional cap
```

### LLM judge

Best for synthesis/inference queries where correctness is nuanced. An LLM
compares the agent's answer against the ground truth and key facts.

```yaml
judge:
  tonk.assess/Judge:
    llm:
      tonk.assess/LlmJudge:
        ground-truth: "Tailwind CSS across all projects..."
        key-fact:
          - Tailwind in dashboard-ui
          - shadcn/ui in dashboard-ui
        system-prompt: "You are a strict technical evaluator."   # optional
        model: claude-haiku-4-5-20251001                          # optional
```

## Output

Results are written to `benchmark/results/results-{timestamp}.json` and a
summary table is printed to stdout:

```
PROBE                            SCORE   TURNS   IN TOK  OUT TOK    TIME ms
---------------------------------------------------------------------------
marcus-lookup-01                  10/10       2     1200      340      14346
marcus-lookup-02                   7/10       3     1800      520       8200
---------------------------------------------------------------------------
TOTAL / AVG                      8.5/10     2.5     3000      860
```

## CLI reference

```
tonk-assess <probe_dir> [OPTIONS]

Arguments:
  <probe_dir>              Path to probe directory (required)

Options:
  --persona <NAME>         Filter by persona name
  --tag <TAG,TAG>          Filter by tag (probes must have all specified values)
  --probe <ID_OR_KEYWORD>  Filter by probe ID or keyword in prompt text
  --judge <TYPE,TYPE>      Filter by judge type (llm, keyword)
  --model <MODEL>          Model for agent runs [default: claude-sonnet-4-6]
  --judge-model <MODEL>    Model for LLM judge [default: claude-sonnet-4-6]
  --output-dir <PATH>      Path to output results directory
  --list                   List available probes and exit
  -v, --verbose            Print debug output to stderr
  -h, --help               Full usage info
```

## Project layout

```
rust/tonk-assess/
  concept/
    probe.yml    — Probe concept definition (schema)
  src/
    lib.rs       — Public library re-exports
    main.rs      — CLI entry point (clap)
    types.rs     — Probe, Score, RunMetrics, JudgeConfig, etc.
    probe.rs     — YAML probe loading
    agent.rs     — Spawns claude CLI, captures JSON output
    judge.rs     — Keyword + LLM-as-judge scoring
    report.rs    — Results JSON + summary table
  tests/
    parse_output.rs — Tests against real claude CLI output
```
