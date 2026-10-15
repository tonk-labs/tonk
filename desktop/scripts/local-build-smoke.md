# Local worker build-loop smoke test

Use an isolated browser and a checkout containing the conditional evaluator.
This test creates a disposable local space; it never uses a signed-in Tonk
profile. The server has no access-service proxy and refuses network API routes.

Build from the Tonk checkout (the HTML path is relative to the Trunk config):

```sh
NO_COLOR=true nix develop --command trunk build \
  --config rust/tonk-ui/Trunk.toml index.html \
  --dist /tmp/tonk-conditional-runtime-dist
```

From Tonk, serve the built assets:

```sh
python3 scripts/serve-local-runtime.py /tmp/tonk-conditional-runtime-dist
```

Start an isolated headless browser:

```sh
CDP_SESSION_ID=tonk-direct-build
cdp() { chrome-devtools "$@" --sessionId "$CDP_SESSION_ID"; }
cdp start --isolated --headless --performanceCrux=false \
  --blockedUrlPattern='file:*' --chromeArg=--disable-crash-reporter
cdp status
cdp navigate_page --url http://127.0.0.1:4187/
cdp take_snapshot
```

Confirm `status` reports headless, isolated, no performance CrUX, blocked file
URLs and no unrestricted paths. Wait for service-worker control, then:

```sh
cdp evaluate_script "$(cat scripts/smoke-local-build.js)"
```

The first call creates a fresh local space, installs
`examples/direct-build.notation`, and returns its `nextURL`. Navigate to that
URL and take a snapshot. Confirm **Waiting for direct apply** is visible.
Run the same script again. It checks current data, previews an edit without
committing, conditionally applies it, rejects a stale write with HTTP 412, and
reads the committed value back. It deliberately refuses to replay a write after
an uncertain outcome; start a new fixture from `/` to repeat a test.

Without navigating or reloading, take another snapshot and screenshot. Confirm
**Applied locally without CLI or sync** replaced the initial status, and no
**STALE WRITE MUST NOT APPEAR** text is present. Inspect console errors and
network activity. Shut down the isolated browser and local server afterward:

```sh
cdp stop
```

Scope: real browser worker/storage and rendered DOM evidence. This does not
prove that a ChatGPT turn, native WKWebView, or external MCP client performed
the edit. Nor does it establish a generic revision-to-render completion protocol:
the script returns `renderingConfirmed: false` until the independent DOM check.

## Native local host

With the same static server running, build the signed app and launch:

```sh
bash scripts/build-app.sh
open '.build/Tonk.app' --args --local-runtime --seed-local-fixture
```

This uses separate local application and WebKit data and requires its own ChatGPT
sign-in. It creates a disposable `Robin Local Build Test` space. Omit
`--seed-local-fixture` on subsequent launches to reuse the existing space.
Open it, choose **Use for chat**, then ask Robin to query `direct-build-task`,
preview a status-only edit, apply using the exact preview revision, read it back,
and inspect the open preview. The local host does not advertise the CLI tool.
Boot, persistence across reopening, and a live ChatGPT-directed status edit have
been checked. The agent recovered from a query-only no-op, then previewed an
assertion, applied one claim, queried the saved value and inspected the new text
without reloading. Inspection still does not provide a render revision receipt.

## Completion-feedback runtime

The later local experiment in
`experiments/render-provenance/runtime-completion.md` adds scoped observations to
`tonk_inspect_view`. Use the combined runtime patch described there and serve its
built assets. Ask the agent to compare each complete observation's checkpoint
vector to the apply/query revision and to use only that observation's accompanying
textContent. Unsupported observations and frames must be reported separately;
global `renderedRevision` is still unavailable. Live status-only edits passed, including after a native restart on the final
build. Read-only cold-load checks remained unverified because initial checkpoints
were missing; both outcomes are recorded with the experiment evidence.

The follow-up cold-load fix and opt-in WKWebView regression are documented in
`experiments/render-provenance/cold-stream/README.md`. The retained batching build
passed first load, reloads, and edit-to-feedback checks in three fresh stores.
