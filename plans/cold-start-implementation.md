# Cold-start audit items 1 and 2 — implemented and measured

Completed 2026-09-10, against source `d757e48ac0849de099a98f47e54e5f7b8ae15525`.
Only Welcome payload compression and single-pass library seeding were changed.
The original audit and pre-existing untracked followup plan are preserved.

## Result

The direct original-versus-final comparison improved median first-visit startup
from **2,132.5 ms to 2,003 ms: 129.5 ms / 6.1% faster**. Both paired batches
improved (2,297 → 2,038 ms and 2,039 → 1,872 ms). Individual samples overlap:
original 1,933–2,804 ms, final 1,856–2,432 ms. This is a modest local Chrome gain,
not a universal latency estimate. The individual experiments below have noisy,
different baselines; their savings must not be added.

The imported snapshot is **1,431,538 bytes / 32.3% smaller**. The three startup
libraries now need **three complete evaluations instead of six** when there is
no head race. Neither reduction implies the same percentage startup speedup.

## Implementation and asset verification

`rust/tonk-core/assets/library/onboarding.yaml`:

| Asset | Original bytes | Final bytes | Verification |
| --- | ---: | ---: | --- |
| Charter normal | 502,016 | 147,480 | WOFF2; all 2,092 glyphs retained |
| Charter italic | 473,788 | 163,076 | WOFF2; all 2,088 glyphs retained |
| Forum image, 1024×604 | 728,578 | 468,634 | Lossless WebP; identical RGBA pixels |
| Build image, 1024×643 | 469,645 | 321,186 | Lossless WebP; identical RGBA pixels |
| Entire snapshot | 4,426,765 | 2,995,227 | Exactly two artifact values changed |

FontTools 4.63.0 converted the complete faces without subsetting. Glyph ordering,
cmaps and decompiled font tables compare identically after accounting for the
WOFF2 compression flag and checksum adjustment in `head`. Font names, weights,
metrics, layout tables and loading code are preserved.

Images use libwebp 1.6.0, `cwebp -lossless -exact -m 6`. Pillow verified identical
dimensions and RGBA pixel buffers. Its optimized PNG outputs were larger than
the originals (828,571 and 540,095 bytes), so those were not retained.

The changed values are the page-fonts module and Welcome view. All 3,115
artifacts, both blob contents, CSS, DOM and lazy-loading behavior are preserved.
Temporary conversion and verification source: `/tmp/tonk-audit-convert.py`.

`rust/tonk-worker/src/router/evaluate.rs` and `repository.rs`:

Library seeds use `seed_on_branch`, taking the branch writer lock before their
first evaluation. Interactive requests retain their lock-free query/dry-run
path. Both paths share the existing bounded CAS refresh/re-evaluation loop,
commit and subscription polling. A seed with no mutation statements is rejected.
Existing timing logs now include whole-operation elapsed time and evaluation
pass count, including the speculative pass where applicable.

Regression tests compare all three real libraries' resulting facts and installed
rule structures. Signed revision metadata is excluded; generated anonymous
variables are alpha-normalized while preserving their references and all rule
structure. Tests also cover rule execution, concurrent seeds, a deliberately
injected head advance requiring refresh/re-evaluation, and interactive previews
while the writer lock is held. Native logs show 1 versus 2 passes for each real
library; every measured final-browser startup reports 1 pass for each of the
60-, 86- and 4-expression seed documents.

## Measurement method and artifacts

Each experiment uses A5/B5/A5/B5: two alternating batches per artifact, five fresh
disposable Chrome profiles per batch, 20 visits per experiment, 80 visits total.
Chrome 152.0.7977.83, 1200×900 viewport, local HTTPS Caddy, no compression or
throttling. No build or other browser test ran during these measurements.

The timer starts on `about:blank` immediately before the first navigation. It
stops at displayed nested Welcome content with its authored fonts loaded and
any on-screen images decoded. Both iframe ancestors must be displayed. The two
below-fold images retain `loading="lazy"`; each is scrolled into view and decoded
at its original dimensions **after timing**. This is visible initial content,
not a fully-painted whole-page endpoint. No existing user browser data was used.

Every visit verifies the document and controlling-worker build identities after
timing, and waits for complete offline-generation adoption before quitting.
All 80 visits passed these checks. The temporary runner and source remain under
`/tmp/tonk-audit-measurement-*`; no benchmark infrastructure remains in source.

| Artifact | Build identity | Description |
| --- | --- | --- |
| `/tmp/tonk-audit-baseline` | `0e76b12bc2ef9a28` | Unchanged release baseline |
| `/tmp/tonk-audit-fonts` | `43eb76719767e710` | Baseline plus WOFF2 only |
| `/tmp/tonk-audit-assets` | `80db245e704b5c31` | Baseline plus WOFF2 and WebP |
| `/tmp/tonk-audit-candidate` | `dfcfe11b4a7a3dc8` | Release build with both requested items |

The two asset experiments copy the exact baseline, replace only the snapshot,
and run the repository's `stamp-service-worker.sh`. Their Wasm is byte-identical
to baseline. All manifest asset digests, manifest digests and worker Wasm stamps
were verified for all four artifacts. Final UI and guest Wasm differ only in
embedded Nix build-directory paths; worker Wasm grows 8,708,181 → 8,709,963 bytes.

## Paired measurements

Medians below combine ten visits per side within each experiment.

| Experiment | Before median | After median | Difference |
| --- | ---: | ---: | ---: |
| WOFF2 only | 2,041.5 ms | 1,885.0 ms | 7.7% faster |
| WebP added to WOFF2 | 2,054.5 ms | 1,871.0 ms | 8.9% faster |
| Single-pass seeds added to compressed assets | 1,945.0 ms | 1,824.5 ms | 6.2% faster |
| Original → both items | 2,132.5 ms | 2,003.0 ms | 6.1% faster |

Full samples, in execution order:

| Experiment / batch | Samples (ms) | Median (ms) |
| --- | --- | ---: |
| fonts A1 | 2475, 2734, 2282, 2105, 1974 | 2282 |
| fonts B1 | 1978, 1891, 2337, 1932, 1866 | 1932 |
| fonts A2 | 2049, 1930, 2034, 1937, 1952 | 1952 |
| fonts B2 | 2045, 1877, 1869, 1879, 1847 | 1877 |
| images A1 | 1857, 1937, 1883, 2052, 2153 | 1937 |
| images B1 | 1860, 1843, 2153, 2060, 1882 | 1882 |
| images A2 | 2258, 2134, 2200, 2057, 2043 | 2134 |
| images B2 | 1852, 1836, 1965, 1851, 2276 | 1852 |
| seeding A1 | 2541, 1942, 1945, 3515, 2396 | 2396 |
| seeding B1 | 2070, 1938, 1824, 1811, 1804 | 1824 |
| seeding A2 | 1854, 2059, 1931, 1945, 1904 | 1931 |
| seeding B2 | 1825, 1972, 1977, 1816, 1823 | 1825 |
| combined A1 | 2804, 2097, 2297, 2168, 2309 | 2297 |
| combined B1 | 2308, 1966, 1968, 2038, 2195 | 2038 |
| combined A2 | 2265, 2039, 2066, 1933, 2035 | 2039 |
| combined B2 | 1856, 2107, 1872, 1864, 2432 | 1872 |

Logs: `/tmp/tonk-audit-{fonts,images,seeding,combined}-paired.log`.
The initial timed attempt incorrectly awaited below-fold lazy images and hit the
script timeout. It produced no samples and is excluded. The corrected endpoint
and post-timing image checks are used consistently throughout the table above.

Final-browser seed totals (including parse, evaluation, matches, commit and
polling) had medians of 167 ms in the seeding comparison and 172.5 ms in the
combined comparison. The old browser log omits its discarded evaluation, so
these totals are not presented as a before/after seed-phase speedup.

## Validation and limits

- Final native router integration: **109 passed**, including the seven evaluator
  regressions and four onboarding tests.
- Chrome service-worker evaluator regressions: **6 passed**.
- Service-worker lifecycle suite (`test:sw`): **127 passed**.
- Final release first-use/Agent-playground/returning-Hub browser regression:
  **1 passed**.
- All **80** fresh-browser measurement visits passed their functional checks.
- Rust formatting and `git diff --check` passed.
- Release build and all artifact digest checks passed. A fresh build of the
  cleaned checkout (`/tmp/tonk-audit-final`) resolves to the identical Nix store
  output as the measured candidate.

The initial native test compile failures were fixture mistakes (owned key and
constrained selector), and the initial rule comparison needed identity
normalization. The first release build caught an incorrect test-only gate on
the interactive bridge wrapper; Wasm availability was restored before the final
successful build and measurements. Temporary artifact restamping initially hit
read-only copied directory permissions; only the disposable copies were made
writable. These resolved failures are not omitted from the verification record.

Unrun: Safari/device, real-network/CDN performance, offline reload after adoption,
broader workspace tests/lint, CI and deployment. Adoption checks establish full
generation preparation, not offline-reload coverage. No commit or deployment was
made, and dependencies, lockfiles and compiler profiles are unchanged.
