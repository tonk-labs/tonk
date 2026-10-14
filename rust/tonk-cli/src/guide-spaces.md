# Spaces

A space is a named, synced store of facts. By default its site data lives in
Tonk's central space store; a directory binding only selects it and stores no
facts. `tonk space new <name> --site <path>` is the explicit exception: it
creates or adopts site data at that path.

Resolution order is `--space <name>`, then `TONK_SPACE`, then the nearest
directory binding created by `tonk space use <name>`. There is no global
fallback. In automation, pin the space per process or bind a dedicated working
directory once.

`tonk space` lists registered spaces and marks the active one. `tonk space
use <name>` binds the current directory and its descendants. `tonk space
unbind` removes an exact binding; run it from the bound directory or pass its
absolute path.

To access an existing browser space, run `tonk join <invite-link>`. The
link carries a reusable invitation key and scoped grants, so keep it private.
Import works after the browser closes, without CLI account login. Resume an
interrupted import with `tonk --space <name> join`.

The space listing reports local roster facts only. Remote authority is checked
when a sync operation uses it; removing or losing access keeps aliases,
downloaded data and unsynced edits.

Account management belongs in the Tonk UI. Existing replicas and credentials
remain on disk; importing an invitation does not sign into an account, move
ownership or transfer local edits into the newly connected replica. CLI-created
spaces remain local until an authorized remote is configured.

## Renaming through the shared worker tools

Read the current name with `tonk/repository:` and the authorized space DID as
`this:`. For a rename, use the worker command below through `tonk_evaluate` (or
`tonk eval`). Replace `urn:example:authorized-space` with that exact DID and
choose the quoted name. This command updates both the space's own name and its
account-directory label. The older space-library `tonk/rename-repository` rule
only changes the space fact and can leave the picker label stale.

<!-- tested-example: rename-space -->
```yaml tonk=parse
command!: &agent/rename-space
  description: Rename the authorized space and its account-directory label.
  with:
    space:
      description: The authorized space DID.
      the: xyz.tonk.rename-repository/space
      as: entity
    name:
      description: The new space name.
      the: xyz.tonk.command.rename-repository/name
      as: text
agent/rename-space!:
  space: urn:example:authorized-space
  name: "New space name"
```

The worker rejects commands targeting a different space from a content branch.
Query `tonk/repository:` again and check the host's picker after its catalog
refreshes. Command effects can settle after the evaluation response. Never
repeat a rename merely because the response was lost; query first.
