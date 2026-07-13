# Dialog-native data verbs: assert / retract / query

Date: 2026-07-13
Status: approved design, pre-implementation

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
`rest: Vec<String>` and splits it manually — a leading non-`--` token is the
entity; everything else routes to the dynamic per-concept parser exactly as
today. This avoids clap's finicky optional-positional-before-var-arg
behavior. `tonk assert <concept> --help` renders the mint help (all fields,
required markers); `tonk assert <concept> <entity> --help` renders the
supersede help (all fields optional).

### `retract`

`tonk retract <concept> <entity> [--field <f>]` — same semantics as today's
`rm`: `--field` retracts one attribute (`<f>: _`), omitting it retracts the
whole instance (`..: _`). Help/docs text says "retract", never
"remove"/"delete", and notes that retraction is itself an assertion — a claim
invalidating an old one — not a deletion.

### `query`

`tonk query <concept> [--json]` — today's `list`, renamed. Reads are queries
in dialog.

### `get`

Unchanged. Universally understood, not misleading.

### `schema [<concept>]`

`tonk schema` (no arg) keeps its current behavior: the whole branch as a
re-submittable notation document. `tonk schema <concept>` prints the
one-concept field/type/cardinality/description view `describe` renders today.
One fewer verb; `schema` is already where an agent looks.

## What does not change

Pure front-end rename. Untouched: the notation builders (`data.rs`), the
eval pipeline, the retraction-scope fix in tonk-analyzer, the dynamic
typed-flag machinery (`data_ops/flags.rs`), enumerating errors, `--json`
rendering, exit codes, auto-sync.

Internal `data_ops` functions are renamed to match the surface
(`assert_op`-style naming as needed to dodge the `assert` keyword-adjacent
name, `retract`, `query`) — cosmetic, for coherence.

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

## Testing

Existing `data_verbs.rs` suite carries over under the new names — including
the raw-claims retraction-scope assertions. New cases: `assert` with no
entity mints; `assert` with an entity supersedes and leaves siblings; a
non-entity second token that looks like a value is handled sanely; `schema
<concept>` matches old `describe` output; `schema` with no arg unchanged.
Repo style throughout: `#[dialog_common::test]`, `it_does_x`, `mod when_…`.

## Non-goals

- Aliases or deprecation shims for the PR2 names.
- Entity-only `retract` (concept inference).
- Any change to `eval`, the notation grammar, or the authoring verbs planned
  for PR3 (those will adopt this vocabulary when built).
