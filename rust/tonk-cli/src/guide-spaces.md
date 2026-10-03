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
