# tonk-portal

The `<tonk-portal>` custom element: imperative, author-supplied HTML rendered inside an isolated iframe.

The declarative `<tonk-view>` / `<tonk-display>` stack paints a template by interpolating an entity's fields into the page DOM, which cannot express arbitrary imperative work (canvas/WebGL drawing, third-party widgets, custom state machines). `<tonk-portal>` is the escape hatch for that: it writes an author-supplied HTML document (which may run its own scripts) into a sandboxed iframe. It backs view types like `text/html`.

Like `<tonk-view>`, the portal is a painter, not a fetcher: it opens no subscription and resolves no descriptor of its own. It receives an already-fetched HTML string through its `content` attribute and does one imperative thing, assigning the iframe's `srcdoc`. The `content` is itself first-class dialog data: the `portal` concept holds it, and a nested `<tonk-display model=portal>` fetches it. The `portal` concept and its canonical view live in the standard library (`tonk-core/assets/library/core.yaml`), seeded at repository creation.

This crate compiles to wasm and registers the element via [`register`](src/lib.rs).

## Providing content

The element observes three attributes: `content`, `entity`, and `model`.

- `content` is the HTML document body. On connect, the portal creates a single child `<iframe>`, prepends a small bridge bootstrap script to the content (see below), and assigns the result as the iframe's `srcdoc`.
- A `content` change reassigns `srcdoc` on the **same** iframe (the element is not torn down and rebuilt) after cancelling any live subscriptions the discarded window had opened.
- `entity` and `model` scope the portal. They are handed to the iframe as `context` (`{ this, model }`) and a change re-scopes by reloading the iframe so the bootstrap re-runs author code under the new context.

The iframe always fills its container (`width`/`height` 100%, `border` 0). On disconnect the iframe is detached and its subscriptions cancelled.

## Isolation and sandbox model

The iframe is sandboxed with `sandbox="allow-scripts"` and **no** `allow-same-origin`, so it loads at an opaque (null) origin. Scripts run, but author code cannot reach `parent.document` or any other page DOM. Content is delivered via `srcdoc`, never by a fetched URL or by reaching into the iframe document from the parent.

The opaque origin is the isolation boundary: the iframe talks to the parent only over a `MessageChannel`. The bootstrap script (in [`bridge`](src/bridge.rs)) defines `window.tonk` synchronously, opens a channel, and posts a `hello` to the parent transferring one port. Because an opaque origin must post to `"*"` and reports its origin as `"null"`, the parent authenticates the handshake by `event.source` identity (matching the message against a registered iframe's live `contentWindow`), never by `event.origin`.

## The live-data bridge

Author code in the iframe sees one injected object:

```text
window.tonk = {
  context: { this, model },
  query(body?)      -> Promise<Conclusion[]>,
  subscribe(body?)  -> ReadableStream<Conclusion[]>,
  transact(request) -> Promise<receipt>,
  ready: Promise<void>,
}
```

The parent is a pure port relay. After the handshake it binds the transferred port and posts `ready { context }` back, then translates each inbound envelope into the existing `tonk-query` / `tonk-subscribe` / `tonk-claim` consumer events on the `<tonk-portal>` element, which bubble to the installed host on the document. Subscription frames arrive back through the portal's `reset` / `error` methods (the same seam `<tonk-display>` uses) and are posted to the iframe as `subscribe-event` / `subscribe-error`. A `query()` / `subscribe()` call with no argument builds the scoped-entity query from the model descriptor and `entity` (see [`query`](src/query.rs)), matching what `<tonk-display>` would read.

## Modules

- [`element`](src/element.rs): the `<tonk-portal>` custom element: lifecycle, iframe ownership, `srcdoc` painting, and the `reset` / `error` prototype shims.
- [`bridge`](src/bridge.rs): the iframe bootstrap, the page-level `hello` listener and registry, port binding, and the envelope dispatcher relaying to host consumer events.
- [`query`](src/query.rs): wire-query construction for no-argument bridge calls.

## Audio and video recording

`window.tonk.recordAudio({maxDurationSeconds, onLevel, signal})` and
`window.tonk.recordVideo({maxDurationSeconds, audio, onLevel, signal})` request a
recording from the trusted host page. Video includes microphone audio by default;
pass `audio: false` for camera only. Each call resolves after host consent and
browser device permission with `{result, stop(), cancel()}`. `result` resolves to
an audio or video Blob, with the MIME type selected from browser-supported formats.

```js
const recording = await window.tonk.recordVideo({maxDurationSeconds: 10});
const blob = await recording.result;
const url = URL.createObjectURL(blob); // revoke when playback is finished
video.src = url;
```

`stop()` finishes and returns the recording; `cancel()` or an AbortSignal rejects
with `AbortError`. `onLevel({elapsed, waveform})` receives 128 byte waveform samples
approximately every 50 ms when audio is enabled. Duration is clamped to 1–30 seconds.
Camera capture is capped at 1280×720 and 30 fps, with a requested video bitrate of
2.5 Mbps. Actual dimensions, bitrate, and MIME type depend on the browser/device.

The host owns MediaRecorder and all device tracks. Nested guests forward requests
through their existing authenticated bridge ports. Only metering and the finished
Blob cross into space content; streams and device handles stay in the host.
Each recording requires a host-owned consent button identifying the requested
devices, and a visible host stop control remains available.

The existing `mic-start`, `mic-started`, `mic-level`, `mic-stop`, `mic-cancel`,
`mic-result`, and `mic-error` envelopes are scoped to the originating port and
request ID. `mic-start` adds a `video` boolean and optional `audio: false` for
silent video; legacy requests remain audio-only. There is at most one capture
per page, shared across both APIs. Cancellation, errors, maximum duration,
portal teardown, and handshake replacement release the tracks. The iframe
sandbox and Permissions Policy are unchanged.

Browser regression: `tests/microphone-browser.cjs` exercises the real bootstrap
and media module through two opaque-origin frames with Chrome's synthetic camera
and microphone. It checks audio metering, decodable video with and without audio,
automatic stopping, consent rejection, cancellation, concurrent-request rejection,
pre-aborted signals, denied device permission, the host Stop button, and teardown. Run with Playwright available through `NODE_PATH`
or `TONK_PLAYWRIGHT_PATH`; optionally set `TONK_CHROME_PATH`. The fixture intercepts
its localhost URL and does not need a running dev server or real media devices.
