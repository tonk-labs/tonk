# Measure and reduce browser cold start

Measure wall time from the first application navigation in a fresh disposable
Chrome profile until the onboarding space content is visible. Use the release
build, a fixed local server, and five fresh browser sessions; report median and
range. Browser launch and build time are outside this measurement. Keep the
baseline release artifact unchanged for comparisons.

Use temporary measurement code. No preserved benchmark suite, tracing,
throttling, scenario matrix, or statistical acceptance framework is needed.
Compare each Wasm loading change using the same simple measurement, repeating
runs if the difference is within the observed variation.

Preserve opaque-origin guests, host-mediated IO, top-document passkey ceremonies,
exact worker-byte verification, and atomic offline generations. Do not change
account, routing, or onboarding behavior to improve the timing.

Baseline measured 2026-09-09: **2,169 ms median**, range **2,080–2,289 ms**.
Five runs: 2,169, 2,149, 2,289, 2,080, 2,174 ms. Headless Chrome
152.0.7977.83, 1200×900 viewport, local HTTPS Caddy file server with no
compression or throttling. Each run starts on `about:blank` in a fresh browser
profile. Stop when `.wp-outer` in the nested onboarding frame is displayed and
contains “makes your small software”; this includes DOM polling overhead.
The served release is source `686ef8c55172677e61f65322d642ae33bc29a54e`,
artifact `/tmp/tonk-wasm-cold-start-b0`, worker build `89de328c6bc04929`
(verified after timing on every run). This is a local loading baseline,
not an estimate of internet download time. The temporary test passed all five
runs and was removed afterward.

The following are candidate improvements to investigate one at a time.
Compiler-profile changes are retained; the execution log records the other
experiments and their retain/reject decisions.

## 1. Remove sequential guest asset waits

- [x] Parallel payload assembly is retained or rejected using the common loop,
  with required/optional asset failure semantics preserved.

In `tonk-portal/src/bridge.rs`, restructure `build_inject_payload()` so manifest
resolution unlocks concurrent glue, Wasm, WA CSS/JS and independent editor-shell
requests. Fetch snippets after glue parsing; fetch fonts after CSS discovery.
Use bounded concurrency for font/snippet lists and `fetch_bundle_graph()`;
preserve deterministic dependency identities and blob rewrite order. Do not
combine this with memoization or editor removal. Keep required glue/Wasm errors
fatal and optional-resource degradation consistent with current behavior.

Use controlled delayed responses in a real portal fixture to prove independent
requests start before their siblings finish; the current sequential loader
should fail that assertion. Test missing required assets, optional failures,
nested guests and navigation away during loading. Profile payload assembly,
usable content against the baseline.

## 2. Defer the code-editor core and language graph

- [x] On-demand code loading is retained or rejected, including measured editor
  first-use latency and offline behavior.

Follow the existing prose/table shell and `need-prose`/`need-table` mechanism.
Change `rust/tonk-code/src-js/` and its build entry points to emit a small element
registration shell; extend `tonk-portal/src/bridge.rs` with matching `need-code`
and injection messages. Inspect the actual package scripts and source layout
before editing. `build_inject_payload()` should include only the shell. First
connection requests the required core and language graph, memoizes concurrent
requests, and exposes a retryable failure instead of a permanently pending
promise. Keep language-relative import rewrites and prose-embedded code working.
Do not globally disable editor registration or infer required elements solely
from initial HTML; routed content can add them later.

The initial failing test is a non-editor guest fetching editor-core assets
through its foreground runtime loader. Background offline-cache requests must
be distinguished by initiator; their presence is not a lazy-loader failure.
Test late element insertion, two editors, nested guests, an embedded prose code
block, language selection, load failure/retry, and offline opening after a
complete cache fill. Rebuild checked-in JS with package-defined commands.
Measure the actual boot graph rather than the entire asset directory; the
initial inspection found 10 files / 674,825 raw bytes, which must be remeasured.

## 3. Compare compiler profiles one variable at a time

- [x] UI and worker profile variants have measured size/runtime results and
  only independently beneficial settings remain.

`Cargo.toml` currently defines thin LTO and one codegen unit for release;
`wasm-release` adds size optimization `s`. All three Trunk Rust links in
`rust/tonk-ui/index.html` specify `data-wasm-opt="z"`, but only guest selects
`wasm-release`. Test UI with `s`, then worker with `s`; test `z` against each
winner separately; finally compare full LTO independently. Use named profiles
and release-only Trunk selection so development behavior remains unchanged.
Verify the installed Trunk accepts the attributes before relying on them.
Keep `wasm-opt -Oz` fixed during these comparisons. Do not alter lockfiles,
panic behavior, or shared native release settings.

Inspect build logs to confirm the intended profile was used. Compare final
post-bindgen/post-wasm-opt artifacts, not Cargo intermediates. If attribution
is needed, inspect retained functions/data with twiggy or an equivalent Wasm
analyzer against a separate symbol-bearing build; a Cargo dependency alone
does not prove retained binary cost. Profile worker query/render throughput
and editor interactions as guards against size/speed tradeoffs. Restamp final
artifacts via the existing Nix post-fixup process after every transformation.

## 4. Reuse immutable guest preparation, then test compiled-module delivery

- [x] Preparation caching and compiled-module delivery each have independent
  retain/reject decisions and no cross-generation reuse.

First memoize immutable guest assets and prepared font/CSS data in the trusted
host, keyed by build/manifest asset identity and stylesheet identity. Preserve
per-injection theme and document state. Evict failed promises for retry and
bound retained memory to the active build. Do not cache a payload whose
transferred ArrayBuffers become detached after the first postMessage: retain
immutable source bytes and create safe per-recipient buffers. A two-guest test
should initially observe duplicated preparation, then prove reuse while both
guests receive valid bytes. Include nested guests and navigation teardown.
Profile first visit, nested guest startup, repeated route loads, and memory.

Only after deciding that change, test compiling Wasm in the host and delivering
a `WebAssembly.Module` through structured clone (not an ArrayBuffer transfer
list). Prove a minimal real sealed-iframe fixture works with this repository's
bindgen glue, CSP, Chrome and Safari before changing application loading.
Start compilation concurrently with other assets, using streaming compilation
where responses and MIME types allow it. Preserve a byte-based fallback for
unsupported environments and distinguish unsupported transport from malformed
Wasm failures. Each guest still needs its own instance, imports, and memory.
Test two simultaneous guests, retry, old/new build identity, and offline load;
measure compilation, memory, main-thread long tasks, and usable content.
Do not make the service worker instantiate unverified streamed bytes.

## 5. Schedule offline filling around foreground startup

- [x] Offline-fill scheduling has a retain/reject result without weakening
  update publication or eventual offline completeness.

If wall-time comparisons point to background contention, investigate `ensureOfflineGeneration()` and the
eight-request pool in `fetchVerifiedAssets()` in `assets/service_worker.js`.
If no overlap or material contention exists, record that evidence and reject
this hypothesis without production changes. Otherwise test a single scheduling
policy: limit first-install background fill to one request until usable content
is acknowledged, then restore eight. Add a bounded timeout fallback so a closed
or failed page cannot leave offline fill throttled indefinitely. Keep incumbent
update installation and its complete-before-takeover behavior unchanged.

Extend `tests/service-worker.test.mjs` with held responses to prove the limit,
promotion on acknowledgement/timeout, retry after interruption, and complete
generation publication. Continue exact byte verification. Profile foreground
time and total offline-ready time; explicitly report the tradeoff and reject
changes that strand incomplete generations. Do not claim total download bytes
fell merely because some work moved after first render.

## 6. Evaluate explicit optional Wasm modules, then binary splitting

- [x] Each proposed split has either a measured retained implementation or a
  documented rejection/compatibility blocker; no unmeasured split remains.

Use retained-size analysis to order candidates in
`tonk-guest/src/bin/guest.rs`: inspector/notebook, board, then any other clearly
optional registration surface. Also examine top-document registration/dialog
code reached by `tonk-ui/src/bin/ui.rs`. Test one boundary at a time as its own
Wasm module, with a small registration/loading interface and host-mediated
asset delivery. Delaying a Rust function call or moving code into a linked
crate alone does not reduce initial download size. Shared dependencies may
duplicate across modules; measure initial and total bytes plus memory.

Before integrating a split, prove a disposable fixture registers and exercises
one real optional element after asynchronous module loading, including shared
host events. Preserve late element insertion, nested guests, one-time custom
element definition and visible/retryable failures. For account code, preload
before the user action or require a fresh explicit ceremony action after load;
never lose WebAuthn user activation by inserting an async download before the
credential call. Check first-use latency; reject a split that only
makes the landing view faster at an unacceptable interaction cost.

Finally run a bounded Binaryen `wasm-split` feasibility experiment on the
largest remaining startup-dominating module. Use a startup execution profile
covering the fixed routes, keep the original unsplit artifact, and prove the
secondary loader works with bindgen imports/tables and first invocation of a
previously unexecuted function. Do not assume arbitrary synchronous Rust calls
can await module downloads. Test the loader contract before application
integration; reject it if it needs an unsupported suspension mechanism or
cannot preserve browser/offline compatibility. Any viable split must extend
the artifact manifest, `scripts/hash-guest.sh`, `_headers`, and
`scripts/stamp-service-worker.sh` so all chunks belong to the correct immutable
generation. Run the same performance loop; infrastructure complexity without
a significant measured win is a rejection.

## Execution log

2026-09-09: Implementation started from the clean baseline checkpoint. The
baseline artifact symlink still resolves to the original Nix release. Step 1
starts with a controlled-response browser test of the real
`build_inject_payload()` path:
hold glue and require independent asset requests to start before releasing it.
The initial Nix invocation was blocked by sandbox access to its fetcher-cache
SQLite database; the unchanged command was retried with host access. No runtime
optimization or retain/reject decision has been made yet.

Step 1 checkpoint: the held-glue browser test failed against the sequential
loader (0.51 s, expected overlap assertion), then passed with concurrent
assembly (0.01 s). The 46-test portal browser suite passed, including required
and optional fetch-rejection checks and existing opaque-frame/disconnect
coverage. A subsequent focused graph test passed, observing four concurrent
requests, one fetch of a shared dependency, and deterministic discovery order.
The implementation uses the existing workspace `futures` dependency (only its
portal dependency edge changes in Cargo.lock). Release comparison and loading-
time navigation/nested-runtime coverage remain pending; this is not yet a
retained performance optimization.

Step 1 correctness checkpoint: all 48 portal browser tests passed after adding
real opaque-frame byte transfer and removal during an in-flight injection.
That fixture initially missed the frame's listener; replacing a fixed delay
with an explicit ready message resolved the test failure. Portal-only Wasm
Clippy (`--all-targets --no-deps --locked -- -D warnings`) passed. The command
including dependency lints stopped at the existing `large_enum_variant` in
`tonk-identity/src/custodian.rs:23`; no identity code was changed.

Step 1 decision: **rejected; experimental source and tests removed**. The
release candidate `/tmp/tonk-wasm-cold-start-p1` (worker `7635e03477f21039`)
measured 2,253 / 2,089 / 2,037 / 2,632 / 2,037 ms: median 2,089 ms,
range 2,037–2,632 ms. Because the difference overlapped variation, the same
five-session loop was repeated sequentially with no build running:

- Preserved baseline: 2,140 / 2,183 / 2,252 / 2,122 / 2,249 ms;
  median 2,183 ms, range 2,122–2,252 ms.
- Candidate: 2,169 / 2,305 / 1,995 / 2,298 / 1,965 ms;
  median 2,169 ms, range 1,965–2,305 ms.

Every run passed the nested onboarding visibility and worker-build checks.
The repeated median difference is 14 ms and ranges overlap substantially;
there is no demonstrated repeatable cold-start win to justify retention.
The candidate guest Wasm grew from 3,279,122 to 3,296,354 bytes. This rejects
this implementation under the agreed local loading metric, not concurrent
fetching in every network environment. Temporary timing code was removed.

Step 2 in progress: the baseline import probe confirmed that the code bundle
registers its core diagnostics provider even with no editor present. A shell
experiment now registers only `tonk-code`; the existing implementation lives
in `src-js/editor.ts` as a controller for the same host element. The portal
adds `need-code`/`inject-code`, with a shared retryable core promise and a
visible retry button. `npm ci --ignore-scripts` and package `npm run build`
completed. Ten artifact tests passed, including shared loading, fetch failure
retry, values set before loading, delayed insertion, and detached elements.
Release build `/tmp/tonk-wasm-cold-start-p2` and real nested-guest editor tests
are pending. This experiment has no retain/reject decision yet.

Step 2 release checkpoint: `/tmp/tonk-wasm-cold-start-p2`, worker
`f25957d24d19fee8`, passed the real nested onboarding test. The unchanged
baseline failed with two foreground `tonk-code-lang-dialog-yaml.js` requests;
the candidate had none. Two late-inserted editors preserved pre-load values
and mounted in 75.9 ms. After waiting for the generation's `adopted` marker,
a Chrome offline reload followed by first editor opening passed in 47.4 ms.
This uses the repository's CDP offline-emulation approach. Boot graph analysis
reconfirmed baseline 10 files / 674,825 bytes versus current 1-file / 1,738-byte
shell; total offline assets are not reduced. Portal-only Clippy passed.
TypeScript found an existing unused diagnostic field in the moved module;
removing that field and its assignment made `tsc --noEmit` pass. The candidate
artifact predates that nonfunctional cleanup. Timing comparison, prose-embedded
code and language-switch checks remain pending.

Step 2 decision: **rejected; experimental source, generated assets and tests
removed**. First timing batch: 1,961 / 1,842 / 2,327 / 1,914 / 1,823 ms;
median 1,914 ms, range 1,823–2,327 ms. Sequential repeat:

- Baseline: 1,932 / 2,395 / 1,915 / 2,040 / 1,903 ms;
  median 1,932 ms, range 1,903–2,395 ms.
- Lazy candidate: 2,063 / 2,033 / 1,909 / 1,891 / 2,099 ms;
  median 2,033 ms, range 1,891–2,099 ms.

The direction reversed on repetition, so there is no demonstrated repeatable
wall-time improvement. Online/offline first use and the 674,825-to-1,738-byte
foreground graph reduction are real, but insufficient under this plan's
retention criterion. Prose embedding and language-switch validation were not
completed because the candidate was rejected before integration. A complete
experiment patch is preserved at `/tmp/tonk-lazy-editor-experiment.patch`.
Automatic approval review rejected directory-wide cleanup; cleanup succeeded
with the exact reviewed experimental paths after backing up the diff.

Step 3 started with UI-only `wasm-ui-size` (`inherits = "release"`,
`opt-level = "s"`) selected by `data-cargo-profile-release`. The installed
Trunk is 0.21.14; its binary contains that attribute and the official asset
reference documents it as release-only:
https://trunk-rs.github.io/trunk/guide/assets/index.html . Build logs must still
confirm selection. Worker, guest, native release, lockfiles and wasm-opt are
unchanged for this experiment. Build output target is
`/tmp/tonk-wasm-cold-start-ui-s`; no result yet.

UI `s` checkpoint: final UI Wasm 3,153,347 -> 2,273,197 bytes (27.9% smaller).
Build log confirms `wasm-ui-size` for UI, `release` for worker and the existing
`wasm-release` for guest. First timings 1,960 / 1,959 / 2,151 / 2,167 / 1,999 ms
(median 1,999, range 1,959–2,167). Repeat baseline 2,072 / 2,569 / 2,096 /
2,172 / 1,951 (median 2,096, range 1,951–2,569); UI s 2,368 / 1,992 / 2,057 /
2,204 / 1,964 (median 2,057, range 1,964–2,368). The 100-query/100-editor-update
guard measured baseline 63.8/50.7 ms and UI s 68.6/52.9 ms, with matching
response bytes and editor lengths. Provisionally retain for the measured
binary-size reduction, not a demonstrated local wall-time improvement.
Worker s is the next active variant. Inactive z/full-LTO profiles are declared
for subsequent isolated comparisons; remove any unused profiles at completion.

Worker s first batch: final worker Wasm 18,379,852 -> 8,915,027 bytes
(51.5% smaller). Build log confirms `wasm-worker-size` and unchanged UI s /
guest wasm-release. Paired UI-s-only baseline: 1,961 / 2,043 / 1,947 / 1,985 /
2,060 ms (median 1,985, range 1,947–2,060). Worker s: 1,818 / 2,236 / 1,843 /
1,802 / 1,821 ms (median 1,821, range 1,802–2,236). Query/editor guard:
67.3/50.4 ms baseline, 68.9/50.2 ms candidate, same response bytes and editor
lengths. Repeating startup because the outlier overlaps baseline variation.

Worker s repeat: baseline 1,936 / 2,986 / 1,980 / 2,043 / 1,976 ms (median
1,980, range 1,936–2,986); candidate 2,120 / 1,874 / 1,863 / 1,864 / 1,961 ms
(median 1,874, range 1,863–2,120). Retain worker s: median improvement repeated
(164 ms then 106 ms), substantial final-byte reduction, no observed guard
regression. UI z is next, independently changing UI s -> z with worker s fixed.

Analysis tooling checkpoint (not served): installed twiggy 0.8.0 under
`/tmp/tonk-wasm-tools`. The baseline guest has no retained Rust names. A
separate guest build with debug=1/strip=none completed, then matching bindgen
`--keep-debug` processing. Binaryen 129 crashed rewriting DWARF; `-Oz
--strip-dwarf --debuginfo` succeeded and preserves function names for twiggy.
Report `/tmp/wasm-guest-retained.txt` identifies large portal/delegation and
parser paths, with inspector functions among optional candidates. Its debug
artifact size is not a release-byte comparison; indirect-call-table dominance
also means retained counts must not be summed to claim per-crate savings.

UI z: final UI Wasm 2,069,533 bytes versus s 2,273,197 (another 9.0% smaller).
First timing 1,960 / 2,045 / 1,835 / 2,231 / 2,041 ms (median 2,041,
range 1,835–2,231), query/editor guard 70.5/50.0 ms. Paired repeat UI s:
2,096 / 1,832 / 2,676 / 2,028 / 2,140 (median 2,096, range 1,832–2,676);
UI z: 1,828 / 1,955 / 1,919 / 2,148 / 1,864 (median 1,919, range
1,828–2,148). Direction reversed relative to the prior s batch, so there is no
repeatable local timing difference. Provisionally prefer z for further byte
reduction with no observed guard regression. Next: worker z with UI z fixed.

Worker z: final Wasm 7,482,425 bytes versus s 8,915,027. First paired baseline
1,806 / 1,860 / 1,965 / 1,832 / 1,938 ms (median 1,860); z 2,045 / 1,928 /
2,022 / 2,029 / 1,948 (median 2,022). Query guard 70.5 -> 81.5 ms; editor
50.1 -> 50.1 ms. Repeat baseline 1,844 / 2,045 / 1,819 / 1,806 / 1,821
(median 1,821, range 1,806–2,045); z 2,141 / 1,926 / 2,049 / 1,913 / 2,465
(median 2,049, range 1,913–2,465). Query guard 68.9 -> 82.7 ms; editor
50.2 -> 50.4 ms. **Reject worker z**: startup and query slowdown both repeated.
Worker remains s. Next compare UI-only full LTO, inheriting UI z; worker s
and guest thin LTO remain unchanged.

Step 4 preparation-cache gate: temporary fetch instrumentation on the real
cold onboarding route saw one `Window.fetch(str)` payload fetch and one
`Window.fetch(str, init)` relay fetch in the trusted top document. Generated
bindgen glue and `bridge.rs` call sites confirm the latter is the nested
host's relayed request, not a second local payload preparation. The nested
realm is not instrumented by this top-target CDP preload, so no direct nested
count is claimed. **Reject a realm-local preparation cache for this cold-start
metric**: it would cold-miss in each preparing realm; cross-realm full-payload
reuse would require a broader protocol, not just memoization of this function.
No production cache was added; this is a topology-gate rejection, not a timed
cache implementation. Compiled-module transport gets its own minimal fixture
next; a failed transport boundary would reject it before bindgen integration.

Step 4 compiled-module decision: **reject at the transport boundary**. A real
Chrome opaque iframe (`sandbox="allow-scripts"`) received `messageerror` when
sent a minimal valid `WebAssembly.Module` by structured clone (no transfer
list). Controls returned the expected integer 7 for both same-origin module
transport and opaque-origin byte transport. Thus compilation and instantiation
work, but the required sealed-frame module transport does not. No bindgen or
Safari integration was attempted after this decisive Chrome prerequisite
failure; byte delivery remains unchanged. Together with the preparation-cache
topology gate above, step 4 retains no production change.

UI full LTO: final UI 1,854,682 bytes versus thin 2,069,533 (10.4% smaller).
First paired thin timing: 2,070 / 1,812 / 1,950 / 1,875 / 1,821 ms (median
1,875); full: 2,157 / 2,059 / 2,229 / 2,045 / 2,174 (median 2,157).
Query/editor guards thin 94.4/51.7 ms, full 72.2/53.1 ms. Repeat thin:
1,976 / 2,054 / 2,030 / 1,833 / 2,042 (median 2,030, range 1,833–2,054);
full: 2,133 / 1,954 / 2,229 / 1,988 / 1,844 (median 1,988, range 1,844–2,229).
The apparent startup regression did not repeat. Provisionally retain full LTO
for UI's additional byte reduction; do not claim a demonstrated timing gain.
Next is worker-only full LTO with worker s and UI z/full-LTO fixed.

Binaryen feasibility gate (step 6, no application integration): Binaryen 129
split a minimal module into a primary importing `placeholder.deferred.0` and
a secondary importing the primary's table. Synchronous secondary loading
returned 7. Returning a Promise from the placeholder returned **0 on the first
invocation**, then 7 after loading, in both Node and real Chrome. Chrome exposes
`WebAssembly.Suspending`; the existing synchronous bindgen contract does not
use JSPI or promise-wrapped exports. **Reject the drop-in wasm-split approach**:
asynchronous downloading is not transparent to its callers. A JSPI/glue and
reentrancy redesign would be a separate project, requiring compatibility
validation. No largest-module startup profile or application chunk manifest was
built after this prerequisite failed; no split is retained or claimed measured.

Step 6 explicit-module size gate: independent standalone guest builds used
`wasm-release`, matching bindgen, and `wasm-opt -Oz`. The paired local baseline
was 3,270,019 bytes (not substituted for the Nix timing baseline).

| Boundary | Initial guest | Optional module | Initial reduction | Combined growth |
| --- | ---: | ---: | ---: | ---: |
| Inspector + notebook | 3,065,830 | 1,333,995 | 204,189 | 1,129,806 |
| Board | 3,252,095 | 947,822 | 17,924 | 929,898 |
| Tree inspector | 3,183,180 | 268,670 | 86,839 | 181,831 |

Declared minimum linear memories (not measured resident memory): baseline
1,572,864 bytes; optional inspector/board each 1,310,720; optional tree
1,179,648. **Reject all three at the size/memory gate**: modest initial savings
come with duplicated code and another substantial private linear memory.
No loader was integrated, and no first-use latency or application timing win
is claimed. Source registrations were restored exactly and temporary bin
sources removed; paired outputs remain under `/tmp/tonk-guest-split`.

Top-document registration/dialog code was also inspected (`ui.rs` and
`register_dialog.rs`). Its synchronous open/pending/focus/stash callbacks own
singleton state and top-document ceremony sequencing. Reject an account split
in this bounded pass: it needs a new state/interface and ceremony-validation
project before any measured candidate exists. Workspace/display/portal/sigil
registrations underpin the current guest surface rather than a similarly
isolated optional feature. Together with the Binaryen contract rejection
above, no optional split or chunk-manifest change is retained.

Worker full LTO: 8,708,181 bytes versus thin 8,915,027 (2.3% smaller).
First paired thin: 1,915 / 2,296 / 1,850 / 2,047 / 1,854 ms (median 1,915);
full: 1,850 / 1,852 / 1,860 / 2,036 / 2,241 (median 1,860). Query/editor
guards 72.6/52.3 -> 70.3/52.2 ms. Repeat thin: 2,055 / 1,926 / 1,821 /
1,819 / 1,922 (median 1,922, range 1,819–2,055); full: 1,819 / 1,847 /
2,374 / 1,876 / 2,125 (median 1,876, range 1,819–2,374). Retain full LTO:
smaller bytes, both paired medians modestly lower, no observed guard regression.

Final compiler choice consolidated into two release-only profiles:
`wasm-ui-release` (z, fat LTO) and `wasm-worker-release` (s, fat LTO).
Inactive experimental profiles removed; native release, development, guest
profile, lockfiles and wasm-opt remain unchanged. Final clean-source artifact is verified below; the offline-fill investigation
is recorded separately below.

Clean-source validation checkpoint: temporary measurement source removed after
preserving its executable at `/tmp/tonk-wasm-measurement-runner` and its patch
at `/tmp/tonk-wasm-measurement-source.patch`. Only Cargo.toml, index.html and
this plan are modified. Repository `test:sw` passed all 127 tests;
`cargo fmt --all -- --check` and `git diff --check` passed. Final Nix build passed at `/tmp/tonk-wasm-cold-start-final`. No production tracing or benchmark
suite is retained.


Final release verification: build `0e76b12bc2ef9a28`; Nix logs confirm
`wasm-ui-release`, `wasm-worker-release`, and unchanged guest `wasm-release`.
Post-optimization UI is 1,854,682 bytes (baseline 3,153,347; 41.2% smaller),
worker 8,708,181 (18,379,852; 52.6% smaller), guest 3,279,122 (unchanged).
All 850 manifest asset SHA-256 values, manifest digest, worker digest, document
build meta tag and version/build identities were checked against final files.
Final paired baseline: 2,075 / 2,037 / 2,156 / 1,987 / 2,044 ms (median
2,044, range 1,987–2,156); final: 1,847 / 2,470 / 1,798 / 1,861 / 1,836
(median 1,847, range 1,798–2,470). The median is 197 ms / 9.6% lower, with
overlapping ranges; this is a local five-session comparison, not a tail-latency
or internet-download claim. Earlier final batches had medians 1,854 and 1,905.

Offline-fill investigation: five fresh final-artifact sessions observed first
verification 1,707–1,831 ms before usable content, last asset progress
1,299–1,419 ms before it, and complete adoption 472–503 ms afterward.
A temporary restamped artifact delaying only `ensureOfflineGeneration()`'s
fill by 2.5 seconds (incumbent installation unchanged) isolated contention:
normal median 1,854 vs delayed 1,778 ms; repeat normal 1,905 vs delayed
1,766 ms. Delaying all fill is diagnostic, not a retained scheduling policy.
The one-request-until-timeout spike is evaluated next; no production worker
change has been made.


Reject the one-request fill policy at the startup spike. A second temporary
restamped artifact starts one asset fetch loop immediately, holds the other
seven until a 2.5-second timeout, and passes this gate only from first-install
background fill; update installation retains eight workers. Its five-session
median was 1,835 ms (range 1,819–2,051), then 1,855 (1,772–2,064), against
normal final medians 1,847 and 1,827 (the latter range 1,801–2,369).
Unlike delaying all fill, the proposed one-request policy did not yield a
repeatable startup improvement. The timeout is later than every candidate's
usable-content measurement, so adding an acknowledgement at that boundary
cannot improve the already-measured foreground interval. No acknowledgement
protocol, held-response policy tests, or production scheduler was added after
this rejection; this is a timeout-path spike, not a fully integrated policy.
All existing verification and publication code remains unchanged.

The rejected one-request timeout spike still adopted complete offline
generations in all five sessions, 1,475–1,856 ms after usable content, versus
472–503 ms normally. It therefore worsened offline-ready latency without a
repeatable foreground benefit. This establishes eventual adoption for the
spike, not interruption/retry or acknowledgement correctness.

All six investigations are now decided. The retained production diff consists
only of two named Cargo profiles and their two release-only Trunk selectors.
Temporary implementation and test sources were removed. The final Nix release,
127 service-worker/JavaScript tests, formatting and whitespace checks passed;
final isolated Chrome startup and exact artifact identity checks passed.
Safari/device coverage, full Rust workspace lint/tests, CI and deployment were
not run for the final diff. Earlier experiments' narrower checks and failures
are recorded above and are not substitutes for these unrun checks.

Final runtime guard pairs (baseline -> final, ms): 100 sequential queries
64.3 -> 74.8, 70.9 -> 71.8, 67.9 -> 71.2; 100 editor value replacements
52.5 -> 53.1, 50.5 -> 50.4, 53.0 -> 54.1. Query response bytes remained
445,200 and editor final length 900. Query medians were 67.9 -> 71.8 ms
(5.7% slower, 3.9 ms total); editor medians 52.5 -> 53.1. Retain the
profiles for the much smaller artifacts and repeated startup improvement,
while explicitly accepting this small local query-guard cost. These narrow
HTTP/editor checks do not establish broad query/render throughput parity.
