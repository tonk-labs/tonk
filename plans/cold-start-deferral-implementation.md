# Welcome seed and image deferral

Requested scope: audit reassessment items 1 and 2. The live checkout is
409983c25; the audit's scoped component snapshot is absent here.

Implement a dependency-closed Welcome/shell snapshot plus one optional demo
shard. Keep sidebar metadata in Welcome. Start optional preparation after the
Welcome render, and await the same preparation on early demo navigation.
Store import markers atomically with seed assertions; a credential journal
separates Welcome readiness from full completion. Resume must not replay imported
facts over edits. Existing completed journals retain returning-Hub behavior.

Extract the two lossless WebP images to immutable binary assets and use deferred
host-mediated blob fetches. Import bytes only on demand or during optional
preparation, retaining branch portability and eventual offline availability.

Verification: partition/closure checks, focused worker tests for concurrent
creation, readiness, completion, interrupted resume and edits; browser first-use,
early demo navigation, image decoding, reload/offline checks; Rust formatting
and a release build. Record observed results and remaining limits below.

## Implemented result

The source snapshot was 2,995,227 bytes. Welcome now imports 906,489 bytes
(69.7% less), 1,217 artifacts and seven modules. One 1,041,149-byte optional
shard contains 1,902 artifacts, the other 19 modules and the original two blobs.
Both extracted images are byte-identical WebP files (468,634 and 321,186 bytes),
with original dimensions 1024x604 and 1024x643. Only the Welcome view and Vault
active module changed among the original 3,115 facts; four blob metadata facts
were added. Regeneration is deterministic. Maintenance instructions are in
`scripts/onboarding/README.md`.

The loader requests the optional shard after real Welcome content appears and
two animation frames pass. Earlier navigation awaits that same request. Failed
preparation offers a retry. A removed space or mismatched profile cannot trigger
late import. Each import marker shares its facts' commit; both retries and the
first optional import preserve existing values. The original completed-journal
and returning-Hub behavior remains intact.

Images are absent from the initial JSON and view response. IntersectionObserver
fetches branch blob bytes through the existing host relay only when an image
intersects the viewport. Optional preparation also imports them for eventual
sync/export/offline use. Independent service-worker offline preparation can
still fetch static assets in the background. The worker's own deferred asset
reader uses its sealed generation, including after an interrupted offline
resume, rather than bypassing the cache or mixing deployment versions.

The route inventory's commands-not-routes guidance was checked. The new branch
`/onboarding` POST is a fixed bundled-import continuation, covered by its raw
data-plane exception, and is documented in the pinned route table.

## Verification (2026-09-10)

- Focused onboarding tests: 6 passed. Includes concurrent first use/completion,
  Welcome-before-demos, edits before first optional import, interrupted journal
  saves, removal, and image hash rejection/readback.
- Worker router checkpoint: 110 passed; its only failure was the missing new
  route in the pinned inventory. After correcting the inventory, both route
  table tests passed. No broader rerun was needed for this test-only inventory.
- Service-worker lifecycle suite: 128 passed, including sealed-library reads
  offline and rejection of invalid paths.
- Extended browser regression: passed on the packaged dev artifact and again
  on the release artifact. It holds optional preparation, checks Welcome and
  no pre-scroll blob requests, decodes both images by scrolling, selects Agent
  playground before releasing preparation, checks its copy prompt, returns to
  Hub, waits for offline generation adoption, reloads offline, decodes both
  images, and renders all eight navigable bundled pages.
- The all-pages fixture initially tried a ninth legacy Heading presets record
  that lacks the node order needed to appear in the sidebar. It now uses the
  same known-node set as the application; the snapshot was not changed for this.
- Rust formatting, JS syntax, partition conservation, deterministic generation,
  image identity checks and `git diff --check` passed.

Release runtime build: `/tmp/tonk-deferral-verified` (Nix store output
`ydikhq7cbz5685y5c6ypf2faznywsp5k-tonk-ui-trunk-0.6.14`). The subsequent Trunk
copy declarations were verified in the rebuilt dev output. For release browser
verification the same four newly declared assets were added to a disposable
copy and restamped by the repository script: `/tmp/tonk-deferral-release`, build
`8415ca6c93a55e7f`. All 854 asset digests match; all onboarding files match the
worktree; the built onboarding production Rust matches the final worktree.
The Nix release build was not repeated solely for the final copy declarations
and native-test-only edits. The runtime and final assembled asset set were
verified together in the release browser test.

Resolved failures: initial Nix/browser sandbox access; two Rust API/fixture type
mismatches; a nondeterministic schema selection in the splitter; a missing Trunk
asset copy list (correctly rejected as HTTP 503 by the generation verifier);
the route inventory; the unreachable legacy page in the browser fixture; and a
final test-only formatting adjustment. One superseded intermediate Nix build
was stopped. Nix emitted a cache.flakehub.com 401 warning but built successfully.

A temporary pre-change dev observation measured a 1,139.5 ms Welcome request,
with guest requests beginning at 1,326.5 and 1,615.5 ms after navigation. That is
diagnostic evidence only. No paired latency comparison was run and no runtime
speedup is claimed. Safari/device, remote/CDN behavior, CI, deployment, and a
complete new-process offline browser restart were not tested. No dependencies,
lockfiles, commits, user browser storage or unrelated plans were changed.
