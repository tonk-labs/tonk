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
