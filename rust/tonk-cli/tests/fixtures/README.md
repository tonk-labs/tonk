# Migration fixtures

## `legacy-spot-v0.6.7.tar.gz`

A complete spot **directory** — the on-disk database, not an export — created
and populated by the pre-dialog-upgrade build: tag `v0.6.7` (commit
`d7074057`), pinning dialog at `rev = e8bbe462`.

The database rather than a CSV, because the export step is half the migration
and the half that needs the old binary. A committed CSV would let a test skip
straight to the import and prove less than it appears to.

**The current build cannot read it.** Opening the branch fails with:

```
Failed to decode a block: Msg("missing field `branch`")
```

The revision block is still CBOR, so it decodes — its *shape* changed. That is
the whole reason the migration exists, and it is why `tonk export` has to run
under the old binary. There is no path where the new build reads this
directly.

## What it contains

1197 exported rows, of which 533 carry the old `dialog.*` schema namespace
that became `db.*`. The application data includes one `note` concept with

    title: "written by the old build"

on entity `did:key:z6Mk3VY17HUDh9rW6UpiDdtF9BGmdqfsYC2ZGzk4rAJadk2H`. A
migration test asserts that exact query still resolves afterwards, to the same
entity and the same value — data and identity both, which is what makes a
migrated spot the *same* spot to its peers rather than a copy.

## Regenerating

```
git worktree add /tmp/old-tonk v0.6.7
cd /tmp/old-tonk && nix build --accept-flake-config .#tonk-cli
env HOME=/tmp/oldhome TONK_UNSAFE_ALLOW_DEVICE_ROOT=1 ./result/bin/tonk spot new legacy
# ...seed data, then archive the spot directory:
tar -czf legacy-spot-v0.6.7.tar.gz -C "/tmp/oldhome/Library/Application Support/tonk/spots" legacy
```

`HOME` is overridden because the spot registry resolves through
`dirs::data_dir()`, which on macOS ignores `TONK_HOME` and would otherwise
write into the real one.
