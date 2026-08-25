# Sync

Each space has a local `main` branch. A configured upstream identifies the
remote branch that `tonk pull` fetches and merges and `tonk push` advances.
`tonk remote` lists remotes; `tonk remote add` registers one and
`tonk remote set-upstream` selects the upstream.

Committing data commands pull before the write and push afterwards when an
upstream is configured. `--no-sync` disables that wrapper for one command.
`--dry-run` never contacts the upstream because it cannot commit.

`tonk status` fetches and reports whether local main is synced, ahead, behind,
diverged, or has no upstream. Its JSON form keeps `sync.fetched`, so callers can
distinguish an unreachable upstream from a current comparison.

`tonk invite` mints an invite URL for the current space, carrying the selected
remote when one resolves. `tonk join <url> --name <name>` creates a local space
from that invitation. Joining and syncing copy facts; removing a local replica
does not erase replicas already held elsewhere.
