# bench — agent-episode benchmarks for the slide ↔ tonk-ui flow

## Usage

    bench/bin/bench run <scenario>        # full run: stack, episode, screenshots, judge, report
    bench/bin/bench run smoke --scripted  # plumbing check without spending a claude episode
    bench/bin/bench report [N]            # trend over the last N runs (default 10)
    bench/bin/bench baseline <scenario> <run-dir>   # promote a run's screenshots to baselines

Runs land in `bench/runs/<timestamp>-<scenario>/` (gitignored):
transcript, screenshots, metrics.json, judge.json, scores.json, report.md.

Requires the repo devshell (wrangler, trunk, chromedriver via $CHROMEDRIVER,
jq, imagemagick) plus the `claude` CLI and Chrome at the default
/Applications path.

## Notes from implementation

### site.sh: remote URL includes /ucan/ suffix

The plan template used `BENCH_URL` bare as the remote URL. The access service
mounts its UCAN endpoint at `/ucan/`, so `site.sh` passes `$BENCH_URL/ucan/`
to `slide remote add`. Confirmed from `rust/tonk-access-service/src/lib.rs`:

    .post_async("/ucan/", handlers::ucan::handle)

### BLOCKED: local R2 round-trip fails — access service needs real R2 credentials

`slide status` reports `no-upstream` after `remote set-upstream` because the
first `slide status` (or any push) tries to fetch the upstream head and the
access service returns 500:

    POST /ucan/ 500 Internal Server Error
    {"error":{"code":"INTERNAL_ERROR","message":"Missing R2_ACCESS_KEY_ID: Binding `R2_ACCESS_KEY_ID` is undefined."}}

Root cause: the access service worker reads `R2_ACCESS_KEY_ID` and
`R2_SECRET_ACCESS_KEY` via `ctx.secret()`, which in wrangler dev requires a
`.dev.vars` file. The `bench/wrangler.bench.toml` does not supply these.
More fundamentally, the worker generates presigned S3 URLs pointing to
`https://{R2_ACCOUNT_ID}.r2.cloudflarestorage.com` — the real Cloudflare R2
S3 endpoint — so even with dummy credentials in `.dev.vars`, the slide client
would attempt to PUT/GET against Cloudflare's network, not against miniflare's
local R2 binding.

The local R2 binding (`[[r2_buckets]]`) is configured but not used by the
handler. The wrangler-R2-parity spike therefore requires either:

1. Supplying real R2 credentials (via `.dev.vars` with actual keys) and
   accepting that pushes hit Cloudflare's R2, or
2. A worker variant that routes through the local R2 binding when
   `R2_ACCOUNT_ID` is a sentinel value (e.g. `bench-local`).

`bench/bin/site.sh setup` and `invite` work correctly up to the point of the
first network I/O. The CLI surface (`slide init`, `remote add`, `remote
set-upstream`, `invite --remote`) is confirmed correct.
