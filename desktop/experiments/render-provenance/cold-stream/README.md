# Native cold-load stream reproduction

The installed completion experiment rendered saved text but sometimes lacked initial
query checkpoints. A fresh isolated WKWebView reproduced this without an account or
agent. A temporary decode-error trace ruled out silent checkpoint decoding failures;
that instrumentation was removed after the test.

The smaller fixture here has no Tonk code: its service worker emits a snapshot and
checkpoint, then leaves the response open. On this Mac, four trials of separate
chunks delivered both frames twice; a demand-driven highWaterMark=0 variant also
passed twice. Four trials of a single combined chunk all passed. See baseline.json.
A one-second fixture deadline records whether the idle tail arrived; the actual
Tonk reproduction waited 15 seconds and still had missing checkpoints. These are
observations on this WebKit build, not a diagnosis of WebKit internals or proof that
all browsers share the issue. An early transfer-only explanation was rejected when
repeated direct-fetch trials also stalled.

The retained fix batches up to 32 immediately-ready SSE events into each worker body
chunk. Each event retains its original framing and order. It does not wait for a
full batch, add a heartbeat, change queries, or promote incomplete evidence.
`../tonk-sse-batching.patch` is independent of checkpoint support and applies to the
main-based host-fix checkout. The combined runtime patch includes it as well; do not
apply both patches.

## Run

Serve the tiny fixture on its own port (not the Tonk runtime scope):

```sh
python3 -m http.server 4191 --bind 127.0.0.1 --directory experiments/render-provenance/cold-stream
TONK_STREAM_PROBE=1 TONK_COLD_LOAD_EVIDENCE=/tmp/stream-matrix.json swift test --filter ColdStreamDiagnosticTests.testInitialServiceWorkerFrames
```

For the actual Tonk test, build and serve the patched runtime on port 4187 using
`runtime-completion.md` and `scripts/serve-local-runtime.py`. Then:

```sh
TONK_LOCAL_RUNTIME_SMOKE=1 TONK_COLD_LOAD_EVIDENCE=/tmp/native-cold-completion.json swift test --filter ColdStreamDiagnosticTests.testLocalRuntimeColdCompletion
```

It creates a fresh WKWebView data store and a disposable local fixture, verifies
cold load and two reloads, then previews, conditionally applies, rejects a stale
write, reads back, and verifies updated inline content without refreshing. Every
complete observation must contain 14 checkpoints matching the saved query revision.
These tests skip in ordinary runs and never use the app's account or existing spaces.

## Retained verification

- Worker regression failed before the change: first body chunk contained only the
  snapshot. After the change, all 3 query-route tests passed, including immediate
  batch delivery, a later singleton checkpoint and clean EOF.
- Runtime build `c6f4686fc9e81247` passed three fresh-store native runs: 9 initial/
  reload checks and 3 conditional edits without reload. Every observation had 14
  checkpoints equal to the saved query revision. Each edit also verified preview
  isolation, stale-write 412, and readback. See `native-fixed.json`.
- Final focused native suite: 2 inspection tests passed, 2 opt-in tests skipped,
  0 failures. The 3 explicit opt-in integration runs are reported above.
- Temporary fetch/decode instrumentation was removed. The retained runtime change
  for this diagnosis is only query-response batching and its focused regression.
- This fixes the observed initial delivery gap locally. It does not establish a
  global render revision, certify unsupported templates/frames, or constitute a
  production deployment. No performance improvement was measured.

The signed Local app also passed after a full quit/relaunch, without an edit or
refresh: 14 task checkpoints matched saved edition 6. Profile chrome remained
unverified because `style` is outside the template allowlist; two frames were
unavailable. See `signed-app.json`. This does not certify the whole preview.
