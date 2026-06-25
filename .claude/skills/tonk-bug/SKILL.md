---
name: tonk-bug
description: Submit (and query/update) a bug in a tonk space using the `tonk`/`slide` CLI and asserted-notation. Use whenever asked to file, report, triage, or list bugs in a tonk repo — NOT the `terma:bug-report` skill (that writes BUGS.md). Covers the `bug` concept schema, value conventions, the next-ident lookup, and the create/retract notation that actually works against this CLI.
allowed-tools: Read, Bash(tonk:*), Bash(slide:*), Bash, Glob, Grep
---

# Submitting bugs to a tonk space

`tonk` (aliased as `slide`) is a notation-driven CLI. It is **not** the
`tonk --json concept define / create / query` interface described in the
plugin `tonk` guide — that flag-based interface does not exist on this
binary. Everything goes through `tonk eval` running an asserted-notation
document. Run `tonk guide notation` if you need a syntax refresher.

## Orient first

```bash
tonk status                 # synced / ahead / behind / diverged / no-upstream
tonk concepts | grep bug    # confirm the `bug` concept exists on this branch
tonk schema | grep -A8 'concept!: &bug'   # see the live field list
```

## The `bug` concept

Fields (body shorthands map to `squash.bug/*` attributes via the concept):

| field      | type           | convention                                   |
|------------|----------------|----------------------------------------------|
| `ident`    | Text           | `"BUG-N"` — N is the next integer            |
| `title`    | Text           | short summary                                |
| `detail`   | Text           | full description, repro steps, hypotheses    |
| `priority` | Text           | `low` \| `medium` \| `high`                  |
| `status`   | Text           | `triage` \| `todo` \| `done` \| `canceled`   |
| `ordering` | SignedInteger  | numeric, matches N (e.g. `15`)               |
| `assignee` | Text           | a `did:key:…` or empty string `""`           |

New bugs are typically `status: "triage"`, `assignee: ""`.

## Pick the next ident / ordering

`ident` and `ordering` are NOT auto-assigned — compute the next one.
Sort numerically (don't lexically sort the float column):

```bash
tonk eval -c 'bug:
  this: ?b
  ordering: ?o' 2>&1 | grep -oE 'ordering: [0-9.]+' | grep -oE '[0-9.]+' | sort -n | tail -1
```

Next ident = `BUG-<max+1>`, next ordering = `<max+1>`.

## Submit a bug

A committing `eval` auto-syncs (pull-before / push-after). Use a `&anchor`
on the head to mint and name the entity; quote all Text values. Use a raw
multi-line literal for the document, not `\n` escapes.

```bash
tonk eval -c 'bug!: &bug-15
  ident: "BUG-15"
  title: "Short summary of the problem"
  detail: "Observed behavior, steps to reproduce, hypothesis, suggested fix."
  priority: "medium"
  status: "triage"
  ordering: 15
  assignee: ""'
```

The response echoes `entities: { bug-15: did:key:… }` (the new entity id)
and `claims: 7` (one per field). Then confirm `tonk status` → `synced`.

For long, structured detail, write the document to a file and pass the
path, which avoids shell-quoting pain:

```bash
tonk eval /path/to/bug.tonk
# or pipe:  cat bug.tonk | tonk eval -
```

## Query bugs

> **Concepts have no optional fields.** A concept query matches an entity
> only if it carries **every** attribute in the concept's `with:` map. A
> bug missing even one field (e.g. blank `assignee` was never asserted)
> will NOT appear in a `bug:` query. To find such entities, query by
> **domain** instead (an ad-hoc query that only constrains the attributes
> you name, ignoring the rest):

```bash
# Domain query — matches any entity with these squash.bug/* attributes,
# regardless of which other bug fields it has or lacks.
tonk eval -c 'squash.bug:
  this: ?bug
  title: ?title
  status: ?status'
```

```bash
# Concept query — only matches bugs that have ALL bug fields.
tonk eval -c 'bug:
  this: ?b
  ident: ?ident
  title: ?title
  priority: ?priority
  status: ?status'

# Filter by a constant (only triage bugs that have all fields)
tonk eval -c 'bug:
  this: ?b
  ident: ?ident
  status: "triage"'
```

Rule of thumb: use the **concept** query for fully-formed bugs; drop to a
**domain** (`squash.bug:`) query when you might be missing fields or want
to scan loosely.

## Resolve a person for `assignee`

`assignee` is the `did:key:…` of the **person**, not the CLI operator and
not a member handle. When asked to "assign to <name>", resolve the person
entity first — don't guess and don't use `tonk identity` (that's the
device/operator key).

```bash
# The person entity carries squash.person/name = the full display name.
tonk eval -c 'squash.person:
  this: ?p
  name: ?name'
```

Match the requested name to a `this:` did. Watch out:

- **`tonk identity`** returns the operator/device did — wrong for
  assignment.
- The **`member`** concept carries space-local handles only (`gozala`,
  `gozala.firefox`, `gozala.safari`, …), one per replica, NOT the full
  name. Don't assign to a member entity.
- The full display name (e.g. `"Irakli Gozalishvili"`) lives on
  `squash.person/name`. Use that entity's `this:` did as the `assignee`.

If the name doesn't appear, search the dump: `tonk export | grep -i <name>`.

## Update a bug

Join a query that binds the entity to an assertion against that variable.
For each match of `?b`, the assertion fires once.

```bash
tonk eval -c 'bug:
  this: ?b
  ident: "BUG-15"

bug!:
  this: ?b
  status: "done"
  assignee: "did:key:z6Mk…"'
```

## Retract a field / delete a bug

Blank value `_` retracts. `field: _` drops one attribute; `..: _` drops
every attribute in the concept's `with:` map (an effective delete).

```bash
tonk eval -c 'bug:
  this: ?b
  ident: "BUG-15"

bug!:
  this: ?b
  ..: _'
```

## Adding your own attributes / extending the schema

You can declare a new attribute and assert it on bugs without touching the
`bug` concept. Example — let bugs depend on other bugs:

```bash
tonk eval -c 'attribute!: &bug/dependency
  description: Allows you to associate dependency of this bug
  the: squash.bug/dependency
  as: entity
  cardinality: many'
```

Then assert it on a bug (cardinality `many` → assert the field repeatedly
or with multiple entities), and query it via the domain:

```bash
# Find bugs that declare a dependency.
tonk eval -c 'squash.bug:
  this: ?bug
  dependency: ?dep'
```

You could also `concept!:` a new concept whose `with:` is all the bug
fields plus `dependency` — but because concepts require every field, that
concept query will surface ONLY bugs that have a dependency. For "bugs
with X, ignore the rest," the domain query above is usually what you want.

## Gotchas

- **No `--json`, no `create`/`query`/`concept define` subcommands.** Those
  belong to a different (plugin) reference; this binary rejects them.
- **Quotes are load-bearing.** `priority: high` is a *symbol* resolved
  through the name table (usually wrong); `priority: "high"` is the literal
  string you want. Quote all Text values.
- **To preview without writing, use `--dry-run`** (not `--no-sync`).
  `tonk eval --dry-run …` runs the analyzer + queries and returns matches
  (and the entity it *would* mint), but drops the transaction: zero claims,
  unchanged revision, branch left `synced`. It implies `--no-sync`, so it
  never touches the remote. `--no-sync` by contrast still **commits
  locally** and leaves the branch diverged from upstream until you
  `tonk pull` / `tonk push`.
- **`ident`/`ordering` are manual.** Compute the next number before
  asserting; don't assume they increment automatically.
- **`assignee` is a person, not the operator.** Resolve it via the
  `squash.person` query (see *Resolve a person for `assignee`*); never use
  `tonk identity` or a `member` handle.
- A re-asserted identical body is a no-op (same content-addressed entity);
  changing any field mints a new entity unless you target an existing one
  via `this: ?b` from a query join.
