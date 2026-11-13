# bench/trace: what the interface does when things go wrong

`run.sh` starts a local stack (the access service, the `tonk-perf` front
proxy serving a trunk dist, and a network gate in front of both), seeds two
spaces from a CLI "other device", joins them in a fresh Chromium, and runs
scenarios that have made the interface reload or stall:

| Scenario | What it does |
|---|---|
| `cold-join`, `open-second-space` | Join both spaces through their invites |
| `warm-reload` | Reload a space the browser already has |
| `switch-spaces` | Move between the two spaces in place, four times |
| `idle` | Leave the tab alone (`TRACE_IDLE_MS`, default 90 s) so the worker can fill its offline copy |
| `restart-then-stall` | Stop the service worker, stall the network, reload |
| `two-reloads-on-stall` | Stall the network, then reload three times |
| `worker-restart` | Stop the service worker while a space is open |
| `remote-write-while-viewing` | The CLI writes and pushes; wait for the page to show it |
| `remote-write-then-stall`, `stall-before-sync` | Remote writes around a stalled network |
| `slow-reload`, `stalled-reload`, `offline-reload` | Reload and switch spaces at 2 s latency, on a network that never answers, and offline |

The network is `tonk-perf link`: a shaped TCP link in front of the page
(`page`) and one in front of the access service (`ucan`), the remote the
browser syncs with, its live-update socket and its block reads and writes
included. The access service names the `ucan` link in its object permits
(`ACCESS_PUBLIC_ENDPOINT`), so nothing the browser sends goes around it.
The CLI "other device" pushes to the service directly, so a stalled link
never holds up a remote write.

`TRACE_PROFILE` names the network the links start as (`lan` by default;
`broadband`, `mobile`, `slow3g`, `satellite`, `flaky`, `nat`; see
`tonk-perf profiles`). A scenario switches it through the control server
(`config.gate`): `GET /__gate?mode=open|latency|offline|stall[&ms=N]`.
`offline` resets every connection and refuses new ones; `stall` holds every
byte, on open connections too, until the link opens, when it is delivered;
`open` goes back to the profile. `/profile?name=...`, `/reset` and `/stats`
are there too, each optionally `&link=page` or `&link=ucan`.

## Output

Each run writes `bench/trace/runs/<stamp>/`:

- `events.jsonl`: one line per event, each with its scenario and frame
  (`top`, or `guest:<depth>`). The kinds are guest frames and views inserted and
  removed, the loading pulse and boot overlay shown and hidden, top-page loads,
  page and service worker console output, network switches, and whether the
  offline copy was ready when each scenario started.
- `summary.json`: per scenario, time to settle (content in a guest and no
  pulse for 1 s), top-page loads, guest frames built and removed, views
  removed, and pulse count and duration.
- `links.jsonl`: every HTTP exchange that crossed a link, with its link,
  kind (`ucan`, `object-get`, `object-put`, `socket`, `service`, `api`,
  `asset`), bytes, and when the browser sent it, got its first byte and its
  last. An exchange still unanswered when the run ends was never answered;
  in `stall` mode, those name what the page is waiting on.
- `inflight.json`: the exchanges still open when the run ended; one never
  answered is a request the page waited on to the end. `links-stats.json`:
  connections, bytes, resets and forgotten connections per link.
- `requests.jsonl`, screenshots, and the access service's and proxy's logs.

## Creating a space

`create-space.mjs` times a space's creation in a fresh browser, from the
press of the hub's create button to the new space's canvas, phase by phase:
the profile's command, the space's worker starting, its kernel's first boot
(handed over, installed, compiled, instantiated, initialized), the seed and
the renderer it keeps, its first page load and its first render. It reads
the phases from what the page shows and what the space's worker logs, over
the DevTools protocol, so it needs no Playwright.

```sh
TRACE_DRIVER=bench/trace/create-space.mjs bench/trace/run.sh
```

It writes `create-space.json` (each run's phases and their medians).
`CREATE_RUNS` sets the number of runs, 3 by default.

## Running

```sh
cargo build --release -p tonk-access-service --features helpers --bin tonk-access-local
cargo build --release -p tonk-cli -p tonk-perf
(cd rust/tonk-ui && trunk build --release)
bench/trace/run.sh                       # every scenario
TRACE_SCENARIOS=idle,two-reloads-on-stall bench/trace/run.sh --notes 50
```

`--dist` points at another trunk dist, and `TRACE_KEEP=1` leaves the stack
running after the run.
