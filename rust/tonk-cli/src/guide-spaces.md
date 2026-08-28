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
