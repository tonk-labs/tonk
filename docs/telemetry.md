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
| `cli_command_run` | `command`, `subcommand`, `success`, `exit` (success / parse-error / analyze-error / commit-error / io-error), `duration_ms`, `version`, `os`, `arch`, `environment` (constant `"cli"`), ``$lib`` (constant `"tonk-analytics"`); eval adds `source` (inline / file / stdin), `format`, `dry_run`, `quiet` |

### Web app

| Event | Properties |
|---|---|
| `app_loaded` | `version` |
| `$pageview` | `route` — the path with every non-literal segment replaced by a hash (e.g. `/space/1a2b3c4d5e6f7a8b/view/9c8d7e6f5a4b3c2d`); ``$current_url`` carries the same normalized value |
| `commit` | `branch` (`main` / `meta`; anything else reports as `other`) — fired on the worker's `tonk:local-commit` broadcast for every durable transact/evaluate commit; only the visible tab records it |
| `sheet_activated` | none |
| `panic` | `message` (first line of the panic message) |

Every web event additionally carries an `environment` super property
derived from the hostname: `production` (hub.tonk.xyz), `staging`
(staging.tonk.xyz), or `dev` (anything else). The marketing site's
own snippet registers `environment = "website"` in its repo, so all
surfaces share one filterable dimension within the PostHog project.

Autocapture and session recording are disabled. PostHog runs
cookieless (`persistence: "memory"`); no third-party script is loaded
(the bundle is self-hosted).

The web shell also listens for a generic `tonk:analytics` DOM event
(`detail: { name, props }`) so future components can emit events
without new dependencies. Nothing dispatches it today; any future
dispatcher is responsible for keeping its payload content-free
(hashes and counts only), like every event above.

## Turning it off

- CLI: `tonk telemetry off` (persisted), or `DO_NOT_TRACK=1`, or
  `TONK_TELEMETRY=0` per invocation. `tonk telemetry` shows the
  effective state. The persisted choice lives at ``<platform data dir>/tonk/telemetry.json`` (``TONK_TELEMETRY_STATE`` overrides the directory).
- Web: run `localStorage.setItem("tonk:telemetry", "off")` in the
  console and reload. (A settings toggle is a planned follow-up.)
- Builds without a `TONK_POSTHOG_KEY` baked in send nothing at all —
  this is the default for local builds, CI, and forks.

## For release builds

Set `TONK_POSTHOG_KEY` (a PostHog *project API key* — public by
design) and optionally `TONK_POSTHOG_HOST` (default
`https://eu.i.posthog.com`) in the environment of the release build.
`TONK_POSTHOG_KEY`/`TONK_POSTHOG_HOST` are also read at runtime, and
`TONK_POSTHOG_ENDPOINT` overrides the host for integration tests.

The nix packages (`tonk-cli`, `tonk-ui`, and therefore
`tonk-cloudflare-artifacts`) bake the key in from the `posthogKey`
binding in `flake.nix`, so `wrangler deploy` release builds carry it
automatically; dev-shell `cargo`/`trunk` builds stay key-less unless
the env var is exported.
