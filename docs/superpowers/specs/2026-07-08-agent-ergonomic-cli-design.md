# Agent-ergonomic tonk-cli: an argument-based command surface

Date: 2026-07-08
Status: approved design, pre-implementation

## Problem

tonk-cli's only mutating verb is `eval`, which runs a bespoke multi-line DSL
(asserted-notation) through the analyze→commit pipeline. This is the extreme
case of what Microsoft's "Don't rewrite your CLI for agents" measures: agents
must both learn a language absent from their training AND cram it through the
shell via `eval -c '…'` heredocs, hitting every failure mode the study names
(syntax validation, nesting validation, field/type matching, shell escaping),
plus a learn-the-language tax on top.

Bench baselines (codex/gpt-5.5, this repo) corroborate:

- **targeted-edit 9/10** — clean, but the one friction is pure DSL tax: the
  agent "read the full guide and the notation guide before making what is a
  one-line edit."
- **interview-build 3/10** — a strong interview, but the artifact tanked on
  (a) DSL notation-validation rejections (`attribute!` missing description, a
  rule failing to bind `this`) and (b) the render-gap: the built thing never
  surfaced on the space home.
- **cold-onboard 7/10** — confounded by a bench harness artifact (see
  Adjacent fix 1), but showed the same install/orientation friction.

The article's evidence-based direction: constrained, argument-based interfaces
beat structured payloads for agents on correctness, cost (4–11× fewer tokens),
and reliability — and the causal factor is *fixed flag names the agent reads
from `--help`*, not "args" in the abstract.

## Design basis (decided during brainstorming)

- **Coverage:** the arg surface reaches the whole loop — instance data, schema
  authoring (concepts/attributes), view authoring, and the home re-point — not
  just data CRUD. The schema/view-authoring side carried the most baseline
  friction.
- **Flag shape:** schema-aware typed flags for the data verbs (fixed,
  `--help`-discoverable from the branch schema), and a fixed *enumerated*
  grammar for the authoring verbs (types/cardinalities as validated enums).
  `key=value` is explicitly rejected — it forfeits the fixed-flags-from-`--help`
  property that makes args win.
- **Conventional verbs:** `add`/`set`/`get`/`list`/`rm`/`describe`/`home` —
  git/kubectl/docker-shaped, so an untrained model's priors transfer.
- **Enumerating errors:** every error names the valid options (unknown field →
  the concept's fields; bad type → the valid types).
- **`--json` output** on reads; the stable per-stage exit codes already exist.
- **Auto-surface + explicit override** for the render-gap (below).
- **Additive, not a rewrite:** `eval` stays as the escape hatch.

## Architecture: a thin constrained front-end over the eval pipeline

Every verb assembles an asserted-notation document *in-process* and runs it
through the same `tonk_evaluator::evaluate` (analyze → query → plan → commit)
path `eval` already uses. There is no second write path and no new source of
truth — the arg surface is purely an input-safety layer over tested machinery
(validation, effects, auto-sync). This keeps `eval` as the escape hatch for
anything the verbs don't cover and means the verbs inherit sync/commit
semantics for free.

New code lives in a verb module set (e.g. `rust/tonk-cli/src/verbs.rs` plus
`verbs/` per-verb files, following the repo's `foo.rs` + `foo/` convention and
no-`mod.rs` rule). Each verb: (1) opens the site, (2) for schema-aware verbs,
reads the target concept's attributes from the branch, (3) builds a notation
string, (4) calls the existing eval runner, (5) renders a concise result
(human default, `--json` optional).

## Verb surface

### Data (schema-aware typed flags)

- `tonk add <concept> --<field> <value> …` → `<concept>!: { <field>: <value> … }`
- `tonk set <entity> --<field> <value> …` → query-bind the entity, then assert
  a cardinality-one overwrite of each field
- `tonk get <entity>` → the entity's fields (human or `--json`)
- `tonk list <concept>` → all instances (human table or `--json`)
- `tonk rm <entity> [--field <f>]` → retract one field (`<f>: _`) or the whole
  instance (`..: _`)

The typed flags are derived from the concept's `with:` attributes on the
branch, so `tonk add habit --help` lists `--name`, `--target`, etc. Outside a
`.tonk/`, `--help` degrades to generic guidance rather than erroring.

### Authoring (fixed enumerated grammar)

- `tonk concept add <name> --attr <name>:<type>:<card> [--attr …]
  [--description <text>]` — `<type>` ∈ the schema's value kinds
  (text, entity, integer, float, unsigned-integer, …), `<card>` ∈ {one, many};
  both validated with enumerating errors. Emits the `attribute!:`/`concept!:`
  block (with the required descriptions the analyzer demands, so the agent
  can't hit the "missing description" rejection).
- `tonk view add <model> --template <html> | --template-file <path>` — authors
  a `tonk:view` instance for the model; auto-surfaces (below).
- `tonk home <model>` — pin the space home explicitly.

### Discovery + errors

- `tonk describe <concept>` — lists a concept's fields, types, cardinalities
  and descriptions (the flag list an agent needs). May be folded into a richer
  `tonk concepts <name>`.
- Every failure enumerates the fix: unknown field → valid fields; bad type →
  valid types; bad cardinality → {one, many}; unknown concept → nearest matches.
- `--json` on the read verbs; per-stage exit codes unchanged.

## Auto-surfacing the build (render-gap)

The space home renders `<tonk-display entity={replica} model=tonk/space />`,
and `tonk/space` is a cardinality-one `name!` alias defaulting to `tonk:blank`.
Templates re-point it to an origin-keyed root concept (the `tonk:binder`
pattern). Agents never do this, so their builds don't surface — confirmed in a
real interview-build episode even with the full stdlib present.

Verified minimal recipe (headless `tonk render` + live browser):

```yaml
concept!: &<ns>/home
  this: <ns>:home
  description: <one line>
  with:
    subject:
      description: The repository's subject DID.
      the: dialog.origin/subject
      as: entity

view!: &<ns>/home-view
  this: id:<ns>:home/view
  model: <ns>:home
  display: |
    <tonk-display model=<data-concept> />   # no entity => directory render

name!:
  this: id:tonk/space
  entity: <ns>:home
```

Load-bearing facts (from verification): the root concept's field must map to
`dialog.origin/subject` `as: entity` (the only attribute already present on the
replica entity the home route renders); a concept can't omit `with:`; the field
name is irrelevant; `<tonk-display model=X />` with no `entity=` renders X's
directory and composes for multiple models; `tonk:view` is already seeded on a
fresh `tonk init` (so no extra seeding needed); a naive re-point straight to a
data concept throws "Concept mismatch: required attribute missing".

Behavior: `tonk view add <model>` (when no explicit home is set) and
`tonk home <model>` (always) author this recipe for the agent and re-point
`tonk/space` (last-writer-wins). The CLI prints the live URL
(`your build is live at /space/<repo>/`). The agent never learns the alias
exists.

Known follow-up to confirm during implementation: the verification run saw the
live directory fall back to "No view for <model>; showing the default" per item
even though headless `tonk render` used the custom view — a likely reactor-side
gap in custom per-item view resolution, independent of the re-point mechanic.
Confirm the authored home surfaces the intended custom presentation, not just a
default render.

## Adjacent fixes (small, independently landable)

### 1. Seed reaches the remote

`tonk init` seeds the stdlib as a *local* commit before any upstream exists; a
committing eval only auto-pushes when an upstream is already set, so the seed
never pushes. A repo created via `tonk init` and shared over the CLI hands the
joiner a branch missing `tonk:view` and the routing/view infrastructure (the
joiner then hand-authors it — exactly the cold-onboard friction).

Fix: `tonk invite --remote` / `tonk share` push the local branch before minting
the invite, so the joiner gets current state including the seed. (You are
sharing — syncing first is the least-surprise behavior.) Plus the bench harness
fix: `bench/bin/site.sh setup` pushes after `set-upstream`, so cold-onboard
mirrors a real web-created space (where the service worker seeds `core.yaml`
onto the remote) instead of an artificially barren branch. cold-onboard is then
re-baselined for a clean "before".

### 2. Dropped items (both premises disproven at implementation time)

Two candidate fixes were investigated and **dropped as non-issues** after
direct verification. Recorded here so they are not re-attempted:

- **"`tonk render` fails loudly on a missing view"** — already true. `tonk
  render` exits non-zero (code 4) with no output on any resolution failure.
  The "Model not found" banners in the baseline were the web UI's full space
  page, not the CLI.
- **"Authored concepts aren't addressable by name"** — false for anchored
  creation. A notation anchor (`concept!: &habit`) publishes the
  `dialog.name/referent` claim (`schema.rs:197`), and `tonk render habit`
  resolves it (verified: 1369 bytes of rendered data against the known-good
  targeted-edit seed). The earlier "no concept matched" result came from a
  malformed scratch `attribute!` block that never created the concept — a test
  artifact, not a product bug. The bench's "tonk-created concepts have no Name
  claim" note is stale for anchored creation.

Consequence for the authoring verbs: `tonk concept add` must **anchor** the
concepts it generates (`concept!: &<name>`), so name-addressability comes for
free — no separate `Name`-claim assertion is needed.

## Sequencing (smallest-first; each phase re-baselines on the bench)

1. **First PR — seed reaches the remote** (the one verified prerequisite):
   `tonk invite --remote`/`tonk share` push before minting, plus the bench
   `site.sh` push, then a cold-onboard re-baseline. Small, standalone.
2. **Data verbs** (`add`/`set`/`get`/`list`/`rm`) + `describe` + enumerating
   errors + `--json` — the core agent win. Establishes the verb-module pattern
   (thin front-end over `eval`) later verbs reuse.
3. **Authoring verbs** (`concept add` — anchoring its concepts so they resolve
   by name — and `view add`) + auto-surface + `tonk home`.

Both dropped phase-1 candidates (render-fails-loudly, name-addressability) are
non-issues — see "Dropped items". Each phase re-runs the relevant bench
scenarios to measure the delta against the eval-only baselines.

Each phase re-runs the relevant bench scenarios to measure the delta against
the eval-only baselines (targeted-edit's one-line edit should collapse from
"read two guides" to a single `tonk set`; interview-build's build should
auto-surface).

## Testing

- **Unit:** each verb builds the expected notation — assert the generated
  document against a golden string — and round-trips through `eval` on a scratch
  site (assert the resulting branch state). Cover enumerating-error paths
  (unknown field/type/cardinality/concept).
- **Integration:** the bench scenarios re-run per phase; the deltas are the
  acceptance evidence.
- Repo test style throughout: `#[dialog_common::test]`, `it_does_x` names,
  grouped by behavior; shared helpers via `tests/common.rs` with `autotests =
  false` + explicit `[[test]]`. No `mod.rs`.

## Non-goals

- Replacing or deprecating `eval` (it remains the escape hatch).
- A parallel write path that bypasses the analyze→commit pipeline.
- The npm distribution / invite-prompt copy work (separate effort).
- Rewriting the notation grammar itself.

## Open questions (resolve at implementation time)

- Whether the schema-aware `--help` is true clap-dynamic command generation or
  hand-rolled concept-specific parsing after a schema read — the ergonomic
  requirement (fixed fields in `--help`, enumerating errors) is fixed; the
  mechanism is an implementation choice.
- Exact value-kind enum names to expose in `--attr <name>:<type>:<card>` (read
  from `tonk-schema` so the CLI and analyzer agree).
- Whether `tonk describe` is a new verb or an extension of `tonk concepts`.
- The reactor per-item-view fallback (above) — confirm/repair so auto-surfaced
  homes render the authored view, not the default.
