# UI profiling baseline

The local S1 observed-readiness procedure passed its frozen calibration on
2026-09-17: **360/360 sessions**, no retries or failures, and 720 clear
before/after environment admission checks.

| Collection | Median paired B-A | Exact median interval | Result |
| --- | ---: | ---: | --- |
| Matched 500 ms control | +461.5 ms | +446.0 to +497.5 ms | PASS |
| A/A baseline | -4.4 ms | -30.9 to +16.5 ms | PASS |
| Independent A/A repeat | -7.4 ms | -25.3 to +18.8 ms | PASS |

Intervals have 97.266% coverage under independent pairs. The fixed acceptance
budgets were +/-100 ms around zero for A/A and around 500 ms for the control.
This establishes a bounded local calibration, not zero bias or arbitrary precision.

The baseline is **1224.6 ms median** over 120 uninjected fresh-profile sessions
(Q1/Q3 1200.0/1256.0 ms). Its independent 120-session repeat is **1221.1 ms**
(Q1/Q3 1196.8/1243.5 ms). Do not include injected control timings in the baseline.

- [Machine-readable baseline](s1-calibrated-baseline-20260917.json)
- [Calibration history, failures and limitations](harness-calibration.md)
- [Reference collection](calibration-v5-ac-aa1-20260917/summary.json)
- [Independent repeat](calibration-v5-ac-aa2-20260917/summary.json)
- [Matched sensitivity control](calibration-v5-ac-delay500-20260917/summary.json)
- [Commands and protocol](../../scripts/perf/README.md)

Scope: desktop Chrome 153.0.8010.48 at 1280x800 on an AC-powered M3 Pro,
local test services and pinned release build `7c0b391f8bd92952`. The measured
endpoint is the displayed nested Welcome headline and enabled share control;
trusted-click menu checks and artifact verification run after timing. This is
not paint timing, field INP, mobile/Safari or production-network evidence.

This PR contains profiling/test infrastructure only. The reference release
artifact includes the earlier CSS correctness fix, which is **not** included
as a product change here. Other scenario fixtures and historical comparison
commands are not certified by this S1 calibration. Future product comparisons
need fresh paired A/B collection and an independent confirmation.

## Evidence inventory

Committed calibration directories preserve all indexed sample rows, summaries,
protocols, manifests and collection-level environment receipts, including failed
and interrupted attempts. The final v5 acceptance directories also preserve all
720 per-session admission receipts, losslessly bundled into one
`environment-samples.jsonl` per collection. Generated screenshots, per-request/result
duplicates and verbose runner/startup logs remain in the original experiment
workspace; absolute paths in receipts are provenance, not portable file links.
No sample was discarded to turn a failed acceptance collection into a pass.
