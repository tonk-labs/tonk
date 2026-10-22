# Interface dashboards

Tonk is used through three interfaces: the web app, the `tonk` CLI, and the macOS
desktop app. Each has its own PostHog dashboard, plus one overview that compares
them. The queries are versioned in
[`interface-dashboards.json`](interface-dashboards.json).

| Dashboard | Scope | Answers |
|---|---|---|
| Interfaces overview | CLI, desktop, and production web | How many profiles use each interface and which combinations; product outcomes, top actions, failures, and release adoption side by side |
| Web app | `environment` = `production`, `staging`, or `dev`, and `shell` ≠ `desktop` | Daily profiles, product outcomes, failures, latency, startup readiness, and account journeys, split by environment |
| CLI | `environment` = `cli` | Daily profiles and runs, command health, product outcomes, failures, platforms, and account commands |
| Desktop app | `environment` = `desktop` (native events only) | Launches and profiles, release adoption, outcomes by surface, sharing, chat and views, and failures |

## Definitions

- **Interface** comes from each event's `environment` and `shell`: `cli` is the
  CLI, `desktop` is the desktop app, and `production`/`staging`/`dev` is the web
  app unless `shell` is `desktop`. The overview counts production web only; the
  Web app dashboard shows every web environment.
- **Profiles** are distinct complete profile hashes (`tonk:` followed by 64 hex
  characters). Anonymous and unresolved events are excluded. As in
  [the metric definitions](metrics.md), a profile is not a person or an account.
- **Outcome rates** use the attempt cohort of
  [`analytics-contract.md`](../analytics-contract.md): attempts older than ten
  minutes, matched by `attempt_id`; a missing terminal event counts as
  abandoned, not failed.

## Desktop space views

Space views inside the desktop app run the web app, so their events keep the web
app's `environment` (`production` on tonk.network) and add `shell` = `desktop`;
in a browser `shell` is `browser`. See [telemetry](../telemetry.md).

- The Web app dashboard excludes `shell = 'desktop'`.
- The overview counts production events with `shell = 'desktop'` as desktop.
- The Desktop app dashboard stays on native `environment = 'desktop'` events.
  Its outcomes, sharing, chat and views come from the app itself, not from the
  embedded web app.

Events from before `shell` existed have no value and count as browser, so
historical desktop space views still appear as web. Queries compare with
`coalesce(properties.shell, '')` so those events are not dropped.

## Publishing

Install and sign in to `posthog-cli` for project 70116 (or set `POSTHOG_CLI`).
Then:

```console
python3 -m unittest scripts/tests/test_interface_dashboards.py
python3 scripts/posthog-interface-dashboards.py validate
python3 scripts/posthog-interface-dashboards.py publish
```

`validate` runs every query and writes nothing. `publish` creates or updates the
four dashboards, keeps any other dashboard memberships of the insights, and
records the published IDs back into the spec. `--dashboard NAME` limits either
mode to one dashboard. The existing Product decisions dashboard is unchanged.
