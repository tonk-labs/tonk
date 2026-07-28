# CLI directory bindings for spot selection

**Date:** 2026-07-27
**Revised:** 2026-07-28
**Status:** implemented on `feat/cli-sessions`
**Branch:** `feat/cli-sessions`

## Problem

A machine-global active spot cannot represent two terminal tabs or
agents working concurrently. Requiring every invocation to repeat
`--spot` or `TONK_SPOT` is safe but awkward. A fresh process already
has one stable piece of context: its working directory.

Tonk needs a persistent directory-scoped choice without moving or
copying spot data into that directory.

## Model

A spot remains a named entry whose site data lives in the central Tonk
store. The registry also holds directory bindings:

```json
{
  "bindings": {
    "/Users/jack/tonk/tonk": "dev",
    "/Users/jack/notes": "garden"
  },
  "spots": {
    "dev": {
      "site": "/Users/jack/Library/Application Support/tonk/spots/dev"
    },
    "garden": {
      "site": "/Users/jack/Library/Application Support/tonk/spots/garden"
    }
  }
}
```

The directory is only a canonical path key into `spots.json`. No Tonk
data or pointer file is written into the working directory.

This borrows the useful part of Git worktrees: a durable association
between a central repository and a directory, explicit listing, and
path-based context. It does not call the directory a worktree because
Tonk does not create a working copy there.

The top-level `path → spot` map makes “one directory, one spot”
structural. A nested map under each spot could let two spots claim the
same path and require a hidden tie-break.

Old `attachments` maps deserialize as `bindings`. The old `current`
field is accepted only as migration input, is never consulted, and is
dropped on the next registry write.

## Resolution

```text
--spot > TONK_SPOT > nearest bound ancestor of cwd
```

There is no global fallback.

The walk starts at the cwd and climbs to the root. The first binding
wins, so a nested project can override its parent. `TONK_SPOT` stays
above directory bindings because an explicitly pinned process must not
depend on where its harness launched it.

`SpotStore::resolve(flag, env, cwd)` receives the cwd rather than
reading process state. Tests can therefore exercise resolution without
mutating the process environment or working directory.

`Source::Directory(PathBuf)` carries the exact binding that won.
Human and agent-facing output can report `directory /path` rather than
an unexplained active name.

## Commands

- **`tonk use <name>`** binds `$PWD` to an existing spot. Rebinding
  replaces the old spot and reports it. There is no `--here`; local is
  the only persistent meaning.
- **`tonk spot new <name>`** creates or adopts the central site and
  binds the invocation directory on success.
- **`tonk join ... --name <name>`** registers the claimed central site
  and binds the invocation directory on success.
- **`tonk spot unbind [PATH]`** removes the exact binding for `PATH`,
  defaulting to `$PWD`. It refuses to unbind an ancestor implicitly and
  names the bound ancestor in the error.
- **`tonk spot list`** marks the spot active for this invocation,
  reports its source, and lists every directory binding.
- **`tonk status`** begins with the active spot and source.

`--spot` and `TONK_SPOT` remain ephemeral overrides. They do not rewrite
directory bindings.

## Naming

`use` is the intent-level verb: “use garden here.” The implementation
and documentation call the relationship a directory binding.

Rejected primary verbs:

- `worktree add`: implies Tonk copies data into the directory.
- `mount`: implies the path exposes a filesystem.
- `attach` or `bind`: mechanically precise, but weaker everyday
  language for the common action.

`tonk spot unbind` is an administrative command, so the precise
mechanical term fits there.

## Visibility and errors

Successful `use` output names the active spot, canonical directory,
and central site. `spot list` says `active here`, not `current`.

After any spot-scoped command fails, the CLI prints a stable stderr
footer when resolution succeeded:

```text
active spot: garden (directory /Users/jack/notes)
site: /Users/jack/Library/Application Support/tonk/spots/garden
```

The footer is unconditional. Agent detection is unreliable, and the
same context helps humans and scripts. It does not fetch remote sync
state while handling an unrelated error; `tonk status` remains the
explicit sync-state command.

When no override or binding resolves, the error names the local fix:

```text
no spot active for this directory; run `tonk use <name>`, pass --spot, or set TONK_SPOT
```

## Edge cases

- Paths canonicalize on write and lookup. A vanished path falls back
  to its raw absolute spelling so `spot unbind /old/path` can remove
  it.
- A binding at `/` is legal and applies everywhere unless a deeper
  binding wins.
- `spot rm` removes every binding naming the removed spot.
- An orphaned hand-written binding reports its path and points to
  `tonk spot unbind`.
- Unknown registry fields still survive writes. Only the deliberately
  retired `current` field is consumed and dropped.

## Tests

Unit coverage includes precedence, nearest-ancestor resolution,
no-global-fallback migration, rebind reporting, exact unbind, orphan
errors, and binding pruning.

CLI coverage uses isolated registries and real subprocess cwd values.
It verifies separate directory contexts, flag and environment
overrides, active-spot error context, `spot new` rebinding, list output,
and exact unbind behavior.

## Follow-ups

- A local-only `tonk context --json` can expose the same resolution in
  a versioned agent contract without making error strings an API.
- `tonk spot prune` could remove bindings whose directories no longer
  exist, matching Git worktree maintenance without adopting its data
  model.
