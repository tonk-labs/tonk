# Runtime completion feedback — local integration

The experimental runtime now exposes `tonkDisplay.inspectCompletion()` in its
ordinary, non-test build. Tonk forwards these observations through
`tonk_inspect_view`. The runtime method has no native-host dependency.

## Contract

A synchronous observation walks the root display and its light-DOM descendants.
For every display it requires current subscription setup, all required model,
view, entity and (when applicable) bookmark checkpoints, and a mounted view that
finished applying the same folded entity frame. It rejects retained reconnect
content, changed routing context, disconnected/restarted displays, held history,
fallback/carousel/self rendering, portals, custom components, host-attribute
bindings and event-bearing templates. Supported templates are a restricted set
of static HTML elements plus nested displays.

The method returns its accompanying `textContent` in the same browser task as
these checks. Unsupported observations return a reason (including missing input names) and no subtree text;
the existing native DOM snapshot remains separate. Text is bounded to 6,000
characters and labelled as textContent, not visibility/screenshot evidence.

`complete` means these synchronous renderer obligations are satisfied. It does
not imply a common revision across queries, frames or branches, visual correctness,
interaction success, or absence of subsequent mutations. Callers must compare
each required checkpoint with the desired revision. Global `renderedRevision`
remains null. Checkpoint vectors and page data are untrusted observations, not an
authorization mechanism or cryptographic proof of DOM state.

The implementation retains the last applied frame per view to compare against
the display's expected frame. Its memory overhead has not been benchmarked.

## Reproduce

`tonk-runtime-completion.patch` is a **combined alternative** to the older Tonk
transport, nested-browser and subscription-attempt patches. Do not apply both.
It applies to Tonk `7ff32b38c32d08c4fc00e4dd0757df4dfd7bee0a` and includes the
transport and race correction as prerequisites. The independent race correction
is also draft [Tonk PR #1057](https://github.com/tonk-labs/tonk/pull/1057), based on
current main `474f03c8c` (which already includes conditional evaluation).

Apply `dialog-query-checkpoint.patch` to dialog-db
`120fba8b6490c31b2ae1523e6add5b800f9b989e`. Generate Cargo path overrides with
`cargo-overrides.py` using canonical absolute paths. Keep a separate target
folder for each workspace. For Trunk's nested Cargo invocations, append the
local override table to the scratch checkout's `.cargo/config.toml`; do not
commit that machine-specific configuration or the generated Cargo.lock.

From the patched Tonk checkout and its Nix development shell:

```sh
cargo test -p tonk-display --lib --target wasm32-unknown-unknown
cargo test -p tonk-display --test render_completion_fullstack --target wasm32-unknown-unknown
trunk build --config rust/tonk-ui/Trunk.toml index.html --dist /tmp/tonk-completion-runtime-dist
```

From Tonk, serve `/tmp/tonk-completion-runtime-dist` with
`scripts/serve-local-runtime.py`, build using `scripts/build-app.sh`, and launch
with `--local-runtime`. This uses the existing isolated Local profile. No
production or staging data, dependency pins, or remote deployment were changed.

## Evidence

- Standalone host correction: 61 browser tests; signed commit
  `72b007a0db7062e39ab317fc2c0070dbc0d86426`; verified draft PR head.
- Renderer suite: 259 passed. It initially exposed two order-dependent registry
  fixtures that treated `tonk-display` as passive markup. Both passed alone;
  the unmodified display suite passed 256 tests. The fixtures now explicitly
  register the real wrapper and retain their authored state-slot children.
  The full 259-test suite then passed.
- Real-worker integration: conditional preview/apply, nested rendered text,
  checkpoint-vector equality with the applied revision, saved-record readback,
  and navigation while a second conditional edit is in flight all pass. The
  worker router and IndexedDB run in the browser test page, not a service worker.
- The first native reload check exposed missing completion evidence. A focused
  regression reproduced that one stream error cleared every input checkpoint;
  healthy streams deduplicate unchanged checkpoints, so recovery of one input
  could leave the display permanently unverified. Error invalidation now affects
  only the named input (untagged errors still invalidate all). The full 259-test
  display suite and real-worker integration passed again. See
  `reconnect-checkpoint-baseline.txt`.
- Native inspection tests: 2 passed; signed native app build and signature check
  passed. Ordinary hidden/password input exclusion remains covered.
- Actual service-worker smoke: preview did not commit, conditional apply/readback
  succeeded, stale apply returned 412, and the new text appeared without reload.
  Console errors were the expected stale-write 412 and the development hot-swap
  WebSocket endpoint missing from the static server. The server has no API/sync
  backend; worker keepalive requests are not proof of remote synchronization.
- Live Robin turn in the existing disposable `Robin Local Build Test` used the
  direct schema/query/preview/apply/inspect tools, accepted a status-only edit,
  and observed the result without reload. Independent parsing of its actual tool
  outputs confirms the query revision and 14 required checkpoints across four
  nested displays match the applied edition-5 revision. See
  `native-completion-smoke.json` for the compact evidence.
- Runtime build `988b423d4bfbde3e` was used for that first mutation test. Final
  installed build `96eab1acc4d180d6` includes per-input error invalidation and a
  bounded 24-event callback trace. Final hook tests (28) and real-worker test (1)
  passed. After a clean native restart, the agent's edition-6 status-only edit
  read back correctly and produced a complete task observation with 14 matching
  checkpoints without refresh. Full-revision comparison code was reviewed;
  `native-completion-final.json` retains its Boolean results, not raw signatures.
- Native cold-load evidence remains incomplete: initial view/entity checkpoints
  sometimes never reach the display callback, despite correct saved text. This
  also occurred after clean restart, without preceding error callbacks, so the
  per-input invalidation fix does not explain or solve it. A subsequent edit
  produced complete task evidence. Profile chrome remained unverified and two
  frames unavailable. This was the open issue at that checkpoint. The later
  [cold-load experiment](cold-stream/README.md) reproduces a service-worker chunk
  delivery stall and verifies ready-frame batching in build `c6f4686fc9e81247`.
  Universal completion is still not claimed.
- Combined patch passed forward application against its exact base using a
  temporary index, reverse application against the working experiment, and
  source whitespace checks. Local override configuration and lockfile are excluded.

This is a locally installed experiment. It does not yet certify arbitrary Tonk
interfaces, iframe boundaries, commands or custom-element work, and is not a
production release or a measured end-to-end speedup.
