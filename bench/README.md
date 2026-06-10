# bench — agent-episode benchmarks for the slide ↔ tonk-ui flow

## Usage

    bench/bin/bench run <scenario>        # full run: stack, episode, screenshots, judge, report
    bench/bin/bench run smoke --scripted  # plumbing check without spending a claude episode
    bench/bin/bench report [N]            # trend over the last N runs (default 10)
    bench/bin/bench baseline <scenario> <run-dir>   # promote a run's screenshots to baselines

Runs land in `bench/runs/<timestamp>-<scenario>/` (gitignored):
transcript, screenshots, metrics.json, judge.json, scores.json, report.md.

Requires the repo devshell (caddy, trunk, chromedriver via $CHROMEDRIVER,
jq, imagemagick) plus the `claude` CLI and Chrome at the default
/Applications path.

## Notes from implementation

### site.sh: remote URL includes /ucan/ suffix

The access service mounts its UCAN endpoint at `/ucan/`, so `site.sh` passes
`$BENCH_URL/ucan/` to `slide remote add origin`.

### Local stack design

The local stack is hermetic: `tonk-access-local` runs the native access service
(in-process S3 via `LocalS3`, bound to a random port) and Caddy serves the
trunk dist with `/ucan/*` proxied to it — same-origin like prod. The remote URL
for slide is `$BENCH_URL/ucan/`.

### Post-join sync

After `bridge.sh` completes the join, the browser replica needs to pull from the
shared remote before concept data is visible. `join.rs` calls
`POST /api/repository/{name}/branch/main/sync/pull` right before the post-claim
navigation, so the data is usually present by the time the browser lands on
`/space/{name}`. The sync is handled entirely by the in-browser service worker.

`bridge.sh` fires a second pull (idempotent) and polls until it confirms success
to make the guarantee deterministic for screenshots. The background auto-sync
tick is 20 s (`TICK_INTERVAL_MS` in `sync_controller.rs`) but the explicit pull
brings the replica to HEAD immediately.

The `POST /api/...` routes are all handled by the service worker, not by Caddy.
Synchronous XHR from `execute/sync` (WebDriver) works because the SW intercepts
page-level fetches regardless of sync flag. Caddy only sees `/ucan/*` traffic.

### Concept route shape

`slide share concept <name>` generates `then=concept/<name>` in the launcher URL.
After join, tonk-ui navigates to `/space/<local-name>/concept/<name>`. That URL
matches the chromed route `space/:space/concept/:source` and renders
`TonkConceptView` — an auto-generated table showing the concept's fields and
any asserted instances.

The `*subject` wildcard display route (`space/:space/*subject`) is defined in
`launcher.rs` AFTER the chromed `ParentRoute` so it does not shadow the
`concept`/`view`/`layout`/`board` routes. Leptos 0.8 router matches in
definition order; a wildcard defined before a specific route wins unconditionally.
