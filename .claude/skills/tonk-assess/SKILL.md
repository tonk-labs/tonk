---
name: tonk-assess
description: Generate, add, and update probes and personas for the tonk-assess benchmark CLI. Creates probe YAML files, persona profiles, corpus artifacts, and system prompt files. Supports adding tool comparisons (e.g. Zep vs Carry) and incorporating user research insights.
---

# tonk-assess — Probe & Persona Generator

Generates and maintains benchmark content for `tonk-assess`, a CLI that runs AI agent probes and scores answers. This skill helps:

- **Create** persona profiles with realistic corpus artifacts
- **Create** probe YAML files with judges (keyword or LLM)
- **Add** probes for new tool comparisons (e.g. Zep vs Carry vs baseline)
- **Update** corpus artifacts with user research insights
- **Expand** probe coverage for existing personas

---

## Directory Layout

```
benchmark/
├── probe/                          # Probe YAML files (this is the probe_dir)
│   └── <persona>/
│       └── <tier>/
│           ├── <persona>-<tier>-01.yml
│           └── ...
├── personas/
│   └── <persona>/
│       ├── profile.md              # Persona description
│       └── artifacts/              # Corpus — files the agent can access
│           └── projects/
│               └── ...
└── prompts/                        # Shared system prompt files
    ├── base.md                     # For base condition (file access)
    └── carry.md                    # For carry condition (tool access)
```

The probe ID is derived from the filename (e.g. `marcus-lookup-01.yml` becomes `marcus-lookup-01`).

---

## Probe YAML Format

Every probe file uses the `tonk.assess/Probe:` wrapper:

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

### Required fields

| Field | Description |
|---|---|
| `persona` | Persona name (matches directory under `personas/`) |
| `tag` | List of tags for filtering (e.g. `[lookup, base]`, `[synthesis, carry]`) |
| `prompt` | The question sent to the agent |
| `judge` | Scoring config — either `keyword` or `llm` (see below) |

### Optional fields

| Field | Description |
|---|---|
| `name` | Human-readable label (shown in `--list` instead of ID) |
| `corpus` | Relative path from probe dir to corpus files (default: `../personas/<persona>/artifacts`) |
| `source-file` | Paths within corpus where the answer lives (documentation only) |
| `allowed-tool` | Tools the agent may use (default: `Read,Glob,Grep`) |
| `system-prompt` | Relative path from probe dir to a markdown file appended as system prompt |
| `max-turns` | Max agent turns (default: 10) |
| `mcp-config` | Path to MCP server config JSON |

### Tags

Tags are arbitrary strings used for filtering with `--tag`. Common conventions:

- **Difficulty tier**: `lookup`, `synthesis`, `inference`
- **Condition**: `base` (file access), `carry` (tool access)
- **Custom**: anything else useful for filtering (`fast`, `regression`, `api`, etc.)

A probe for the base condition and a probe for the carry condition testing the same question would be two separate YAML files with the same `prompt` but different tags and agent config:

```yaml
# marcus-lookup-01-base.yml
tonk.assess/Probe:
  persona: marcus
  corpus: ../personas/marcus/artifacts
  tag: [lookup, base]
  prompt: "What test framework does the api-gateway project use?"
  judge: ...
  source-file:
    - projects/api-gateway/CLAUDE.md
```

```yaml
# marcus-lookup-01-carry.yml
tonk.assess/Probe:
  persona: marcus
  corpus: ../personas/marcus/artifacts
  tag: [lookup, carry]
  prompt: "What test framework does the api-gateway project use?"
  allowed-tool:
    - Bash
  system-prompt: ../prompts/carry.md
  judge: ...
  source-file:
    - projects/api-gateway/CLAUDE.md
```

---

## Judge Types

### Keyword judge

Best for lookup queries with clear factual answers. Each keyword has a score; the agent's answer is checked for term presence.

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

Best for synthesis/inference queries where correctness is nuanced. An LLM compares the agent's answer against the ground truth and key facts.

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

---

## CLI Usage

```
tonk-assess <probe_dir> [OPTIONS]

# List probes
tonk-assess benchmark/probe --list
tonk-assess benchmark/probe --list --tag lookup
tonk-assess benchmark/probe --list --persona marcus
tonk-assess benchmark/probe --list -v              # verbose: show details

# Run probes
tonk-assess benchmark/probe --persona marcus --tag lookup
tonk-assess benchmark/probe --tag base --judge keyword
tonk-assess benchmark/probe --probe marcus-lookup-01

# Options
--persona <NAME>       Filter by persona
--tag <TAG,TAG>        Filter by tag (must have all)
--probe <TEXT>         Filter by ID or prompt text
--judge <TYPE,TYPE>    Filter by judge type (llm, keyword)
--model <MODEL>        Agent model (default: claude-sonnet-4-6)
--judge-model <MODEL>  Judge model (default: claude-sonnet-4-6)
--output-dir <PATH>    Results directory (default: <probe_dir>/../results)
-v                     Verbose output
```

---

## Workflow: Generating a New Persona

### Step 1: Interview

Ask the user:
- Who is this persona? (role, background, technical stack)
- What knowledge do they have? (project configs, notes, documentation)
- What formats are their files in? (CLAUDE.md, README, Obsidian, etc.)

### Step 2: Create the persona profile

Write `benchmark/personas/<name>/profile.md` with background, tools, knowledge topology, and what a good answer looks like.

### Step 3: Generate corpus artifacts

Create realistic files under `benchmark/personas/<name>/artifacts/`. Match the persona's actual file formats. Real-looking content with natural inconsistencies is better than polished perfection.

### Step 4: Generate probes

For each probe, create a YAML file. Aim for 15 probes per persona across difficulty tiers:

- **5 lookup** — single-fact retrieval, answerable from one file
- **5 synthesis** — combining facts from multiple files
- **5 inference** — reasoning over knowledge, drawing conclusions not explicitly stated

For each probe:
1. Write the ground truth answer first
2. Then write the prompt that requires knowing that answer
3. Pick the judge type: keyword for factual lookups, LLM for nuanced answers
4. Tag with difficulty tier and condition (`base`, `carry`, or both)

Name files as `<persona>-<tier>-NN.yml` (e.g. `marcus-lookup-01.yml`).

### Step 5: Verify

```bash
# List all probes for the persona
tonk-assess benchmark/probe --list --persona <name>

# Dry run: check probes load correctly with verbose
tonk-assess benchmark/probe --list --persona <name> -v
```

---

## Workflow: Adding Probes for a New Tool Comparison

Use this when a product person wants to benchmark a new memory tool (e.g. Zep, Mem0, LangMem) against the baseline or against Carry.

### Step 1: Understand the tool

Ask the user:
- What tool are we comparing? (name, what it does)
- What condition does it represent? (a new tag, e.g. `zep`, `mem0`)
- How does the agent access it? (MCP server, Bash commands, specific tools)
- Is there a system prompt the agent needs? (instructions for using the tool)
- Does the tool need its own MCP config JSON?

### Step 2: Create the system prompt (if needed)

Write `benchmark/prompts/<tool>.md` with instructions for the agent to use the tool. Follow the same pattern as existing prompts — brief, directive, focused on how to query the tool.

### Step 3: Create probe files

For each existing probe that should be tested against the new tool, create a new YAML file with the tool's tag. The prompt and judge stay the same — only the agent config changes.

Naming: `<persona>-<tier>-NN-<tool>.yml` (e.g. `marcus-lookup-01-zep.yml`)

```yaml
# marcus-lookup-01-zep.yml — same question, different tool
tonk.assess/Probe:
  persona: marcus
  corpus: ../personas/marcus/artifacts
  tag: [lookup, zep]
  prompt: "What test framework does the api-gateway project use?"
  allowed-tool:
    - Bash
  system-prompt: ../prompts/zep.md
  mcp-config: ../configs/zep.json    # if needed
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

### Step 4: Add tool-specific probes (optional)

Some tools may have strengths that warrant unique probes not in the existing set. Create these as new probe files with appropriate tags. These probes should still have base condition counterparts so results are comparable.

### Step 5: Verify

```bash
# List probes for the new tool
tonk-assess benchmark/probe --list --tag <tool>

# Compare probe counts — should match base set (plus any tool-specific extras)
tonk-assess benchmark/probe --list --tag base | wc -l
tonk-assess benchmark/probe --list --tag <tool> | wc -l

# Run a single probe to sanity check
tonk-assess benchmark/probe --probe <persona>-lookup-01-<tool>
```

---

## Workflow: Incorporating User Research Insights

Use this when user research uncovers new information about how a persona works, what files they have, or what questions they ask. This updates the corpus and may add probes.

### Step 1: Understand the insight

Ask the user:
- Which persona does this apply to? (existing or new?)
- What did we learn? (new files they have, workflow details, pain points, questions they ask)
- Does this change existing corpus files or add new ones?
- Are there new questions we should test?

### Step 2: Update corpus artifacts

Based on the insight, either:

**Add new files** — Create new files under `benchmark/personas/<persona>/artifacts/` that reflect the newly discovered content. Match the persona's existing file formats and conventions. Keep it realistic — if Marcus has messy CLAUDE.md files, new files should have the same character.

**Update existing files** — Edit existing corpus files to incorporate new details. Be careful to preserve the natural feel — don't over-polish. Add notes-to-self, TODOs, or commented-out sections if that's the persona's style.

### Step 3: Add probes for new knowledge

If the new corpus content enables new questions, create probe files:

1. Write the ground truth answer first (what should the agent say?)
2. Write the prompt that requires finding/reasoning over the new content
3. Choose the judge type:
   - **Keyword** for facts that have clear key terms
   - **LLM** for answers requiring nuance or multiple facts
4. Tag appropriately (tier + condition)
5. List `source-file` paths pointing to the new/updated corpus files

### Step 4: Audit existing probes

Check whether any existing probes are affected by the corpus change:
- Do any ground truth answers need updating?
- Do any `source-file` references need updating?
- Are any keyword judge terms affected?

```bash
# List all probes for the persona
tonk-assess benchmark/probe --list --persona <persona> -v
```

### Step 5: Verify

```bash
# Make sure probes still parse
tonk-assess benchmark/probe --list --persona <persona>

# Verify source-file paths exist
# (manually check that source-file entries in each probe point to real files in the corpus)
```

---

## Workflow: Adding Probes to an Existing Persona

Use this to expand probe coverage without changing the corpus. Good for testing edge cases, covering gaps, or adding probes after reviewing eval results.

### Step 1: Identify the gap

Common reasons to add probes:
- A capability isn't being tested (e.g. no probes about error handling preferences)
- Eval results show suspiciously high scores — probes may be too easy
- A new difficulty tier or topic area needs coverage

### Step 2: Review existing corpus

Read the persona's corpus files to find untested knowledge:

```bash
# See what files exist
ls benchmark/personas/<persona>/artifacts/

# See what's already tested
tonk-assess benchmark/probe --list --persona <persona> -v
```

Look for facts, patterns, or connections in the corpus that no probe currently tests.

### Step 3: Write probes

Follow the same process as new persona probes:
1. Ground truth first, then prompt
2. Pick judge type
3. Tag with tier + condition
4. Set `source-file`

Number new probes sequentially after existing ones (e.g. if `marcus-lookup-05.yml` exists, next is `marcus-lookup-06.yml`).

### Step 4: Create condition variants

If the probe should be tested under multiple conditions (base, carry, zep, etc.), create a separate YAML file for each. Same prompt and judge, different tags and agent config.

---

## Quality Checklist

- [ ] Corpus artifacts look like real files (not too clean, natural inconsistencies)
- [ ] Lookup probes are genuinely single-fact (could be answered from one file section)
- [ ] Synthesis probes require combining information from 2+ files
- [ ] Inference probes require reasoning beyond what's explicitly stated
- [ ] Keyword judges have relevant terms with appropriate score weights
- [ ] LLM judges have specific ground truth and key facts (not vague)
- [ ] `source-file` paths actually exist in the corpus
- [ ] Every probe has appropriate tags for filtering
- [ ] `corpus` path resolves correctly relative to probe dir
