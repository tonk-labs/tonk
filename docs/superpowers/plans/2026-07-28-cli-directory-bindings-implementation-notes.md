# Implementation notes — CLI directory spot bindings

Running log for the revision of
`2026-07-27-cli-spot-attachments-design.md`.

## `use` stays as the intent-level verb

The registry operation is a directory binding: a canonical directory
path points at a named spot whose data remains in the central spot
store. The user-facing command remains `tonk use <name>` because it
states the desired outcome without exposing registry mechanics.

Alternatives rejected:

- `worktree add` implies that Tonk creates a working copy in the
  directory. It does not.
- `mount` implies that spot data becomes visible through the
  filesystem path. It does not.
- `attach` and `bind` accurately describe the registry write, but are
  weaker everyday language for the common operation.

The inverse administrative operation is `tonk spot unbind [PATH]`.

## Removing `current` changes create and join

`spot new` and `join` used to write the global `current` field. Simply
deleting that write would make the next command fail unless the user
remembered a separate `tonk use`.

Both commands will instead bind the directory in which they run. This
preserves their create-and-activate behavior while keeping activation
local to a directory. A process-level `--spot` or `TONK_SPOT` still
outranks the binding for the current invocation.

## The registry migration must consume retired fields

The registry preserves unknown fields across writes. Removing the
typed `current` field naively would therefore move it into the
flattened unknown-field map and write it back forever.

`current` remains a deserialize-only compatibility sink: resolution
never reads it, and the next registry write drops it. The old
`attachments` key is accepted as an alias and serializes back as
`bindings`.

## Error context is unconditional

There is no reliable way for the CLI to distinguish an agent from a
human or a script. Spot-scoped failures therefore print the same stable
active-spot footer for everyone. The footer is local registry context;
it does not fetch sync state while handling an unrelated error.
