# CLI canonical spot storage + registry

**Date:** 2026-07-20
**Status:** approved design, pending implementation
**Branch:** `feat/cli-canonical-dir`

## Problem

`tonk` today roots everything at a `.tonk/` directory discovered by
walking up from the cwd (`rust/tonk-cli/src/site.rs`). That forces
users to `cd` into the right directory before every command, forces
applications built on tonk to track where sites live themselves, and
lets the same spot end up replicated across scattered filesystem
locations.

## Design

Tonk becomes opinionated about where spot data lives, the way docker,
ollama, and postgres manage their resources: a platform-canonical
storage directory, a name registry, and a selected "current" spot so
the CLI works from anywhere.

### Vocabulary

- **spot** — the named logical unit in the registry. What users
  select and what applications resolve by name.
- **site** — the physical directory backing a spot (what `.tonk/`
  holds today: the dialog repo blocks). `TonkSite` and the whole site
  layer are unchanged; everything new is a resolution layer above it.

### Layout

macOS shown; `dirs::data_dir()` supplies the platform equivalent on
Linux/Windows, matching the existing `telemetry.json` / `update.json`
placement:

```
~/Library/Application Support/tonk/
  spots.json          # registry: name → site path, plus current selection
  spots/<name>/       # canonical site dirs (repo blocks directly inside, no .tonk nesting)
  telemetry.json      # existing
  update.json         # existing
```

### Registry

`spots.json` is the single source of truth mapping spot names to site
paths. Paths are stored absolute and expanded so any application can
read the file and resolve a name with zero path logic:

```json
{
  "current": "garden",
  "spots": {
    "garden": { "site": "/Users/jack/Library/Application Support/tonk/spots/garden" },
    "work":   { "site": "/Users/jack/work/site" }
  }
}
```

- Writes go through temp-file + atomic rename in the same directory.
- A corrupt or unparseable registry is a hard error naming the file.
  It is never silently recreated or repaired.
- Spot names are validated to a conservative slug
  (`[a-z0-9][a-z0-9-_]*`) because canonical names become directory
  names.

### Resolution

Every data command resolves its spot in strict precedence order:

1. `--spot <name>` — new global CLI flag
2. `TONK_SPOT` — environment variable
3. `current` — the registry's global selection

The cwd is never consulted. `discover_and_open` and `find_site_root`
are deleted. Failure modes:

- No selection anywhere → error with a hint: `tonk use <name>`, or
  `tonk spot new <name>` when the registry is empty.
- Unknown name → error listing the registered spots.

Resolution reports its **source** (`flag`, `env`, or `global`)
alongside the name, and command surfaces expose it (see Concurrency).

### Commands

- `tonk use <name>` — set the registry's `current`. Errors on unknown
  name.
- `tonk spot new <name> [--site <path>]` — create the site (canonical
  `spots/<name>/` by default, `--site` overrides the storage
  location), register it, and select it. `TonkSite::init` is
  idempotent, so `--site` pointing at an existing site directory
  adopts it — this is also the migration path for pre-existing
  `.tonk/` dirs: `tonk spot new myproj --site ~/proj/.tonk`.
- `tonk spot list` — one row per spot: name, site path, a marker on
  the current spot, and the source the current selection resolved
  from.
- `tonk spot rm <name>` — remove the registry entry (clearing
  `current` if it pointed there). Data stays on disk and the site
  path is printed. `--delete` also removes the site directory.
- `tonk join <url> --name <name>` — join into a canonical site under
  `<name>`, register, select. The name is required; nothing is
  derived from the invite.
- `tonk init` — removed. `tonk spot new` replaces it.

### Concurrency

The global `current` is a convenience for the human at the keyboard.
Concurrent sessions (parallel agents, bench runs, CI) must pin their
spot explicitly; the precedence order makes that safe because `--spot`
and `TONK_SPOT` are per-invocation / per-process and always beat the
shared `current`. Two sessions each launched with their own
`TONK_SPOT` cannot mix, regardless of who runs `tonk use` meanwhile.

Two guardrails make the contract visible:

1. **Visibility** — resolution carries its source everywhere. `tonk
   spot list` shows which spot is current and whether that came from
   `flag`, `env`, or `global`; errors and status output name the
   resolved spot. A session can always verify what it is about to
   touch.
2. **Agent guidance** — the `tonk` skill and the agent-facing guide
   state the rule: automation always sets `TONK_SPOT` (or passes
   `--spot`) and never relies on `tonk use`. Same contract CI has
   with `DOCKER_CONTEXT`.

A `TONK_REQUIRE_SPOT` hard-mode guard (bare fallthrough to `global`
becomes an error) was considered and deferred — no immediate use.

### Code shape

- New module `rust/tonk-cli/src/spot.rs`: registry types
  (`Registry`, entry struct), load/save with atomic rename,
  `resolve(override) -> resolved {name, site path, source}`,
  canonical-root helper, error types.
- `bin/tonk.rs`: add the global `--spot` arg; replace every
  `current_dir()` + `discover_and_open` pair with resolve-then-
  `TonkSite::open`.
- `invite.rs`: the join path roots at the canonical site (or is
  handed a site path) instead of `cwd/.tonk`; the
  "site already exists" guard keys off the registry name and target
  directory instead of `parent/.tonk`.
- `TONK_SPOTS_STATE` environment variable overrides the directory
  holding `spots.json` and the canonical `spots/` root (same pattern
  as `TONK_TELEMETRY_STATE` / the update state override) so tests run
  against temp dirs.

### Error handling

- Missing/unknown spot: actionable errors as described under
  Resolution.
- Corrupt registry: hard error with the file path.
- `spot new` with a name that already exists: error; re-pointing a
  name is an explicit `rm` + `new`.
- `--site` paths are canonicalized at registration; a non-UTF-8 or
  uncreatable path errors up front.

### Testing

- Registry unit tests: precedence order (flag > env > global), atomic
  write, slug validation, adopt-existing via `--site`, `rm` with and
  without `--delete`, corrupt-file error.
- Site-layer tests are untouched (`init_with`/`open_with` on temp
  dirs still work).
- CLI-level tests pin `TONK_SPOT` plus the registry-path env
  override.

## Out of scope / follow-ups

- Updating the `tonk` skill, bench scenarios, and docs that assume
  cwd discovery ("cd into the project"). They break until they pass
  `--spot` or set `TONK_SPOT`; tracked as follow-up work.
- `TONK_REQUIRE_SPOT` hard mode.
- Any change to profile/identity storage (already canonical under
  `~/Library/Application Support/dialog/`).
