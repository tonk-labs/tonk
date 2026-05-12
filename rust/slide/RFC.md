# Slide — RFC / planning doc

Status: draft, pre-implementation
Branch: `feat/tonk-concept-component`
Owner: TBD
Last updated: 2026-05-07

## TL;DR

Slide is a tiny CLI for reading and writing tonk/dialog facts via the
asserted-notation DSL (`rust/tonk-notation/guide.md`). Its entire mutating
surface is one command, `slide eval`, that consumes a notation document and
runs the same analyze → query → plan → commit pipeline as the worker's
`/evaluate` route — but locally, against a `.tonk/` repo, with no HTTP, no
sync, no service worker.

The target user is not a human at a terminal. It's an LLM agent in a harness:
something that has to (a) learn what the tool does, (b) write valid input,
(c) read back results to decide what to do next. Every design choice below
is in service of those three things.

## Background

We have two existing CLIs over the same underlying database:

- **carry** (`feat/carry`, parked) — a broad CLI surface with separate
  subcommands for `init`, `query`, `assert`, `retract`, `status`, `identity`,
  `invite`, `join`, `remote`, `push`, `pull`. Each command parses
  `field=value` pairs in its own bespoke way and resolves targets
  (domain / concept / file / stdin) through ad-hoc logic. About 15 source
  files; ~25 if you count tests and helpers.
- **tonk-worker + tonk-ui** (`feat/tonk-concept-component`, active) —
  service-worker that exposes the dialog DB over HTTP, including a
  `POST /api/repository/{repo}/branch/{branch}/evaluate` route that takes a
  whole asserted-notation document and runs it as one transaction. The
  worker is wasm-only; the Leptos UI consumes its routes.

Carry's surface predates the maturation of asserted-notation. By the time
the notation got a real analyzer and the worker grew `/evaluate`, most of
carry's per-command DSL was duplicating what the notation already says
better. Slide is the redesign that takes the notation seriously.

## Why a new CLI rather than reviving carry

- Carry will not be developed further. Building on top of it would mean
  re-asserting ownership of code we've decided to park.
- Carry's CLI grammar (positional target + `field=value` pairs + special
  prefixes like `this=` and `@name`) is its own DSL — a second thing for an
  agent to learn beyond the notation. We want exactly one DSL.
- Carry depends on the full sync stack (`tonk-invite`,
  `dialog-remote-ucan-s3`, `tonk-access-service`). Slide doesn't, so its
  build is cheaper and its blast radius smaller.

## Goals

1. **Single learnable surface.** An agent reading
   `tonk-notation/guide.md` plus `slide --help` should be able to use the
   tool without reading any other file.
2. **One round trip per task.** A query + the asserts that depend on its
   bindings + a retraction should all fit in one `slide eval` call, exactly
   like the worker's `/evaluate` route already supports.
3. **Machine-first output.** JSON by default. Stable schema. Exit codes
   that distinguish parse / analyze / commit failure so an agent can branch
   on them without parsing prose.
4. **Self-introspection.** The agent can ask the tool what attributes and
   concepts already exist on the branch and get back a notation document it
   could re-submit verbatim.
5. **Local-only, single replica.** No invites, remotes, sync. This is the
   hard line that keeps the surface tiny.

## Non-goals

- Multi-replica / sync / invite minting. Use carry or the future tonk
  CLI for that. If slide needs networked data later, we'll reach for
  `tonk-worker` / its successor, not bolt sync onto the CLI.
- Schema migrations beyond what the notation already expresses.
- A REPL. Stdin handles the interactive case; agents don't need it.
- A service mode. Slide is a one-shot per invocation.

## CLI surface

```
slide eval [-c "<notation>"] [<file>] [-]   # the only mutating/reading verb
slide init [<label>]                         # create .tonk/ in $PWD
slide identity [--reset]                     # show or regen the local DID
slide guide                                  # print tonk-notation/guide.md
slide schema                                 # print branch's attrs+concepts as a notation doc
slide migrate [--from <path>] [--move]       # one-shot .carry/ -> .tonk/ migration
```

Six subcommands total. Five are read-only support; the real surface
is `eval`. `migrate` exists primarily for one release window — see
the "Migration from carry" section below.

### `slide eval`

Input modes, in priority order:

| Form                         | Source            |
|------------------------------|-------------------|
| `slide eval -c "<doc>"`      | inline string     |
| `slide eval <file>`          | file path         |
| `slide eval -` or `slide eval` (no args, with piped stdin) | stdin |

Reasoning: `-c` mirrors `bash -c`, `psql -c`, `jq -c` — agents recognise
this. Positional file path matches `cat`. Bare `-` is standard. We
explicitly drop carry's `-f <file>` flag in favour of the positional
form: redundant flags are noise the agent has to disambiguate.

Per-invocation flags:

- `--format notation|json` — output shape. Default `notation`: emit a
  re-submittable notation document for matches, with envelope info
  (revisions, claim count, entity bindings) in YAML comment lines at
  the top. `json` emits the structured `EvaluateResponse` for harnesses
  that want to consume it programmatically. See "Output" below.
- `--quiet` / `-q` — suppress matches, print only the commit summary.
  Useful for write-only documents where the agent only wants to know
  whether the commit succeeded.
- `--dry-run` — run analyze + query, skip the commit. The response
  carries `matches_before` only and `commits.claims = 0`. Lets an agent
  preview a transaction without touching the branch.

There is no `--branch` flag at v0. Slide writes to `main` only.
Multi-branch UX (creation, listing, switching) is intentionally
out of scope; adding `--branch` to `eval` alone would create the
illusion of branch support that doesn't exist.

There is no `--repo <path>` flag at v0. Slide walks up from `$PWD`
looking for `.tonk/`, exactly like git or carry; an explicit override
can be added later if a real workflow needs it.

### `slide init`

Creates a `.tonk/` directory in `$PWD`, bootstraps a dialog repository
named `main`, opens the `main` branch, and seeds the built-in
`attribute` and `concept` schemas. Optional `<label>` writes a
`dialog.meta/name` claim on the repo entity. No telemetry, no identity
prompts, no admin flags.

If `.tonk/` already exists, this command is a no-op that prints the
repo's DID. (Distinct exit code 0 either way; the agent can run
`slide init` defensively at the start of a task.)

### `slide identity`

Prints `did: did:key:…`. With `--reset`, deletes and regenerates the
local profile. Identity lives in the platform data dir, the same way
carry handles it (we can lift `identity_cmd::ensure_identity`
mostly as-is).

### `slide guide`

Cats `tonk-notation/guide.md` to stdout, baked into the binary at build
time via `include_str!`. Agents in sandboxes that don't have access to
this repo can still learn the notation by running `slide guide`.

### `slide schema`

Reads every attribute and concept asserted on the branch and emits a
notation document that, if re-submitted to `slide eval` against an
empty branch, would reproduce them. Two reasons this matters:

1. The agent can prime its context with the branch's vocabulary in one
   tool call.
2. The output is its own answer to "what is the right syntax?" — every
   line is a worked example.

### `slide migrate`

One-shot migration from a `.carry/` directory to a `.tonk/` directory.
Without arguments, walks up from `$PWD` looking for a `.carry/` and
copies it to a sibling `.tonk/`. With `--from <path>`, takes that
explicit source. With `--move`, deletes the source after the copy
completes and the destination opens cleanly.

Detail in the "Migration from carry" section below.

## Migration from carry

Carry's on-disk format is dialog's `NativeSpace` operator base — a
directory tree keyed by repository name. Carry uses repo name `main`,
two branches (`main` content + `meta`), under
`Directory::At(.carry)`. Slide will use the *same* layout under
`Directory::At(.tonk)`. So byte-for-byte, the dialog data inside
`.carry/` and the dialog data slide expects inside `.tonk/` are the
same shape.

That makes migration a directory rename plus a sanity check:

1. **Locate** the source `.carry/` (explicit `--from` or walk up
   from `$PWD`).
2. **Refuse if `.tonk/` already exists in the destination** — never
   silently merge or overwrite. The agent must remove or rename the
   conflicting `.tonk/` first.
3. **Copy** `.carry/` → `.tonk/`. (Default. With `--move`, slide
   does an atomic rename when source and destination are on the same
   filesystem, falling back to copy-then-delete otherwise.)
4. **Open** the new `.tonk/` to verify the dialog repository loads
   and both branches are intact. If open fails, slide rolls back
   (deletes the partially-copied `.tonk/`) and reports the error.
5. **Print** the repo DID and a one-line note that any sync remotes
   configured under carry's meta branch are preserved (slide doesn't
   read them today, but they're still on disk for the future).

Identity is *not* migrated. The user's profile lives in the platform
data dir already and is shared by carry and slide. The DID stays the
same; the agent doesn't need to re-delegate anything.

The migration is intentionally not automatic on first `slide init`:
silently moving the user's data on a defensive `slide init` would
violate the "actions with blast radius beyond the local cwd need
explicit consent" principle. Make the user (or agent) say "migrate".

This subcommand is expected to be load-bearing for one release
window. After existing carry users have moved, we can soft-deprecate
it (still ship it, but don't advertise) without removing it.

## Why a notation-only surface beats more flags

This is the question to push back on: should slide expose more
fine-grained commands (`slide assert <concept> field=value …`,
`slide query <concept> field=?` etc.) like carry did, or should it
funnel everything through `eval`?

I recommend funnelling everything through `eval`, for these reasons:

1. **The notation is already the more expressive surface.** The
   notation can describe joins, multi-statement transactions, named
   variables that flow between query and mutation, retraction by query
   result, and built-in schema declarations — all inside one document.
   A flag-based CLI cannot express joins or multi-expression
   transactions without inventing additional syntax (see carry's
   `this=…` sigil, `@name` prefix, file-vs-target ambiguity in the
   first arg). Each invented sigil is one more thing the agent must
   memorise.

2. **One DSL is cheaper to teach than two.** With `eval`, the agent
   reads `slide guide` once and is done. Anything it can do, it does
   in YAML. With per-command flags, the agent has to learn the YAML
   notation (for files) *and* the flag grammar (for inline calls), and
   reason about which is appropriate when.

3. **LLMs write YAML well.** Token-for-token, agents are more reliable
   producing well-formed YAML than they are at composing
   `slide assert person name="Alice" age=28 @alice` invocations,
   especially when shell quoting is involved. The notation also lets
   the agent see the entire transaction in one place, which improves
   self-correction.

4. **The error surface collapses.** With one command, every parse /
   analyze error has the same shape (`source:line:col: message`)
   regardless of what the agent was trying to do. With many commands,
   each command's argument parser produces its own error vocabulary.

5. **It mirrors what's already proven.** `tonk-worker`'s `/evaluate`
   already implements this exact "one document → one transaction" model
   and the editor (`tonk-ui`) drives it. Slide is the same idea,
   stripped of the HTTP envelope.

The cost is real: a one-off `slide eval -c "person: name: \"Alice\""`
is wordier than `slide query person name=Alice` would be. But agents
don't optimise for keystrokes — they optimise for predictability. The
trade is worth it.

If a particular query becomes idiomatic enough to deserve a shortcut,
the right place for it is a *snippet* in the notation guide, not a new
subcommand. We can add `slide guide --topic <name>` later if that
becomes load-bearing.

## Output

Two formats. `notation` is the default; `json` is opt-in via
`--format json`.

### Why notation by default

The strongest argument is **symmetry**: the language an agent reads
out of slide is the same language it writes into slide. That has a
few concrete consequences:

1. **Round-tripping is free.** An agent can run a query, edit a field
   in the output, and pipe the result back as an assertion (after
   adding a `!` to the head). No format conversion, no schema
   mismatch.
2. **The output is its own training data.** Every successful call
   produces a worked example of correct notation. An agent that
   misremembers the syntax can re-derive it from its last good
   response.
3. **One grammar to learn.** With JSON output, the agent has to
   master notation (for input) *and* the `EvaluateResponse` JSON
   schema (for output) and reason about which produces what. With
   notation output, there is one grammar and `slide guide` covers
   all of it.
4. **Fewer tokens.** Notation is denser than the equivalent JSON for
   the same content. Cheaper to fit in context, cheaper to stream.

The arguments for JSON-by-default were really arguments for
"machine-readable output" — and notation is just as machine-readable
once the agent has read the guide. The "structured envelope" piece
(revisions, commit counts, entity bindings) is genuine, but small
enough to fit in YAML comments without losing parse-ability.

The escape hatch: `--format json` for harnesses that prefer to
consume the structured response directly without parsing YAML.

### Notation format

Single document with two sections, separated by an `---` YAML
document marker. The first document is the envelope (a YAML mapping
of pure metadata). The second document is the matches, written as
re-submittable notation expressions:

```yaml
revision-before: rev:abc...
revision-after:  rev:def...
claims:          2
entities:
  alice: did:key:zHj...
  "?p":  did:key:zHj...
---
person did:key:zHj...:
  name: "Alice"
  age:  28
```

Why this shape:

- **The envelope is its own YAML document.** `yaml.safe_load_all`
  yields two values; the agent picks whichever it cares about.
- **Each match is a valid notation query expression.** Head uses the
  URI binding form (`person did:key:...:`), so an agent can re-run
  the result as a query verbatim. Add a `!` to assert; replace fields
  to update.
- **Variables in the envelope use quoted keys** (`"?p"`) so the YAML
  parser doesn't choke on the `?` sigil.
- **Multiple expressions in one input become multiple documents** in
  the matches section, each headed by its own concept name. `---`
  separators give the agent a stable split.

For pure-mutation documents, the matches section is omitted entirely
— the envelope alone is the whole output. For pure-query documents,
`revision-before == revision-after` and `claims: 0`.

For `--quiet`, slide emits only the envelope.

### JSON format

Same shape as the worker's `EvaluateResponse`, exactly:

```jsonc
{
  "revision_before": "…",       // null on first write
  "revision_after":  "…",       // == revision_before for read-only docs
  "matches_before": [
    { "label": "person",
      "results": [
        { "this": "did:key:zHj…",
          "fields": { "name": "Alice", "age": 28 } } ] } ],
  "matches_after": [ /* same shape, post-commit */ ],
  "commits": {
    "claims": 2,
    "entities": { "alice": "did:key:zHj…", "?p": "did:key:zHj…" }
  }
}
```

Reusing the worker's type verbatim means a future "slide as a thin
local-mode wrapper around tonk-worker" refactor is mechanical.

### Exit codes

| Code | Meaning                                             |
|------|-----------------------------------------------------|
| 0    | success                                             |
| 1    | parse error — diagnostics printed to stderr         |
| 2    | analyzer error — schema lookup / unbound var / etc. |
| 3    | commit error — dialog rejected the transaction      |
| 4    | I/O / repo-not-found / identity error               |

Agents can branch on these without parsing stderr. Diagnostics are
emitted in `<source>:<line>:<col>: <message>` form, which matches
what editors and `clippy` produce — the agent's training data is
saturated with this format.

## Architecture

### Crate layout

```
rust/slide/
├── Cargo.toml
├── README.md
├── RFC.md                          # this file
└── src/
    ├── bin/slide.rs                # clap entry point
    ├── lib.rs                      # re-exports for tests / future SDK
    ├── eval.rs                     # the analyze→query→plan→commit pipeline
    ├── site.rs                     # .tonk/ discovery + open + init
    ├── migrate.rs                  # .carry/ -> .tonk/ one-shot migration
    ├── identity.rs                 # local profile management
    ├── output.rs                   # render EvaluateResponse → notation / json
    ├── schema_cmd.rs               # `slide schema` introspection
    └── guide.rs                    # include_str!("../../tonk-notation/guide.md")
```

Aiming for ~6 source files plus tests. Well under half of carry's
surface.

### Dependencies

Take from carry's set, drop everything sync-related:

Keep:
- `tonk-notation`, `tonk-schema`, `tonk-common`
- `dialog-artifacts`, `dialog-credentials`, `dialog-effects`,
  `dialog-operator`, `dialog-query`, `dialog-repository`,
  `dialog-storage`
- `clap` (derive), `tokio`, `anyhow`, `serde`, `serde_json`,
  `serde_yaml`, `dirs`

Drop:
- `tonk-invite`, `tonk-access-service`, `dialog-remote-s3`,
  `dialog-remote-ucan-s3`, `dialog-ucan*`, `dialog-varsig`, `url`,
  `clap_complete`, `dialoguer`, telemetry crates.

### Reusing the worker's pipeline

`tonk-worker/src/router/evaluate.rs` already contains the analyzer
driver, the per-expression query runner, the natural-join logic, the
plan-and-commit loop, and the response shaping. Slide should not
re-implement this — the cleanest move is to **lift the non-axum half
of evaluate.rs into a new module `tonk_schema::evaluate` (or a new
`tonk-evaluator` crate)** so that both the worker route and `slide
eval` are thin adapters over it.

Concretely:

- The worker route becomes: parse body → call `evaluator::run(syntax,
  branch, operator)` → wrap in `Json(...)`.
- Slide's `eval` becomes: read source → call the same
  `evaluator::run` → render with the chosen formatter.

This refactor is the only non-trivial cross-crate change slide
requires; the rest is plumbing. We can do slide v0 by *copying* the
relevant code into `slide/src/eval.rs` and only extracting later if a
third caller appears, but I'd prefer to extract first since we know
both callers exist.

The shared evaluator should keep `EvaluateResponse` /
`QueryMatchBlock` / `QueryResult` / `CommitSummary` as its public
output type — slide and worker stay schema-compatible by construction.

### Repo on-disk format

`.tonk/` mirrors `.carry/` byte-for-byte (single dialog repository
named `main` under a `Directory::At(.tonk)` operator base, identity
in the platform data dir). The directory name differs only because
having two CLIs claiming `.carry/` would be ambiguous. They are not
guaranteed to be cross-compatible — that's a future decision driven by
real demand, not by speculative interop work.

## Phasing

Each phase is shippable on its own.

**Phase 0 — extraction.** Move the non-axum parts of `evaluate.rs`
into a shared module so slide and the worker share one evaluator.
Worker keeps passing tests. No new behaviour.

**Phase 1 — `slide eval` against a local repo.** New `rust/slide/`
crate. Implements `init`, `eval`, `identity`. Notation output (the
default) plus `--format json`. Integration tests cover: empty-branch
attribute declaration, concept declaration, assert-then-query round
trip, multi-expression join, retraction by query result, parse-error
/ analyzer-error / commit-error exit codes, round-trip-stability of
`slide eval -c '<query>'` output back into another `slide eval`.

**Phase 2 — agent-friendliness polish.** `slide guide`, `slide
schema`, `slide migrate`, `--dry-run`, `--quiet`. Add a snippet in
the README that an agent harness can paste verbatim into a system
prompt.

**Phase 3 — evaluation.** Run a real coding-agent harness through a
small task corpus (e.g. "model a TODO list", "model a small
relational schema and query it") and measure:
   - first-attempt parse-success rate,
   - tool calls per task,
   - whether the agent uses `slide schema` / `slide guide`
     unprompted.

This is the real test of whether the notation-only surface is the right
call. If agents consistently struggle, we iterate — most likely by
adding more guide snippets, not more subcommands.

## Open questions

1. **Should `slide schema` round-trip to a runnable document, or just
   describe?** Round-trippable is more useful (the agent can grep its
   own output for the right `the:` URI), but introduces a maintenance
   contract: every notation feature must be expressible in
   `slide schema`'s output too. Probably worth it; flag for review.

2. **Do we want `slide eval --watch <file>`?** Re-run on file change.
   Useful for an "edit the YAML in vim, see results immediately" loop,
   but adds a dependency (`notify`) and isn't strictly agent-relevant.
   Defer.

3. **Telemetry.** Carry has a telemetry sidecar. Slide should ship
   without one and not add it without a specific motivating use case.

4. **Ergonomic shortcut for `slide eval -c "<doc>"`.** SQL CLIs let you
   pipe the doc on stdin without `-c`. Slide already does that. The
   open question is whether `slide "<doc>"` (no subcommand, treat first
   arg as a notation doc if it starts with `<word>:` or contains a
   newline) is worth the parsing fragility. Tentative answer: no — the
   savings aren't worth the heuristic.

5. **Notation envelope vs. matches separator.** The current proposal
   uses a `---` YAML document marker between envelope and matches.
   That requires the agent's parser to handle multi-document YAML
   (saphyr does; not all do). An alternative is to put the envelope
   under a top-level `meta:` key and the matches under `results:`,
   producing a single YAML document — but that breaks the property
   that each match is a free-standing notation expression. Lean
   toward `---`; flag for review once we see real agent harnesses
   try to consume the output.

## Appendix — worked example

What an agent harness sees, end to end. Notation format throughout
(default).

```
$ slide init project
Initialized project repository in /work/.tonk
DID: did:key:zHjK...

$ slide eval -c '
attribute! task-title:
  the:         xyz.tonk.task/title
  as:          Text
  cardinality: one

attribute! task-done:
  the:         xyz.tonk.task/done
  as:          Boolean
  cardinality: one

concept! task:
  with:
    title: .task-title
    done:  .task-done
'
revision-before: ~
revision-after:  rev:abc...
claims:          11
entities:
  task:       did:key:zT...
  task-title: did:key:zU...
  task-done:  did:key:zV...

$ slide eval -c '
task! buy-milk:
  title: "Buy milk"
  done:  false
'
revision-before: rev:abc...
revision-after:  rev:def...
claims:          2
entities:
  buy-milk: did:key:zW...

$ slide eval -c 'task ?t: done: false'
revision-before: rev:def...
revision-after:  rev:def...
claims:          0
entities:
  "?t": did:key:zW...
---
task did:key:zW...:
  title: "Buy milk"
  done:  false
```

The third call's output is itself a valid query expression. To mark
that task done, the agent flips one character (head `:` -> `!:`) and
edits one field:

```
$ slide eval -c '
task! did:key:zW...:
  title: "Buy milk"
  done:  true
'
```

Three calls; no flags beyond `-c`; one DSL in and out. That's the
bar.
