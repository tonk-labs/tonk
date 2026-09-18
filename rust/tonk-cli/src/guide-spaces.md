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

To access existing spaces, run `tonk link` and select them in your browser.
The terminal keeps its private key and receives only the selected space grants.
`--no-open` prints the approval URL for a browser on another machine. Selecting
all spaces means those available now; later additions need another grant.

A browser-generated agent invitation uses `tonk connect <agent-link>`. The
link carries a reusable invitation key and scoped grants, so keep it private.
Import works after the browser closes, without CLI account login. Resume an
interrupted import with `tonk --space <name> connect`.

The ACCESS column distinguishes local-only, invite-backed, terminal-linked and
legacy bindings. Cached access details do not prove that remote access is still
valid. Removing access keeps aliases, downloaded data and unsynced edits.

Existing account-linked installations can explicitly run
`tonk link --convert-account`. Verified selections get separate replicas before
the old account attachment is deactivated. Old data and credentials remain for
recovery; local edits are not automatically transferred to the new replicas.
`tonk space link <name>` still means local-space ownership adoption.
