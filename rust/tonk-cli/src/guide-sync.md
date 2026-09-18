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

To access a browser space, copy its scoped invitation from Tonk and run
`tonk connect <invite-link>`. This works for people and agents.
Importing and syncing copy facts; removing a local replica does not erase
replicas already held elsewhere.

Scoped invitations use ordinary space grants with explicit expiry
(normally 90 days, bounded by the issuer's authority). Revocation blocks new
remote authorizations as it propagates through the service; already-issued transport
URLs can remain usable for up to 60 seconds. Downloaded data and unsynced edits
stay local. Queries needing data that was never downloaded still require remote
access. New authorization is needed after grant expiry or revocation; the CLI
does not fall back to another account's authority.
