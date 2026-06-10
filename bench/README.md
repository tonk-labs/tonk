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
