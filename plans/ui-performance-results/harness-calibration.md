# S1 harness reliability, 2026-09-17

Status: LOCAL S1 CALIBRATION PASSED; baseline recorded below. Earlier
diagnostic results are retained. Preparing a harness-only PR; no new product
optimization has been evaluated. Earlier E03/E04
screens show no demonstrated benefit, not established causal regressions.

## Scope and acceptance

Calibrate fresh-profile desktop Chrome navigation to the actual nested Welcome
headline and enabled share control. Keep the existing post-timing trusted share
interaction and artifact checks. This does not certify paint timing, mobile,
production networking, or the S6 interaction benchmark.

1. Diagnose with the accepted E02 immutable artifact. Record navigation wait,
   each readiness-poll duration and total observation time using the same native
   monotonic clock. Do not infer that the nominal 50 ms sleep bounds observer lag.
2. Run identical-artifact A/A with balanced, seeded, randomized adjacent AB/BA
   pairs. Preserve every failure and all samples. Build before collection.
3. Fix one demonstrated measurement problem at a time; version the procedure
   when its endpoint or observation method changes. Never mix old/new numbers.
4. For a fixed final procedure, require two independent complete 60-pair A/A
   collections. Each must have no failures, consistent browser/artifact/fixture
   identity, and a distribution-free 95% interval for the median paired B-A
   difference wholly within +/-100 ms. Report interval coverage and order effects.
   This calibrates a 100 ms resolution, not arbitrary precision or zero bias.
   The initial 20-pair budget was increased prospectively in cycle 3 using
   completed pilot data; failed earlier collections remain failed.
5. Validate sensitivity using a harness-only known delay at the readiness
   boundary, with the delay reported explicitly and never used in product
   comparisons. A/A precision alone is insufficient to certify the endpoint.

If calibration fails, preserve it and investigate; never rerun unchanged until
it passes. A final protocol change requires both repeat collections again.
Application experiment evaluation stays paused until the evidence supports it.

## Initial inspection

- Clean worktree at `64094d903a909652c40c5708eb6643708b3d73be`.
- Old `/tmp` artifact links are absent, but the E02 Nix store artifact remains.
- The attempted build hit a read-only Nix fetcher database. No source failure;
  reuse and verify the existing immutable artifact for calibration instead.
- Current observer makes repeated WebDriver calls across two opaque frames and
  returns to the outer frame to inspect the share control. Its overhead is not
  separately recorded. Navigation uses the default page-load wait.
- Use the repository's disposable-profile native browser runner: preserving the
  measured fixture is more useful here than a separate exploratory browser CLI.

## Cycle 1: diagnostic pilot

`calibration-v1-pilot-20260917/`: six balanced randomized pairs, 12/12 successful.
The median readiness observation was 1269.4 ms (range 1202.4–1443.3 ms).
Navigation returned at median 45.2 ms; final observer traversal cost median
71.2 ms (range 67.0–72.7 ms). Thus 50 ms polling is not a 50 ms error bound.

Median paired B-A was +15.0 ms; the exact sign-test median interval was
[-54.5, +123.1] ms (96.875% coverage). This small pilot did not meet the +/-100 ms
precision gate. It is diagnostic, not a failed final 20-pair acceptance campaign.
No samples were discarded. The next steps remain the preregistered sensitivity
control and two 20-pair acceptance collections, not repeated six-pair attempts.

The collector uses a prebuilt native test executable and a pinned prebuilt test
server; Nix/Cargo are not invoked between sessions. The artifact SHA256 matches
the accepted E02 record. The original release and ordinary browser profiles are
unchanged. The sensitivity control is explicitly marked in each request/result
and delays actual headline visibility within the inner frame.

## Sensitivity control boundary failure

`calibration-v1-delay500-20260917/` failed its first session with
`positive control did not hide headline`. Document-start CDP injection did not
install the control in the opaque inner frame. The failure was preserved and
collection stopped. No timing from it is evidence of sensitivity.

The revised control enters that frame using the existing WebDriver traversal,
waits for the exact visible headline, hides it for 500 ms, and requires later
polls to observe it visible again. Post-timing verification checks its own
same-realm hide/release timestamps. This avoids assuming CDP covers every realm.
`calibration-v1-delay500-frame-20260917/` is a new collection with that fix.

That control completed 12/12 sessions and recovered +496.7 ms median for the
500 ms delay. Its six-pair interval remained wide [+250.4, +1431.1] ms. The
first three sessions had individual observer calls of 113–303 ms, versus
roughly 65–73 ms later. This identifies observer latency as a material contributor,
not the sole cause of overall application variance.

## Cycle 2: reduced traversal, frozen acceptance collections

Observer v2 returns directly from the inner frame to its parent and resolves the
share control's shadow-root selector in one command. It preserves WebDriver's
displayed/enabled checks, exact headline text, both frame visibility checks, and
post-timing guards. It removes five redundant WebDriver commands from the final
successful poll. The metric/protocol version is bumped to v2: historical v1
timings must not be used as the comparison arm for v2 observations.

Before collecting: 73 Python tests and 8 focused native tests passed. The fixed
acceptance batch uses the same prebuilt runner and artifact for all three
collections, with 20 pairs each and no changes during collection:

- `calibration-v2-aa1-20260917/`, seed 2026091711, no injected delay.
- `calibration-v2-aa2-20260917/`, seed 2026091712, no injected delay.
- `calibration-v2-delay500-20260917/`, seed 2026091713, 500 ms B-only delay.

Both A/A intervals must fit [-100, +100] ms. The sensitivity interval must fit
[400, 600] ms. Preserve all three outcomes, including failures; this is one
fixed acceptance batch, not sequential sampling until a pass.

### Infrastructure failure and correction

The first v2 batch stopped at session 9 of A/A collection 1; the next two
collections each failed their first session. All three failures were before
browser startup (`No such file or directory`, immediately after access-service
startup). The previously verified test server
`/nix/store/x3iqnb0dplxmgpxax87m6r5g7mdx94ig-tonk-ui-test-server` had disappeared.
The runner/artifact and shell/Caddy executables remained. A path in the Nix store
is immutable while present but is not protected from garbage collection without
a GC root. We did not establish which process removed it.

Created indirect GC roots `/tmp/tonk-calibration-artifact` and
`/tmp/tonk-calibration-server`. The surviving server is
`/nix/store/276bvwgzlnx88aq8jhn8b0497qx27zf6-tonk-ui-test-server`, which supports
the same supplied-artifact override and generation fixture and uses the same
Caddy executable. The collector now records and checks server and runner hashes.

The entire fixed v2 acceptance batch is restarted with the same seeds/counts in
new `calibration-v2-pinned-{aa1,aa2,delay500}-20260917/` directories. The earlier
partial batch is invalid for acceptance, never merged into the replacement.

## Cycle 3: sample-size calibration on battery

User confirmed continuing on battery, with AC recalibration later. All current
environment receipts report battery power and low-power mode off. No result
here will be described as AC-calibrated or device-independent.

The protected v2 A/A collections both completed 40/40 successful sessions but
missed the +/-100 ms gate: AA1 interval [-60.6, +117.0] ms; AA2 interval
[-20.7, +102.0] ms (95.861% coverage each). Their exploratory pooled 40-pair
interval is [-19.3, +62.8] ms around a +9.7 ms median. Pooling is for sample-size
planning only and does not turn either failed acceptance collection into a pass.

The protected sensitivity collection failed at session 8: Chrome timed out
navigating to `about:blank` after WebDriver connection, before the profiler's
`driver-ready` point or Tonk navigation. The renderer reported a 60-second
timeout. This is a harness setup failure, not an application timeout or a timing
sample. Its root cause is unproven. Keep the failure and rerun the entire
invalid control collection; do not retry/replace that one sample.

For sample-size planning, center the 40 observed paired differences at their
median and draw 3,000 bootstrap datasets per proposed size using seed 20260917.
Estimated fractions meeting the exact +/-100 ms precision gate: 20 pairs 60.0%,
40 pairs 88.5%, 60 pairs 97.3%. This is an exploratory planning model, not a power
guarantee. Choose **60 pairs per independent A/A collection**, fixed in advance:

- `calibration-v2-battery60-aa1-20260917/`: seed 2026091721, 60 pairs.
- `calibration-v2-battery60-aa2-20260917/`: seed 2026091722, 60 pairs.
- `calibration-v2-battery-delay500-20260917/`: seed 2026091723, 20 pairs.

Keep v2 observer, immutable artifact, server, two-second between-session pause,
and acceptance intervals unchanged. Complete both A/A collections regardless
of the first statistical outcome. Stop on a harness failure. This changes the
sampling budget, not the accuracy threshold; earlier failed collections remain
failed. A future A/B comparison needs this calibrated sample count and schedule.

## Cycle 4: isolate browser startup failures

The first 60-pair collection stopped at session 58 with `tab crashed`; 57
sessions had succeeded. No replacement samples or follow-on collections ran.
Read-only process inspection found no surviving benchmark Chrome/ChromeDriver/
Caddy processes. The memory snapshot showed roughly 4.7 GB free and no swap
activity; it does not establish the cause of the renderer crash.

A new ignored diagnostic starts fresh disposable browsers and executes only
the profiling setup commands on an empty page, without navigating to Tonk.
The original setup succeeded twice, then failed on its third `about:blank`
navigation with `Timed out receiving message from renderer: 60.000`. This
reproduces a browser setup failure independently of Tonk. It does not prove
that the separate tab crash had the same cause.

The next narrow spike skips the redundant initial `about:blank` navigation for
S1 profiling only, validates Chrome's initial URL is `about:blank` or `data:,`,
and leaves ordinary integration-test setup and the actual timed navigation
unchanged. Require the 100-start diagnostic before another timing campaign.
Additional failure context distinguishes S1 browser setup, navigation and
endpoint observation. One compile attempt hit ambiguous `Err(error.into())`
type inference; using `Err(error).context(...)` fixed that compile error.

The skip-navigation spike was rejected: this Chrome opens
`chrome://new-tab-page/`, not an empty document. The retained setup instead uses
WebDriver `pageLoadStrategy: none` for S1 only, navigates to `about:blank`, and
explicitly checks that URL before installing instrumentation. The native timer
still starts before the actual Tonk navigation and the same visible endpoint
ends it. Ordinary integration-test navigation retains its original strategy.

The corrected startup diagnostic passed **100/100 fresh-browser starts** in
73.69 seconds, compared with the earlier failure on the third start. This is
evidence that the explicit wait addresses the reproduced setup failure, not a
claim that every renderer crash is fixed. Phase diagnostics are in
`browser-startup-explicit-wait-20260917/`.

## Cycle 5: v3 battery acceptance

Version the page-load strategy change as `navigation-to-usable-observed-v3`.
Freeze this protocol, native runner, protected E02 artifact and server for:

- `calibration-v3-battery-delay500-20260917/`: seed 2026091730, 20 pairs,
  known 500 ms B-only visibility delay; interval must fit [400, 600] ms.
- `calibration-v3-battery60-aa1-20260917/`: seed 2026091731, 60 pairs,
  interval must fit [-100, 100] ms.
- `calibration-v3-battery60-aa2-20260917/`: seed 2026091732, 60 pairs,
  interval must fit [-100, 100] ms.

Run the sensitivity control first to catch application endpoint problems before
the longer A/A collections. All collections use fresh isolated Chrome profiles,
two-second between-session pauses, unchanged functional/artifact guards and no
builds during measurement. Keep failures and stop on harness errors; never pool
these with v1/v2 observations or declare AC calibration from battery results.

Environment correction: the v3 sensitivity collection's before/after receipts
both record **AC Power**, as does the first A/A collection's starting receipt.
Live `pmset -g batt` also confirms AC. The `battery` directory names were chosen
before collection and are misleading; preserve their immutable paths, classify
using their receipts, and do not pool with the earlier battery collections.
These endpoint receipts do not prove uninterrupted power or background idleness.

The v3 20-pair sensitivity collection completed 40/40 successful sessions.
Median paired effect: +529.9 ms; exact 95.861% interval [+390.5, +674.2] ms.
It detects the injected delay but **fails** the preregistered [400, 600] ms
accuracy gate. This is not a calibration pass. Continue the two fixed A/A
collections without changing their acquisition or acceptance criteria.

### Concurrent-build discovery and user-directed pause

During AA1, a read-only process snapshot found many concurrent `rustc` processes.
A subsequent snapshot showed one compiler at 540% CPU; later inspection still
found two Cargo processes and multiple compilers. These were not launched by the
calibration collector, which invokes only the prebuilt native test executable.
This establishes a concrete host-load confound during AA1, not that compilation
explains every earlier outlier or the sensitivity interval.

Interrupted only the verified calibration controller. Its active test finished
independently, and subsequent process inspection confirmed both had exited.
Retained 51 indexed successful AA1 sessions plus the final unindexed successful
raw `result-051.json` and log. Do not append that orphaned result to manufacture
a complete collection. AA1 is incomplete and environmentally confounded;
AA2 never started. The completed sensitivity control remains diagnostic only.
No unrelated build or personal browser was stopped.

The user then explicitly chose **"Keep this as a diagnostic run; calibrate
later."** No further timing collections are authorized in this working session.

## Checkpoint for later calibration

The collector now uses protocol `s1-aa-calibration-v4`, retaining the unchanged
v3 observed endpoint. It records bounded process/power receipts immediately
before and after each session. Recognized Cargo/compiler/build processes block
acquisition; missing process/power inspection or changed power configuration
also prevents acceptance. An affected completed sample keeps its raw timing but
is marked `environment_invalid`, never counted as a successful timing sample.
Process receipts contain only recognized build names/PIDs, not personal command
arguments. No process is automatically stopped.

The live admission check correctly rejected the current concurrent compilation.
These boundary snapshots cannot detect every brief build, other benchmark or
background workload between checks. A user-coordinated quiet window is still
required; this is not automatic certification of an idle host.

When resumed:

1. Agree a quiet window and verify actual power state; choose new receipt paths
   named for that state. Build/pin inputs before timing. Do not reuse v3 samples.
2. Freeze the collector revision, seeds and sample budget before collecting.
   Retain two independent 60-pair A/A collections and the +/-100 ms gate.
   Revisit the sensitivity sample budget explicitly: the completed 20-pair
   control detected +500 ms but lacked the preregistered precision. Do not retry
   the same collection until it happens to pass or widen the gate afterward.
3. Require complete collections, functional/artifact guards, consistent browser,
   acceptable environment receipts and the known-delay accuracy gate before
   resuming optimization evaluation. The historical five-run ABBA comparison
   command is not certified by this work.

Validation at pause: **75 Python tests, 8 focused Rust profiling tests and 5
native browser-helper tests passed**; formatting and diff whitespace checks
passed. The helper cleanup test initially failed under restricted process access
and passed unchanged with appropriate access. The 100-start browser diagnostic
is recorded above. Full workspace tests, mobile/Safari, presentation timing and
a completed quiet-host acceptance campaign remain unverified. No production
runtime optimization was added or removed during this calibration work.

## Cycle 6: user-authorized quiet-window AC calibration

The user authorized resumption on 2026-09-17. At 17:46 UTC the admission check
found no recognized build processes, AC power and low-power mode off. Artifact,
runner and server hashes match the v3 manifest exactly. Use the existing native
runner, not a separate browser-testing CLI, to preserve the measured fixture.

Freeze collector protocol v4 and the unchanged v3 observer for three **new**
collections, in this order:

- `calibration-v4-ac-delay500-20260917/`: seed 2026091740, 60 pairs, B-only
  500 ms delay; exact median interval must fit [400, 600] ms.
- `calibration-v4-ac-aa1-20260917/`: seed 2026091741, 60 pairs, no delay;
  exact median interval must fit [-100, 100] ms.
- `calibration-v4-ac-aa2-20260917/`: seed 2026091742, 60 pairs, no delay;
  exact median interval must fit [-100, 100] ms.

Increase the sensitivity budget from 20 to 60 pairs prospectively because its
completed 20-pair diagnostic lacked precision. This changes sample size, not the
accuracy gate, and is not a guarantee of passing. No early stopping for favorable
statistics, discarded samples, or pooling of earlier results. Complete all three
unless a collection has failures or environmental admission blocks it. Keep the
two-second between-session pause and before/after environment checks. No builds
or unrelated tests during timing. Record collector source hashes with the batch.

The user also requested a **profiling-harness-only PR once the baseline is set**.
After successful calibration and baseline recording, prepare a clean branch
excluding the existing branch's product changes. Include only harness code,
tests and relevant documentation/evidence. Do not open it with a claimed settled
baseline while calibration remains incomplete or failed.

Cycle 6 completed all 360 sessions without functional, harness or environment
failures. AA1 passed: median -19.2 ms, interval [-37.1, +6.0] ms. The second A/A
also passed (see its immutable summary). The sensitivity control **failed**:
median +584.7 ms, interval [+564.8, +609.0] ms versus the frozen [400, 600] gate.
All intervals have 97.266% exact coverage. Browser-side hide durations were
500.7 ms median, range 500.1-502.8 ms, so timer inflation does not explain the
roughly 85 ms excess observed difference.

## Cycle 7: isolate sensitivity-control overhead

Code inspection identifies an asymmetric control: only B hides an already
observed headline and requires another traversal/poll. A proceeds immediately
to the share check. Hypothesis: this extra observation cycle contributes to the
control's positive bias, separate from application noise. Test this without
changing the native runner or artifact:

- `calibration-v4-ac-delay1-diagnostic-20260917/`, seed 2026091750, six pairs,
  B-only **1 ms** visibility delay. This is an overhead diagnostic, not acceptance.

If supported, match the control operation in both arms: hide A for 1 ms and B
for 501 ms, preserving a 500 ms duration difference and the original accuracy
gate. Keep normal A/A observations free of injected controls. Validate the
matched control in a small diagnostic before freezing another complete
acceptance batch. Preserve cycle 6's failed control; no threshold changes or
post-hoc subtraction of its measured bias.

The 1 ms diagnostic completed 12/12 sessions: median +51.6 ms, interval
[-13.7, +92.1] ms. This small sample supports testing the code-identified
asymmetry but does not establish its exact magnitude or exclude zero effect.

Collector v5 now assigns 1/501 ms delays to sensitivity A/B by default and
records both doses and the expected 500 ms difference. Ordinary A/A still uses
0/0 ms (no injected control); native runner/server/artifact are unchanged.
Unmatched positive-delay controls are explicitly diagnostic and cannot pass
v5 acceptance, even if their interval fits the numerical budget. Seven focused
collector tests passed, including matched-dose and invalid-dose coverage.

Next frozen diagnostic: `calibration-v5-ac-matched500-diagnostic-20260917/`,
seed 2026091751, six pairs, 1/501 ms. This is not final acceptance and is not
pooled with earlier controls. If viable, preregister another full 60-pair
sensitivity collection and two full 60-pair A/A repeats under collector v5.

The matched-control diagnostic completed 12/12 sessions: median +463.9 ms,
six-pair interval [+359.5, +573.3] ms. Its interval is too wide for acceptance;
the point estimate supports proceeding to the larger fixed sample. This result
is retained as a pilot, never counted toward final acceptance.

## Cycle 8: frozen matched-control acceptance

Freeze collector v5, unchanged native runner/server/artifact and the same
environment and functional guards. No builds or tests during collection. Run:

- `calibration-v5-ac-delay500-20260917/`: seed 2026091760, 60 pairs, 1/501 ms
  matched control; interval must fit [400, 600] ms.
- `calibration-v5-ac-aa1-20260917/`: seed 2026091761, 60 pairs, no injection;
  interval must fit [-100, 100] ms.
- `calibration-v5-ac-aa2-20260917/`: seed 2026091762, 60 pairs, no injection;
  interval must fit [-100, 100] ms.

Complete all collections regardless of intermediate statistical outcomes, but
stop on failures or environmental admission errors. No optional stopping,
replacement samples or pooling across versions. Although ordinary A/A execution
is unchanged, repeat both collections under the final collector as required by
the original acceptance plan. All **76 Python tests passed** before collection.
If all gates pass, use the first complete A/A collection as the descriptive S1
baseline and the second as its independent repeat; report their readiness
distributions separately, not just the near-zero paired A/A effects.

## Accepted local baseline and calibration

Cycle 8 completed **360/360 sessions**, with no failures or retries. All 720
per-session before/after environment receipts show available process inspection,
no recognized builds, AC power and low-power mode off. Collector source hashes
still match the frozen batch receipt. Browser identity and artifact/runner/server
identities remained consistent. These snapshots do not certify continuous
idleness or other machines.

| Collection | Median paired B-A | Exact median interval | Gate |
| --- | ---: | ---: | --- |
| Matched 500 ms control | +461.5 ms | +446.0 to +497.5 ms | PASS: within 400-600 ms |
| A/A baseline | -4.4 ms | -30.9 to +16.5 ms | PASS: within +/-100 ms |
| A/A independent repeat | -7.4 ms | -25.3 to +18.8 ms | PASS: within +/-100 ms |

Each interval has 97.266% discrete coverage, assuming independent pairs. Order
medians (AB / BA) were +468.4/+460.7 ms for control, +2.2/-23.7 ms for AA1,
and -3.4/-10.3 ms for AA2. Passing a 100 ms budget does not establish zero bias
or arbitrary precision: the control's point estimate under-recovers by 38.5 ms.

The reference baseline is **AA1's 120 uninjected sessions**: median **1224.6 ms**,
Q1/Q3 1200.0/1256.0 ms, range 1152.4-1597.3 ms. The independent AA2 repeat is
1221.1 ms median, Q1/Q3 1196.8/1243.5 ms, range 1148.8-1460.4 ms. Do not combine
injected control timings into baseline distributions or pool acceptance tests.

Scope: fresh-profile desktop Chrome 153.0.8010.48, 1280x800, Apple M3 Pro,
the pinned E02 release artifact (build `7c0b391f8bd92952`), local test services,
observed nested Welcome/share readiness. This is not presentation timing, field
INP, mobile/Safari, a production-network result, S2/S3/S6 calibration or proof
that earlier product candidates regress. The artifact includes the earlier CSS
correctness fix; its product changes are not part of the harness-only PR.

The historical five-run ABBA comparison is still not certified. Future product
comparisons need fresh A/B under this paired schedule, functional guards and
confidence bounds, followed by independent confirmation. No optimization is
retained on the strength of this calibration alone.

## Harness-only PR extraction

The PR is based on current staging, not the product experiment history. Its S1
observer and browser-helper source are byte-identical to the calibrated source;
collector source also matches the frozen v5 receipt. The pinned release and test
server used for timing are identified in the manifests, not rebuilt from PR HEAD.
Newer upstream helper-server changes were preserved while merging the optional
S6 probe. The probe now remains inactive until a fixture explicitly configures
it, with a regression test; this does not change the already-pinned S1 server.

The PR includes compact indexed calibration evidence, not the earlier product
experiment reports or optimization code. Final per-session environment receipts
are losslessly bundled in `environment-samples.jsonl` (240 per collection).
Verbose logs, screenshots and duplicate individual request/result files remain
in the original experiment workspace. The original calibrated native binary was
also preserved locally as `/tmp/tonk-calibration-native-v3-20260917` before build
cache reuse. None of these local-only files is required to parse the committed
timings, protocol, manifests, admission receipts and verdicts.

Fresh checks on the clean staging-based PR branch: 76 Python tests, 21 Rust
profiling tests, five native browser-helper tests and three sync-probe tests
passed. Three opt-in browser fixtures remained ignored. Formatting, whitespace
checks and all-target/all-feature Clippy for `tonk-ui` and
`tonk-access-service` passed with warnings denied. Auxiliary fixture lint fixes
do not change the calibrated S1 source. Full workspace, Safari/mobile and hosted
CI validation are not claimed here.
