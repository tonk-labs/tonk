# Tutorial: the loop

## Work in an existing space

Start from the user's task and read `tonk space agents get` for any shared
space instructions. Run `tonk show` once for the schema if you do not
already know the relevant concept. Then inspect only what that task needs:
`tonk show <concept> <entity>` for a known instance, or `tonk query <concept>`
to find it. Narrow reads with `--where 'title=Draft launch email'` or
`--where done=false`; repeated filters are ANDed. These filters currently
support text and boolean fields, and many-valued fields match a particular
value. All matching rows are returned, so resolve ambiguous matches before
choosing an entity to update.

Once you know the target and fields, make the requested change. A write receipt
with `verification: verified` confirms the requested fields through a fresh
local read; no additional `show` is needed for that check. If verification fails,
inspect the entity with `tonk show <concept> <entity>` rather than repeating the
write. UI changes still need rendering. A space with no shared instructions is
normal; continue with the task without creating instructions.

Do not query every concept, enumerate all views, or read every guide before
starting. Expand discovery only to answer a specific question blocking the
task. Use `tonk show <name>` for a particular concept or view, and
`tonk help <command>` or `tonk help <guide>` when you need that reference.
Use `tonk status` if the selected space or connection state is unclear.

The `tonk show` overview lists application concepts with short typed fields;
`?` means optional and `[]` means many. Use `tonk show --all` to include runtime
concepts and expand the overview, or `tonk show --notation` for the full schema
export. `tonk show --json` returns the complete structured concept list.

`tonk assert ... --json` returns a `tonk.assert.v1` receipt with `committed`,
`dryRun`, revisions, claim count, entity, `verification` and `sync`. Verification
is local: `verified`, `not-matched`, `failed`, or `not-run` for a dry run. Remote
push is reported separately as `pushed`, `failed`, `no-upstream`, `disabled`, or
`not-committed`. If push failed, the local write is still saved; retry `tonk push`,
not the write. A schema field named `json` keeps its field meaning, as with the
other dynamic assert flags.

## Build a new model when the task needs one

1. Use the schema from `tonk show` to reuse existing concepts where possible.
2. Define a concept with `tonk concept add <name> --field
   <field>:<type>:<cardinality>`. The concept is immediately usable.
3. Create or update facts with `tonk assert`; read them with `tonk query`; use
   `tonk retract` to invalidate a field or instance.
4. Add a view with `tonk view add`. The home page is the space's `/` route;
   a first detail or directory view automatically routes it there when the
   space has no home yet. Use `--home` to install a view and route the home
   atomically, or `tonk space home` to repoint it later. Check the result
   headlessly with `tonk render`.
5. Give every other page its own `route!` (`/todo`, `/todo/{*entity}`,
   written with `tonk eval`) rather than sending people to `@`-shorthand
   URLs: a route maps a path to a concept that picks the page's inputs off
   the tab, and that concept's view renders it. See "Routes" in
   `tonk help views`.
6. Use `tonk eval` for rules, effects, joins, or multi-statement documents the
   convenient verbs cannot express. On a raw first build,
   `tonk eval interactive.notation --home todo` installs the document and
   replaces the home in one transaction.
7. Copy a space invitation from Tonk and run `tonk join <invite-link>`.
   The same command works for people and agents, without browser approval.

Every notation-building write accepts `--notation` to print the document it
would evaluate. A committing write syncs automatically unless `--no-sync` is
set; `--dry-run` evaluates and plans but drops the transaction.

The shortest schema-to-visible-directory workflow is:

```text
tonk concept add todo --field title:text:one
tonk assert todo --title "Write the guide"
tonk view add todo --kind directory --template-file todo.html --home
tonk render todo
```

`--home` routes `/` to `todo` in place of the prior home; it does not append
to it.
