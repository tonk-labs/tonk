# tonk CLI — Agent Reference

tonk is a headless CLI for reading and writing data and views in a spot
(a named local dialog repository). Data lives as claims: you **assert**
claims and **retract** them — a retraction is itself an assertion that
invalidates an old claim, not a deletion.

Commands run from anywhere, against whichever spot is selected —
resolution is `--spot` > `TONK_SPOT` > a directory attached with
`tonk use <name> --here` > `tonk use`. Automation (agents, CI) should
set `TONK_SPOT` or pass `--spot` rather than relying on the global
`tonk use` selection; an agent working out of a fixed directory can
attach it once instead.

## Orientation

```bash
tonk guide            # one-screen index of the agent reference
tonk schema           # every concept + attribute on the branch, as notation
tonk schema <concept> # one concept's subset, same format
tonk concept ls       # name<TAB>description, one row per user concept
tonk view ls          # renderable entities (text/html claim carriers)
tonk status           # synced | ahead | behind | diverged | no-upstream
```

## Data verbs (schema-derived typed flags)

The flags for `assert` are built at runtime from the concept's own schema —
`tonk assert <concept> --help` shows the real fields, types, and which are
required. Errors enumerate the valid options.

```bash
tonk assert <concept> --<field> <value> …            # mint a new instance (all non-optional fields required)
tonk assert <concept> <entity> --<field> <value> …   # supersede fields on an existing instance
tonk query <concept> [--json]                        # every instance, every field bound
tonk query <concept> <entity> [--json]               # one instance
tonk retract <concept> <entity> --field <f>          # retract one field (a many-cardinality field loses every value)
tonk retract <concept> <entity>                      # retract the whole instance
```

Notes:
- `<entity>` is a bookmark name or `did:key:…` URI. The supersede form
  requires the entity to already match the concept; a typo fails with
  "no <concept> instance at …" instead of minting a partial orphan.
- Asserting on a many-cardinality field appends a value.
- Exit codes: 0 success, 1 parse, 2 analyze, 3 commit, 4 I/O.

## Authoring (schema, views, the space home)

```bash
tonk concept add <name> --attr <field>:<type>:<card> [--attr …] [--description <text>]
                                    # types: text, entity, unsigned-integer, …; card: one|many
tonk view add <concept> --template '<html>' | --template-file <path> [--name <anchor>]
tonk home <concept> [<concept> …]   # put concept directories on the space home
```

Notes:
- `concept add` anchors everything, so `tonk assert <name> --help` works
  immediately after.
- `view add` auto-surfaces your build onto the space home when no home is
  set yet; `tonk home` re-points it explicitly (safe to re-run — each run
  replaces the home).
- Writes sync to the upstream automatically (like `tonk eval`); set
  `TONK_NO_SYNC=1` to opt out.

## Escape hatch: eval (asserted-notation documents)

Anything the verbs don't cover — defining concepts/attributes, rules,
views, multi-statement documents — goes through `tonk eval`:

```bash
tonk eval -c '<notation>'     # inline document (-c is required for inline!)
tonk eval ./doc.notation      # from a file
tonk eval - < doc.notation    # from stdin
tonk eval -c '…' --dry-run    # preview without committing
```

`tonk guide notation` documents the grammar; `tonk guide views` covers
`view!:` authoring. A bare positional is a FILE PATH, never inline text.

## Sync and sharing

```bash
tonk push | tonk pull                       # sync main with its upstream
tonk remote add <name> <url>                # register a remote; the first one becomes main's upstream
tonk remote set-upstream <name>             # re-point which remote main tracks
tonk invite                                 # invite URL on the resolved remote's own origin (pushes first)
tonk invite --remote <name>                 # pick the remote when several are registered
tonk invite --no-remote                     # omit remote= (still pushes); the claimer wires one by hand
tonk join '<invite-url>' --name <spot>      # claim an invite into a fresh spot
tonk render <route>                         # headless HTML render (e.g. alice@person!card)
```

## Setup

```bash
tonk spot new <name>              # create a spot (site) and select it
tonk spot new <name> --site <path>  # adopt an existing .tonk directory as a spot
tonk spot list                    # registered spots, with the resolved current
tonk use <name>                   # set the global current spot
tonk identity                     # show the local profile DID
tonk migrate                      # convert a .carry/ site to .tonk/
```
