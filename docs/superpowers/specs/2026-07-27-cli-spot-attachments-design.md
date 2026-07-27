# CLI directory attachments for spot selection

**Date:** 2026-07-27
**Status:** approved design, pending implementation
**Branch:** `feat/cli-sessions`

## Problem

Spot selection resolves `--spot` > `TONK_SPOT` > the registry's
`current` (`rust/tonk-cli/src/spot.rs`). Anyone driving more than one
spot at a time has no comfortable option:

- **Terminal tabs.** `export TONK_SPOT=x` per tab works, but has to be
  remembered on every new tab, and nothing on screen says which spot
  the tab is pinned to.
- **Agents with a fresh shell per command.** `export` cannot persist —
  every single invocation has to carry `--spot x` or an inline env
  prefix.
- **Parallel agents in separate directories.** `tonk use` is the only
  persistent selection and it is machine-global, so concurrent
  sessions clobber each other.

The gap is a *persistent* selection, narrower than machine-global,
keyed on something a freshly spawned process can re-derive without
being told. The working directory is the one key all three shapes
already have.

## Design

A central map from directory to spot name, consulted as a new
resolution tier. The directory is a lookup key into the registry and
nothing more — it never locates data and never creates anything.

This is not the `git worktree` model, where the directory *is* the
working copy. It is the relationship pipenv and poetry have with
virtualenvs: the resource lives centrally and stays managed, and the
project directory is a pointer at it that costs nothing to lose.

### Storage

`spots.json` gains one field beside `current`:

```json
{
  "current": "garden",
  "attachments": {
    "/Users/jack/tonk/tonk": "dev",
    "/Users/jack/notes": "garden"
  },
  "spots": {
    "dev": { "site": "/Users/jack/Library/Application Support/tonk/spots/dev" }
  }
}
```

`#[serde(default)]` so existing registries load unchanged, written
through the existing temp-file-plus-rename path. Keys are
canonicalized absolute paths, so `/tmp` and `/private/tmp` are the
same entry.

It lives in `spots.json` rather than a file of its own because it is
selection state over the registry, exactly like `current`: one load,
and no way for the two to drift.

It is a top-level map rather than a list inside each `SpotEntry`
because that layout makes "one directory, one spot" structural — a
key cannot repeat. Nested under spots, two entries could both claim a
directory and resolution would tie-break on `BTreeMap` iteration
order, succeeding at the wrong answer silently. The invariant the
nested layout would have made free — no attachment without a spot —
costs one `retain` in `remove()`, the same cascade already written
there for `current`.

`SpotEntry` therefore stays `{ site }`, which is what applications
reading `spots.json` resolve paths from.

### Resolution

```
--spot  >  TONK_SPOT  >  attachment (nearest ancestor of cwd)  >  current
```

The walk starts at the cwd itself and climbs to the root; the first
hit wins, so a nested directory overrides its parent (`.gitignore`
semantics).

`TONK_SPOT` stays above attachments deliberately: a harness that
pinned a process must not be overridden by whatever directory it
happened to launch in, and bench/CI already depend on that.

`SpotStore::resolve` takes the cwd as a parameter
(`resolve(flag, env, cwd)`) rather than calling `current_dir()`
itself, matching how `flag` and `env` are already passed in — tests
stay pure and never touch process-global state.

`Source` gains `Attached(PathBuf)`, so every existing readout stays
honest about which tier answered: `tonk status` prints
`spot: dev (attached /Users/jack/tonk/tonk)`, and `tonk spot list`
marks the same. `Source` drops `Copy` for `Clone`.

### Commands

- **`tonk use <name> --here`** — bind `$PWD` (canonicalized) to
  `<name>`; the global `current` is untouched. An unknown name reuses
  `SpotError::Unknown`. Re-attaching an already-bound directory
  overwrites and reports it (`attached /Users/jack/notes to garden
  (was dev)`) — unlike `spot new`, nothing is being destroyed, so no
  `rm`-first dance.
- **`tonk spot detach [PATH]`** — remove the attachment for `PATH`,
  defaulting to `$PWD`, matching **exactly** rather than by ancestor.
  Typing `detach` three levels inside an attached project must not
  silently unbind the project; it reports instead: `no attachment at
  <cwd>; /Users/jack/notes is attached to garden`. The optional
  `PATH` is also how an entry whose directory no longer exists gets
  cleared, since there is no way to `cd` there.
- **`tonk spot list`** — appends a tab-separated `attached:` block
  when the map is non-empty.
- **`tonk status`** — unchanged; the new `Source` variant carries it.

`SpotError::NoSelection` becomes: `no spot selected; run 'tonk use
<name>', add --here to bind this directory, pass --spot, or set
TONK_SPOT`.

### Edge cases

- Canonicalize on write and on read; when canonicalization fails
  (vanished directory) fall back to the raw path, which simply never
  matches and drops resolution to the next tier.
- `spot rm` prunes the removed spot's attachments alongside clearing
  `current`.
- An attachment at `/` is legal and matches everywhere. Not
  special-cased.
- `spot new` and `join` do not auto-attach. Binding a directory stays
  an explicit act.
- An orphaned attachment (hand-edited file naming an unregistered
  spot) hits the existing `SpotError::Unknown`, which lists what is
  registered and, when the name came from an attachment, names the
  directory and points at `tonk spot detach` — otherwise the error
  reads as coming from nowhere.
- `tonk use <name>` (no `--here`), `tonk spot new`, and `tonk join`
  all set the global `current`, but an attachment ranks above it — so
  running any of them from a directory attached to a *different*
  spot confirms a selection the very next command will not honour.
  This is the scenario the tier was built for (an agent binds a
  worktree with `--here`, a human `cd`s in and runs `tonk use
  garden`), so it is not treated as a footgun to leave alone: after a
  successful `select` / `create` / `join`, each command resolves once
  more against the real cwd, and if that still lands on
  `Source::Attached` naming a spot other than the one just picked, it
  warns on stderr (`warning: commands here still resolve to '<name>'
  (attached <dir>); run \`tonk spot detach\` to drop it`), leaving the
  stdout confirmation alone. Silent whenever there is no attachment or
  the attachment already agrees.
- The registry preserves fields it does not recognise, so a writer
  that predates a field cannot silently drop it.

### Testing

Unit tests in `spot.rs`, cwd passed in so nothing is process-global:
deepest ancestor wins, an unattached cwd falls through to `current`,
`TONK_SPOT` beats an attachment, `--spot` beats both, `rm` prunes,
re-attach reports the previous binding, detach matches exactly.

Integration in `tests/cli_spot.rs`: the isolated-binary harness gains
a `current_dir`; two spots and two directories, `tonk status` from a
subdirectory of each resolves differently, and `TONK_SPOT` overrides
both.

Each file keeps its existing test style.

### Documentation

The resolution order is stated in four places, all needing the new
tier:

- `rust/tonk-cli/src/spot.rs` module header
- `rust/tonk-cli/README.md`
- `rust/tonk-cli/src/guide-index.md` — the agent-facing one; it should
  recommend `--here` for a directory-scoped agent while keeping
  "automation pins `TONK_SPOT`"
- root `README.md`

Plus the `--spot` and `use` help strings in `bin/tonk.rs`.

## Out of scope / follow-ups

- `tonk env <spot>` / `tonk shell <spot>` for pinning a tab by
  environment instead of by directory.
- Terminal- or process-keyed sessions (tty, ancestor pid). Considered
  and set aside: it survives `cd`, but needs process introspection,
  pid-reuse handling, liveness pruning, and an anchor rule for
  ttyless agents, and leaves state that cannot be explained without
  naming a pid.
- Auto-attach on `spot new` / `join`.
