# Dialog-native data verbs: assert / retract / query

Date: 2026-07-13
Status: approved design (revised after adversarial review), pre-implementation

## Problem

The data verbs shipped in PR2 (`add`/`set`/`get`/`list`/`rm`/`describe`) are
git/kubectl-shaped. That was deliberate — conventional verbs transfer an
untrained model's priors — but `rm` actively mis-teaches the data model:
dialog never deletes, it asserts a superseding claim. The domain-conventional
verbs are `assert` and `retract`, and using them teaches the correct mental
model instead of a lie. This is not abandoning "conventional verbs"; it is
picking the verbs conventional to this domain.

Dialog has exactly two mutation operations on claims — assert and retract —
so the current `add`/`set` split (new instance vs field overwrite) is two
spellings of the same operation. The rename collapses them.

## Verb mapping

Replace, no aliases. Nothing external depends on the PR2 names — they shipped
days ago on the same unmerged branch (`feat/agent-build`, PR #575), and
aliases would dilute the one-obvious-verb property.

| Today | Becomes | Grammar |
|---|---|---|
| `add <concept> --f v` + `set <concept> <entity> --f v` | **`assert`** | `tonk assert <concept> [<entity>] --f v …` |
| `rm <concept> <entity> [--field f]` | **`retract`** | `tonk retract <concept> <entity> [--field f]` |
| `list <concept>` | **`query`** | `tonk query <concept> [--json]` |
| `get <concept> <entity>` | `get` (unchanged) | `tonk get <concept> <entity> [--json]` |
| `describe <concept>` | folded into **`schema`** | `tonk schema [<concept>]` |

All verbs stay **concept-first** — the concept is load-bearing (`assert`
needs it to read the schema and build the typed flags; the mint form has no
entity yet), and one consistent grammar beats an entity-only `retract` that
would need concept inference and diverge from `assert`.

## Behavior

### `assert` — the one write verb

`tonk assert <concept> [<entity>] --field value …`

- No entity positional → mint a new instance (today's `add`): every
  non-optional field required, schema-derived typed flags.
- Entity positional present → assert superseding cardinality-one claims on
  that entity (today's `set`): every field optional, only named fields change.

Disambiguation: an entity reference never starts with `--`. The handler keeps
`disable_help_flag` + `trailing_var_arg` + `allow_hyphen_values` on a single
`rest: Vec<String>` and splits it manually — a leading non-`--` token is
**always** the entity; everything else routes to the dynamic per-concept
parser exactly as today. This avoids clap's finicky
optional-positional-before-var-arg behavior. `tonk assert <concept> --help`
renders the mint help (all fields, required markers); `tonk assert <concept>
<entity> --help` renders the supersede help (all fields optional).

**The supersede form requires the entity to exist.** Today's `set` emits
`this: <entity>` with no existence check (`data.rs` `build_set`), so a
typo'd entity silently mints a partial orphan — bypassing the required-field
validation that is the mint form's headline invariant. Under `assert`, where
argument shape is the only thing separating the two forms, that backdoor
must close: the supersede handler first queries for an existing `<concept>`
instance at `<entity>` and fails with

    error: no <concept> instance at '<entity>'; run `tonk query <concept>`
    to see what exists

when none matches — a new `DataOpError::NoInstance` variant, mapped
alongside `NoConcept` in the exit-code table. A misplaced value token (a bare word where flags were
intended) hits the same error — the accepted failure mode for the ambiguous
leading token.

The mirror failure gets error copy too: an agent intending supersede who
forgets the entity hits the mint form's missing-required-flag error, which
must append the hint "to update an existing instance, pass the entity before
the flags: tonk assert <concept> <entity> --<field> <value>".

**Cardinality-many fields.** Asserting on a many-cardinality attribute
appends a value in dialog; it does not supersede. The typed-flag layer
currently ignores `.cardinality()` (single-value clap args, no `Append`),
and the analyzer's actual many-behavior through these builders is untested.
The implementation locks this with tests: one `--<field> <value>` per
invocation asserts one value on a many field (mint or supersede); a repeated
flag is rejected by clap as today; `retract --field <f>` retracts **all**
values of the field. `assert <concept> --help` marks many-cardinality fields
as such. If the analyzer turns out not to append, that is surfaced as its
own finding, not silently papered over.

### `retract`

`tonk retract <concept> <entity> [--field <f>]` — same semantics as today's
`rm`: `--field` retracts one attribute (`<f>: _`), omitting it retracts the
whole instance (`..: _`). Help/docs text says "retract", never
"remove"/"delete", and notes that retraction is itself an assertion — a claim
invalidating an old one — not a deletion.

### `query`

`tonk query <concept> [--json]` — today's `list`, renamed. Reads are queries
in dialog.

Known cost, accepted deliberately: `query` imports a filter-expression prior
(SQL, GraphQL), and this form takes none. Filter flags (e.g.
`--where <field>=<value>`) are the intended future direction, so the prior
becomes a roadmap rather than a lie. Until then, a non-concept first
argument (`tonk query 'done=false'`) falls into the existing enumerating
NoConcept error, which names the branch's real concepts and the usage line.

### `get`

Unchanged. Universally understood, not misleading.

### `schema [<concept>]`

`tonk schema` (no arg) keeps its current behavior: the whole branch as a
re-submittable notation document. `tonk schema <concept>` emits **the same
format, filtered to one concept** — that concept's `concept!:` block plus
the `attribute!:` declarations it references, still re-submittable. `--json`
is not accepted (notation is the format, matching the bare form).

`describe`'s human field/type/cardinality table is dropped, not moved: that
job is already done by `tonk assert <concept> --help`, the schema-derived
dynamic help where the flags actually live. One fewer verb, no format
inconsistency.

## What does not change

Pure front-end rename. Untouched: the notation builders (`data.rs`), the
eval pipeline, the retraction-scope fix in tonk-analyzer, the dynamic
typed-flag machinery (`data_ops/flags.rs`), enumerating errors, `--json`
rendering, exit codes, auto-sync.

Internal `data_ops` functions are renamed to match the surface
(`assert_op`-style naming as needed to dodge the `assert` keyword-adjacent
name, `retract`, `query`) — cosmetic, for coherence. All user-facing copy
adopts the vocabulary: the zero-flags rejection ("set needs at least one
--field" → "assert with an entity needs at least one --field"), the
added/updated/removed result lines, and `retract`'s help text, which says
"retract" — never "remove" or "delete" — and notes that a retraction is
itself an assertion invalidating an old claim, not a deletion.

## Blast radius

- `rust/tonk-cli/src/bin/tonk.rs` — `Command` variants (`Add`/`Set` →
  `Assert`; `Rm` → `Retract`; `List` → `Query`; `Describe` removed; `Schema`
  gains an optional positional), handlers, `descriptor()` telemetry strings.
- `rust/tonk-cli/src/data_ops.rs` — fn renames; `add`+`set` merge into one
  entry point that branches on entity presence.
- `rust/tonk-cli/tests/data_verbs.rs` — renamed invocations; new coverage for
  the `assert` mint-vs-supersede split and `schema <concept>`.
- `rust/tonk-cli/README.md` — data-verbs block rewritten with the dialog
  framing.
- `rust/tonk-cli` guide text + top-level `after_help` — verb references
  updated; grep `bench/` scenario `task.md`s for stale verb mentions (rubrics
  are frozen; task.md edits only if they name the old verbs).
- `.claude/commands/tonk.md` — currently documents a wholly fictional CLI
  (`tonk --json create/query/show/update/delete`, `space create`, `concept
  show/delete`). Not a rename casualty, but any agent in this repo that
  loads it will fight whatever verbs ship — rewritten to the real surface as
  part of this work.
- `docs/superpowers/plans/2026-07-08-data-verbs.md` and
  `docs/superpowers/specs/2026-07-08-agent-ergonomic-cli-design.md` — both
  name the old verbs pervasively. Historical documents: each gets a
  "superseded by this spec" note at the top rather than a rewrite.

## Testing

Existing `data_verbs.rs` suite carries over under the new names — including
the raw-claims retraction-scope assertions. New cases: `assert` with no
entity mints; `assert` with an entity supersedes and leaves siblings;
`assert` with a nonexistent entity fails with the existence error (the
backdoor test); the missing-required-flag error carries the supersede hint;
many-cardinality assert appends and `retract --field` clears all values
(raw-claims oracle); `schema <concept>` emits the re-submittable notation
subset; `schema` with no arg unchanged. Repo style throughout:
`#[dialog_common::test]`, `it_does_x`, `mod when_…`.

**Bench re-baseline:** the 07-08 spec's per-phase discipline applies to this
rename too — the verbs it renames were chosen on measured evidence, so the
replacement gets measured. After landing, re-run `targeted-edit` and
`interview-build` to confirm the domain verbs don't cost prior-transfer
discoverability. The frozen rubrics don't name the old verbs, so re-runs
aren't skewed.

## Non-goals

- Aliases or deprecation shims for the PR2 names.
- Entity-only `retract` (concept inference).
- Value-level retraction on many-cardinality fields
  (`retract … --field f --value v`) — dialog supports claim-level
  retraction, but no use case demands it yet; the surface says so in
  `retract --help` rather than pretending `--field` is value-precise.
- `--where` filter flags on `query` (named as the future direction, not
  built now).
- Any change to `eval`, the notation grammar, or the authoring verbs planned
  for PR3. PR3's authoring verbs stay **noun-first** (`tonk concept add`,
  `tonk view add`) — `tonk assert concept …` would collide with the data
  grammar (a concept named `concept`), so authoring never rides the
  `assert` verb.
