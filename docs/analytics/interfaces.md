# Interface dashboards

Tonk is used through three interfaces: the web app, the `tonk` CLI, and the macOS
desktop app. Each has its own PostHog dashboard, plus one overview that compares
them. The queries are versioned in
[`interface-dashboards.json`](interface-dashboards.json).

| Dashboard | Scope | Answers |
|---|---|---|
| Interfaces overview | CLI, desktop, and production web | How many profiles use each interface and which combinations; product outcomes, top actions, failures, and release adoption side by side |
| Web app | `environment` = `production`, `staging`, or `dev` | Daily profiles, product outcomes, failures, latency, startup readiness, and account journeys, split by environment |
| CLI | `environment` = `cli` | Daily profiles and runs, command health, product outcomes, failures, platforms, and account commands |
| Desktop app | `environment` = `desktop` | Launches and profiles, release adoption, outcomes by surface, sharing, chat and views, and failures |

## Definitions

- **Interface** comes from each event's `environment`: `cli` is the CLI,
  `desktop` is the desktop app, and `production`/`staging`/`dev` is the web app.
  The overview counts production web only; the Web app dashboard shows every
  web environment.
- **Profiles** are distinct complete profile hashes (`tonk:` followed by 64 hex
  characters). Anonymous and unresolved events are excluded. As in
  [the metric definitions](metrics.md), a profile is not a person or an account.
- **Outcome rates** use the attempt cohort of
  [`analytics-contract.md`](../analytics-contract.md): attempts older than ten
  minutes, matched by `attempt_id`; a missing terminal event counts as
  abandoned, not failed.

## Known overlap

Space views inside the desktop app run the web app, so their events report as
web (`production`), not desktop. Desktop users therefore also appear on the Web
app dashboard and in the overview's web counts. Separating them needs the web app
to know it is embedded and report that, which is not yet implemented.

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
