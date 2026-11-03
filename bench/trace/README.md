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

`gate.mjs` is the network: `open`, `latency`, `offline` (refuse at once) or
`stall` (accept and never answer, so nothing fails and nothing falls back).
Connections opened before a switch are left alone, as a real network would.
The UCAN remote the worker syncs with is the access service itself, which the
gate does not sit in front of.

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
- `gate.log`: every request the gate saw, with the mode it was in. In `stall`
  mode, this log names the request the page is waiting on.
- `requests.jsonl`, screenshots, and the access service's and proxy's logs.

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
