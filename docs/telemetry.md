# Telemetry

Tonk collects anonymous usage analytics via PostHog to understand
which features get used and where errors occur. This page is the
complete inventory of what is sent and how to turn it off.

## What is never sent

Notation documents, entity names or values, attribute values, file
paths, remote URLs, and raw DIDs. Identifiers that appear in event
properties are SHA-256 hashes.

## Identity

The distinct id is `tonk:<sha256(profile DID)>` — stable per profile,
not reversible. `tonk identity --reset` rotates it. Commands run
before a profile exists report as `tonk:anonymous`.

## Events

### CLI

| Event | Properties |
|---|---|
| `cli_command_run` | `command`, `subcommand`, `success`, `exit` (success / parse-error / analyze-error / commit-error / io-error), `duration_ms`, `version`, `os`, `arch`; eval adds `source` (inline / file / stdin), `format`, `dry_run`, `quiet` |

### Web app

| Event | Properties |
|---|---|
| `app_loaded` | `version` |
| `$pageview` | `route` — the path with every non-literal segment replaced by a hash (e.g. `/space/1a2b3c4d5e6f7a8b/view/9c8d7e6f5a4b3c2d`) |
| `commit` | none (fired on the existing `tonk:committed` event) |
| `sheet_activated` | none |
| `panic` | `message` (first line of the panic message) |

Autocapture and session recording are disabled. PostHog runs
cookieless (`persistence: "memory"`); no third-party script is loaded
(the bundle is self-hosted).

## Turning it off

- CLI: `tonk telemetry off` (persisted), or `DO_NOT_TRACK=1`, or
  `TONK_TELEMETRY=0` per invocation. `tonk telemetry` shows the
  effective state.
- Web: run `localStorage.setItem("tonk:telemetry", "off")` in the
  console and reload. (A settings toggle is a planned follow-up.)
- Builds without a `TONK_POSTHOG_KEY` baked in send nothing at all —
  this is the default for local builds, CI, and forks.

## For release builds

Set `TONK_POSTHOG_KEY` (a PostHog *project API key* — public by
design) and optionally `TONK_POSTHOG_HOST` (default
`https://us.i.posthog.com`) in the environment of the release build.
`TONK_POSTHOG_KEY`/`TONK_POSTHOG_HOST` are also read at runtime, and
`TONK_POSTHOG_ENDPOINT` overrides the host for integration tests.
