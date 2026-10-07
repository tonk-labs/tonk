# tonk CLI — Agent Reference

tonk is a headless CLI for reading and writing data and views in a space
(a named local dialog repository). Data lives as claims: you **assert**
claims and **retract** them — a retraction is itself an assertion that
invalidates an old claim, not a deletion.

Commands run from anywhere, against whichever space is selected — resolution
is `--space` > `TONK_SPACE` > the nearest directory binding created with
`tonk space use <name>`. Automation (agents, CI) should set `TONK_SPACE` or
pass `--space`; an agent working out of a fixed directory can bind it once
instead.

## Orientation

```bash
tonk guide            # one-screen index of the agent reference
tonk schema           # every concept + attribute on the branch, as notation
tonk schema <concept> # one concept's subset, same format
tonk concept ls       # name<TAB>description, one row per concept this space defines
tonk view ls          # name<TAB>entity<TAB>model<TAB>bytes, one row per model carrying show templates
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

## Authoring (schema, views, the space home, routes)

```bash
tonk concept add <name> --attr <field>:<type>:<card> [--attr …] [--description <text>]
                                    # types: text, entity, unsigned-integer, …; card: one|many
tonk view add <concept> --template '<html>' | --template-file <path> [--kind detail|directory|label|title]
tonk space home <concept> [<concept> …]   # route the space's home (`/`) to concept directories
```

Notes:
- `concept add` anchors everything, so `tonk assert <name> --help` works
  immediately after.
- The space's home page is its own `/` route. `view add` auto-surfaces your
  build there when the space has no home yet; `tonk space home` (or `--home`
  on `view add` / `eval`) re-points it explicitly. The route is pinned to
  `id:space/home-route`, so re-running supersedes it rather than adding a
  second `/`. `--notation` prints what it writes. The old `tonk/space` alias
  no longer exists; don't assert it.
- Use routes for pages. A `route!` maps a `path` to a concept; the concept
  picks `xyz.tonk.site/<param>` (plus `replica`, `repo`, `branch`) off the
  tab's site entity and its `ui` view renders the page, passing
  `with="{branch}@{repo}"` to nested `<tonk-display>`s. `{param}` captures one
  segment, `{*span}` the rest (slashes included). Literal paths beat the
  library's catch-alls `/{*model}`, `/{*entity}@{*model}` and
  `/{*entity}@{*model}!{*view}`. Example, via `tonk eval`:

  ```yaml
  concept!: &todo-page
    this: space:todo-page
    description: One todo's page.
    with:
      entity: { description: The todo., the: xyz.tonk.site/entity, as: entity, cardinality: one }
      repo: { description: The repo., the: xyz.tonk.site/repo, as: text, cardinality: one }
      branch: { description: The branch., the: xyz.tonk.site/branch, as: text, cardinality: one }
  view!:
    this: space:todo-page
    show:
      ui: |
        <tonk-display with="{branch}@{repo}" entity={entity} model=todo />
  route!:
    path: "/todo/{*entity}"
    concept: space:todo-page
  ```

  `tonk help views` ("Routes") has the full walk-through.
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

`tonk help notation` documents the grammar; `tonk help views` covers
`view!:` authoring. A bare positional is a FILE PATH, never inline text.

## Sync and sharing

```bash
tonk push | tonk pull                       # sync main with its upstream
tonk remote add <name> <url>                # register a remote; the first one becomes main's upstream
tonk remote set-upstream <name>             # re-point which remote main tracks
tonk invite                                 # invite URL on the resolved remote's own origin (pushes first)
tonk invite --remote <name>                 # pick the remote when several are registered
tonk invite --no-remote                     # omit remote= (still pushes); the claimer wires one by hand
tonk join '<invite-url>' --name <space>     # claim an invite into a fresh space
tonk render <route>                         # headless HTML render (e.g. alice@person!card)
```

## Setup

```bash
tonk space new <name>               # create a space (site) and bind this directory
tonk space new <name> --site <path> # adopt an existing .tonk directory as a space
tonk space list                     # registered spaces and directory bindings
tonk space use <name>               # bind this directory to a space
tonk identity                     # show the local profile DID
tonk migrate carry                # convert a .carry/ site to .tonk/
```
