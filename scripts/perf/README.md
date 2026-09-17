# UI performance campaigns

Standard-library Python orchestration, calibration and deterministic decision
calculators for local UI profiling. Start with the
[accepted S1 baseline](../../plans/ui-performance-results/README.md).

## Calibrated local S1 procedure

The local S1 observed-readiness procedure passed calibration on 2026-09-17:
360/360 sessions, two independent A/A passes and a matched 500 ms control.
The reference baseline is 1224.6 ms median; its independent repeat is 1221.1 ms.
This is a local 100 ms accuracy/precision budget, not arbitrary precision. Follow
[`harness-calibration.md`](../../plans/ui-performance-results/harness-calibration.md)
for current outcomes, failed attempts and the frozen acceptance procedure.

`calibrate_observed.py` runs one **identical-artifact** collection of randomized,
balanced adjacent AB/BA pairs. Build the native runner and server first, protect
Nix inputs with GC roots, then run inside the development environment:

```sh
python3 scripts/perf/calibrate_observed.py \
  --artifact /tmp/tonk-calibration-artifact \
  --runner target/debug/deps/<native-tonk-ui-test-executable> \
  --pairs 60 --seed 2026091731 --output <new-directory>
```

Set `TONK_UI_TEST_SERVER` to the protected prebuilt server executable. The
collector records runner/server/artifact identities, invokes no builds between
samples, and preserves failures without retries. Protocol v5 also records
process/power admission checks before and after every session, stopping on known
build processes, unavailable inspection or changed power. These snapshots are
not proof of continuous host idleness: arrange a quiet window first. An
`environment_blocked`/`environment_invalid` collection cannot pass. Run a separate fixed collection
with `--delay-ms 500` for the browser visibility sensitivity control: A hides
for 1 ms and B for 501 ms, matching the hide/reobserve work while adding 500 ms.
Both doses appear in the protocol and requests/results; never use these samples
as product baseline data. Ordinary A/A has no injected control. An explicit
`--control-base-ms 0` with a positive delay is diagnostic-only and cannot pass.

The reported effect is the **median within-pair B-A difference**, not a difference
of pooled medians. Its distribution-free sign-test interval has at least 95%
coverage under independent pairs; the exact discrete coverage is reported. A/A
passes precision only if the complete interval fits within +/-100 ms, with no
failures and one browser identity. Two independent full A/A passes and the
known-delay check are required before calling this local setup calibrated.
This does not validate paint timing or other workloads/devices.

Observer v3 retains the v1 semantic endpoint, removes redundant WebDriver
frame/selector calls, and uses explicit readiness observation with WebDriver
`pageLoadStrategy: none`. Results identify `navigation-to-usable-observed-v3`;
never pool them with historical v1/v2 samples. The existing `compare-observed`
command still uses the historical five-run A/B/B/A screen and is **not certified
by paired calibration**. Future optimization evaluation must use the calibrated
pair schedule, fresh A alongside B, confidence bounds and a separate confirmation.

## Historical commands

The primary revised-plan commands are `baseline` and `compare-observed`. They run
the small S1 observed-readiness procedure with tracing disabled, fresh disposable
Chrome profiles, exact Welcome/share semantics, post-timing functional checks and
artifact verification. The other commands below preserve the superseded broad
campaign framework and its historical receipts.

Earlier product experiment records remain on the experiment branch; this
harness-only change includes calibration evidence, not product optimization code
or a new verdict on those experiments.
The Rust runner supports **S1 desktop headline/control observations and a trusted
share-menu click**, **S2 desktop returning linked-account observations**, and
**S3 desktop direct-space and repeated-menu observations**. Event Timing is recorded when the browser exposes a matching
interaction. DOM observations remain diagnostics, not trace-validated presentation.
Required timing, resource, fixture and environment evidence remains unavailable;
these samples cannot produce RETAIN.

`perf_environment.py --output <new-receipt.json>` captures bounded host hardware
and active power settings. It preserves unavailable fields as null and never
automatically certifies profile or environment validation. The S2 sampler provisions a real disposable linked account, restarts its profile
without a new authenticator, times visible Hub readiness, and performs account,
content and artifact readback afterward. S2 presentation, background-work and
offline-authorization guards remain unavailable. Its setup failures never use a
replacement browser profile to claim verification.

The S3 direct-space
and repeated-menu fixture has a separate ignored correctness smoke and is wired
into the campaign runner. Its observed completion and Event Timing diagnostics
do not establish the required presentation, subscription or resource metrics.
`cdp_transport.py` is a separate synthetic coverage probe, not production
request accounting; see the execution ledger for its current validation status.

```sh
python3 scripts/perf/ui-perf.py --help
python3 scripts/perf/ui-perf.py baseline --artifact /tmp/tonk-perf-b0 --output plans/ui-performance-results/s1-baseline-YYYYMMDD
python3 scripts/perf/ui-perf.py compare-observed --baseline /tmp/tonk-perf-a --candidate /tmp/tonk-perf-b --experiment E02 --output plans/ui-performance-results/e02-screen-YYYYMMDD
python3 scripts/perf/ui-perf.py validate --config scripts/perf/protocol.example.json
python3 scripts/perf/ui-perf.py calibrate --artifact /tmp/tonk-perf-b0 --config scripts/perf/protocol.example.json --output /tmp/tonk-perf-aa
python3 scripts/perf/ui-perf.py compare --baseline /tmp/tonk-perf-a --candidate /tmp/tonk-perf-b --experiment E01 --phase screen --config scripts/perf/protocol.example.json --output /tmp/tonk-perf-e01-screen
python3 scripts/perf/ui-perf.py compare --baseline /tmp/tonk-perf-a --candidate /tmp/tonk-perf-b --experiment E01 --phase confirm --config scripts/perf/protocol.example.json --output /tmp/tonk-perf-e01-confirm
python3 scripts/perf/ui-perf.py decide --results /tmp/tonk-perf-e01-confirm
python3 -m unittest discover -s scripts/perf -p 'test_*.py'
```

The example configuration is deliberately `frozen: false`, with no registered
experiments. Validation checks schema and prints its canonical SHA256; it does not
certify the fixture or freeze the protocol. Collection refuses unfrozen protocols.
Do not freeze this example until fixture identity, actual visible endpoints and
all required profiles/metrics are implemented and independently validated.

## Protocol and runner interface

`protocol.example.json` shows the schema. The required top-level boolean
`diagnostics_enabled` freezes the instrumentation mode. `selections` explicitly name each
scenario/profile, complete profile settings, fixture recipe, selector, input
sequence, dataset SHA256, cache state and metric definitions. Allowed metric kinds
are `startup`, `paint`, `completion`, `latency`, `offline`, `resource`. Zero-valued
resource baselines need a preregistered `zero_absolute_budget`; latency and offline
budgets already include an absolute threshold. Metric values are nonnegative,
finite session-level scalars or null (unavailable), never per-event arrays.
Metrics with a fixed product requirement, such as E05 idle freshness, may also
preregister an `absolute_ceiling`; any candidate session above it rejects the
experiment rather than allowing a favorable median to hide the breach.

The bootstrap contract is fixed: 10,000 draws, seed 20260916, linear quantiles,
cycle/block/seed/slot pairing. Each registered `experiments` entry has `hypothesis`,
`classification_reason`, a `primary` object containing exactly `scenario`,
`profile`, `metric`, and `affected`/`sentinels` lists of scenario/profile objects.
Classify every selection exactly once before running. Primary kinds are startup,
paint or completion, with the plan's fixed practical targets. Every declared metric
is also checked against its regression budget; omitted metrics cannot establish a
pass. The fixed per-scenario minimum inventory lives in `REQUIRED_METRICS` in
`ui_perf.py`. All sixteen S1–S8 desktop/constrained selections and per-selection
footprint metrics (total transferred/compiled bytes, peak/settled memory and CPU)
are required for retention. Additional experiment-specific guards must be declared.

Collection serially runs exactly:

```sh
nix develop . --command cargo test -p tonk-ui --features integration-tests performance::tests::it_profiles_supplied_artifact -- --exact --ignored --nocapture
```

It sets `TONK_UI_RELEASE_ARTIFACT` to the supplied immutable artifact directory,
`TONK_PERF_REQUEST` to an absolute JSON request path and `TONK_PERF_RESULT` to a new
result path. Requests include the selection, schema version, paired fixture seed,
slot and canonical fixture hash. Success results must echo scenario/profile,
profile settings, seed, slot and fixture hash and verify served identity. The runner
also records `identity_verified`, `profile_verified`, `fixture_verified` and
`environment_verified`. Missing identity is INVALID; missing profile, fixture or
environment validation is INCONCLUSIVE. Profile settings must match the selected
profile exactly. Unsupported scenarios/profiles are harness errors, never silently
substituted. No build or other performance run may overlap collection.

For a diagnostics-off smoke check, set `TONK_PERF_TRACE=0`. This disables the
performance log/trace collection while keeping the external headline observation
endpoint identical. Results record `diagnostics_enabled`; matching this mode across
comparison arms and measuring its overhead remain prerequisites to calibration.
For orchestrated campaigns, set the protocol's `diagnostics_enabled` boolean instead:
the collector sets `TONK_PERF_TRACE=1` or `0` from that frozen value, overriding the
ambient environment for both arms. Successful runs with a missing or different
diagnostics mode are INVALID. Changing the mode changes the protocol hash and
requires a matched rerun.

Screening uses one A/B/B/A cycle of five fresh sessions per block; confirmation
and A/A use three cycles of ten sessions per block. In confirmation, sentinel
selections retain screening counts. Rust creates a fresh isolated browser and
fixture per invocation. A1 pairs B1 and A2 pairs B2 using the same fixture seed and
slot. Each result records the original acquisition sequence: changing JSONL storage
order is harmless, changing the acquisition order is invalid. Whole-runner hangs
are bounded at 600 seconds, then its process group is terminated and a harness error
is retained. A runner-reported application timeout is a failed sample and REJECT.

## Artifacts and decisions

Output directories must be new. `protocol.json` and `manifest.json` preserve the
campaign settings and identities. `request-*.json`, `result-*.json`, `runner-*.log`
and append-only `runs.jsonl` preserve every attempted run, including failures.
Artifact trees are SHA256-hashed before/after samples. A missing result or a zero-test
invocation cannot count as successful execution. Harness failures stop collection.
Malformed runner output remains unchanged in its result file and receives a
sanitized harness-error row before collection exits. Rerun the complete affected
pair in a new campaign with invalidation evidence in the ledger, preserving the
original failed campaign. Do not edit failed observations.

`decide` independently emits `summary.json` (blocks, medians, p95, maxima, intervals)
and `decision.json` (verdict/reasons). Collection success is never a verdict.
`decide` exits 0 for RETAIN, 2 for REJECT, 3 for INCONCLUSIVE. Configuration,
identity or harness errors exit 1. Functional failures/timeouts force rejection;
runner-reported application failures remain failed samples even when the native test
exits nonzero. Missing sessions or metrics force inconclusive. Screening and
identical artifacts can never retain. Calibration additionally reports
`calibration_precision_pass`:
this evaluates regression-budget precision, not readiness of the entire harness.

Evidence must be reviewed separately and recorded in the manifest's `evidence`
object: `calibration_pass`, `instrumentation_overhead_pass`, `correctness_pass`,
`platform_coverage_pass`, `mechanism_pass`, `original_baseline_pass`,
`visual_endpoint_validated`, `full_matrix_coverage_pass`, `sentinel_review_pass`.
These are explicit attestations, not results inferred by the calculator. Link their
raw evidence in the results ledger. Inventory checks remain mandatory regardless
of attestations. Set manifest `final_integration: true` for the final original-B0i
comparison; any sentinel-only selection then blocks retention. The calculator does
not verify external compatibility evidence, campaign uniqueness across directories,
fixture product representativeness, or power-mode correctness itself. The executor
must enforce one confirmation campaign per candidate version and maintain the
ledger. The bootstrap is an uncertainty check; it cannot repair environmental bias.
