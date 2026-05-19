# `<tonk-portal>` — sandboxed artifact rendering

## Context

`<tonk-display>` renders a single entity by querying a `view` row
on the branch and interpolating its `display` HTML against the
entity's fields. The HTML is authored by trusted curators of the
branch and ends up running as a child of the host page: same
origin, same SW, same DOM.

`<tonk-portal>` is the analogue for untrusted HTML — artifacts
contributed by anyone on the branch, served whole-document, that
a host page embeds without granting them access to the
surrounding page, the network, or anything beyond a defined data
API.

The element renders the artifact inside a nested iframe pair:

- An outer iframe with `sandbox="allow-scripts"` (unique opaque
  origin, no network, no SW reach) holds a fixed bootstrap.
- The bootstrap creates an inner iframe with `srcdoc` set to the
  artifact's HTML verbatim. The inner inherits the outer's
  opaque origin (a property of `srcdoc`), so the two are
  same-origin to each other and the artifact's scripts can call
  `parent.tonk` directly.
- The outer brokers data flow to the top frame via a
  `MessageChannel` it constructs and posts up. The top frame
  performs SW-backed queries on the artifact's behalf.

Target usage:

```html
<tonk-portal source="did:key:zArtifact…" />
```

## The artifact's content claim

`<tonk-portal>` resolves the document by subscribing to a single
claim:

```
(the = xyz.tonk.artifact/content, of = <source>)
```

No concept descriptor, no `artifact!` concept on the branch — the
single claim is enough. Authors assert an artifact by writing the
claim directly:

```yaml
artifact!: &welcome
  xyz.tonk.artifact/content: !text/html |
    <!doctype html>
    <html>
      <head><title>Welcome</title></head>
      <body>
        <h1>Welcome</h1>
        <button id="ask">Show entity count</button>
        <output id="out"></output>
        <script type="module">
          const rows = await parent.tonk.query({
            terms: { this: { "?": { name: "thing" } } },
            predicate: { with: {} }
          });
          document.getElementById('out').textContent = rows.length + " things";
        </script>
      </body>
    </html>
```

(The exact notation form is decided in the worker / notation
layer; the design only depends on the claim shape:
`(the = xyz.tonk.artifact/content, of = <entity>, is = <html-string>)`.)

The artifact author writes a full HTML document, including
`<!doctype>`, `<head>`, and `<body>`. The portal does not wrap or
modify it. The artifact's scripts reach the data API via
`parent.tonk`.

## The `source` attribute

`source` is an entity URI — specifically a DID. The element
subscribes directly to
`(the = xyz.tonk.artifact/content, of = <source>)` and uses the
resulting `is` value as the inner iframe's `srcdoc`. No Phase-1
concept resolution, no name lookup.

Rejected at validation:

- empty string,
- any value that doesn't parse as a DID.

A `data-state="error"` reflects the failure; lifecycle event
`tonk-portal:error` carries `{ kind: 'descriptor', message }`.

Live subscription: editing the artifact's `content` claim on the
branch updates the inner iframe (see "Content-change strategy"
below).

Name-based lookup (resolving a human-readable name to an artifact
entity) is out of scope here. Callers that have only a name
resolve it themselves — via a `Name` query or any other lookup —
and pass the resulting DID to `<tonk-portal>`. This keeps the
element's resolution path single-source and its failure modes
narrow.

## Element shape

```html
<tonk-portal
    source="<uri-or-name>"
    [space="<space>"]
    [branch="<branch>"]>
</tonk-portal>
```

Attributes (all observed; changing any restarts flows):

| Attribute | Required | Meaning |
|---|---|---|
| `source` | yes | DID of the artifact entity. Must parse as a DID; anything else is an error. |
| `space` | no | Defaults to `"home"`. |
| `branch` | no | Defaults to `"main"`. |

No children — the portal owns the outer iframe in its light DOM
(no shadow root; mirrors `<tonk-display>`'s posture).

## DOM state signalling

Same convention as `<tonk-display>` — `data-state` on the host
reflects lifecycle for stylesheets:

| State | Reflected as |
|---|---|
| Initial / resolving | `<tonk-portal data-state="loading">` |
| Inner iframe loaded with artifact | `<tonk-portal data-state="ready">` |
| Artifact not found / empty stream | `<tonk-portal data-state="empty">` |
| Concept / network / parse failure | `<tonk-portal data-state="error">` |

Error detail is still dispatched as a custom event.

## Frame topology

Three frames, two trust boundaries:

```
┌────────────────────────────────────────────────────────────────┐
│ Top frame — host page (real origin H, has SW)                  │
│                                                                │
│  <tonk-portal source="...">                                    │
│    └─ <iframe sandbox="allow-scripts" srcdoc=BOOTSTRAP_HTML>   │
│         ┌──────────────────────────────────────────────────┐   │
│         │ Outer frame — opaque origin O₁                   │   │
│         │ Fixed bootstrap document (constant per portal).  │   │
│         │                                                  │   │
│         │  1. new MessageChannel(); keep port1.            │   │
│         │  2. parent.postMessage('tonk:port', '*',         │   │
│         │                        [port2]).                 │   │
│         │  3. Define window.tonk in terms of port1.        │   │
│         │  4. Create the inner iframe:                     │   │
│         │     <iframe srcdoc=ARTIFACT_HTML> (no sandbox    │   │
│         │     attribute → inherits flags; srcdoc → shares  │   │
│         │     opaque origin O₁ with the outer).            │   │
│         │                                                  │   │
│         │  ┌────────────────────────────────────────────┐  │   │
│         │  │ Inner frame — same opaque origin O₁        │  │   │
│         │  │ srcdoc = artifact content, verbatim,       │  │   │
│         │  │ full HTML document. No wrapper injection.  │  │   │
│         │  │                                            │  │   │
│         │  │ Scripts call parent.tonk synchronously     │  │   │
│         │  │ (same-origin to outer).                    │  │   │
│         │  └────────────────────────────────────────────┘  │   │
│         └──────────────────────────────────────────────────┘   │
└────────────────────────────────────────────────────────────────┘
```

### Origin behaviour

- Outer iframe has `sandbox="allow-scripts"` without
  `allow-same-origin`. It gets a unique opaque origin O₁,
  cross-origin to the top.
- The inner iframe has no `sandbox` attribute. Per the HTML spec,
  nested browsing contexts inherit their parent's sandbox flags;
  the inner is sandboxed even without the attribute.
- The inner uses `srcdoc`. `srcdoc` documents inherit their
  parent's origin. The outer's origin is opaque O₁; the inner
  inherits the same O₁, so outer and inner share an OriginID and
  pass same-origin checks against each other.
- Top ↔ outer: cross-origin (H vs O₁). Top ↔ inner: cross-origin
  (H vs O₁). Outer ↔ inner: same-origin (O₁ vs O₁).

### Why nest at all (vs. one sandboxed iframe with a wrapper)

The nested topology, compared to one sandboxed iframe with an
injected `<script>` wrapper:

- The artifact HTML is unmodified. The author writes a full
  `<!doctype html>...</html>` document with no injection into
  their `<head>` or templating around their `<body>`.
- `parent.tonk` is synchronously available. The outer publishes
  `window.tonk` before creating the inner, so inner scripts can
  call `parent.tonk.query(...)` at top level without waiting on
  a ready signal.

### Sandbox rationale

- No network from either iframe. Opaque origin means no cookies,
  no storage, and no SW interception (the SW is scoped to the
  parent origin H, not O₁). `<script src>` and `fetch()` don't
  reach anything readable.
- No DOM access to the top. `window.top.document` throws; only
  `postMessage` crosses that boundary.
- No top navigation, popups, forms, or storage. Only
  `allow-scripts` is granted on the outer.

### Not used here

The existing `/api/repository/{repo}/branch/{branch}/host/{host}/{entity}`
route in `tonk-worker/src/router/host.rs`, with its URL-bound
guest registration and SW-rewritten subresource paths, is for the
display viewer (`render_viewer_view` in `tonk-ui`). Portals don't
navigate to that URL and don't register a guest binding.

## The bootstrap

The outer iframe's `srcdoc` is a fixed HTML document, identical
for every portal instance. Lives as `&'static str` in the crate
(`include_str!("bootstrap.html")`). Roughly:

```html
<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <script>
    // Establishes the bridge to the top frame and exposes
    // window.tonk. Inlined verbatim; see "Bridge handshake"
    // and "Bootstrap behaviour" below.
  </script>
</head>
<body></body>
</html>
```

The artifact's HTML is *not* in this document. It goes into the
inner iframe, which the bootstrap creates programmatically once
the bridge is up.

## Bridge handshake

The outer frame's bootstrap creates the channel and posts it up
— the top frame is a passive listener:

1. Outer's bootstrap runs: `const channel = new MessageChannel();`
2. Outer transfers `port2` up: `parent.postMessage({ type: 'tonk:port' }, '*', [channel.port2]);`
   The `'*'` target origin is required — outer's origin is opaque
   and can't be named.
3. Top frame's `<tonk-portal>` has a `'message'` listener on
   `window` scoped to messages from *this portal's* outer iframe
   (filtered by `event.source === iframe.contentWindow`). On
   match, it pulls `event.ports[0]` and binds it as the host-side
   port for this portal.
4. Outer's bootstrap then assigns `window.tonk = { query, subscribe, transact }`,
   each implemented in terms of `channel.port1`.
5. Outer's bootstrap creates the inner iframe and sets its `srcdoc`
   to the artifact HTML. The bootstrap *waits* on the inner's
   `load` event before signalling `tonk-portal:connected` upward,
   so the host knows the artifact is mounted.

Why guest-creates-port:

- Top doesn't have to listen for the iframe's `load` and time its
  `postMessage` against it. The handshake completes when the
  outer is ready to do work, not when the browser fires `load`.
- The port is bound to *this* outer iframe instance. If the outer
  reloads (which it does on `source` / `space` / `branch` change),
  the host gets a fresh port over a fresh `'tonk:port'` message;
  the old one becomes garbage.
- Filtering top-side by `event.source === iframe.contentWindow`
  prevents spoofing from other frames (extensions, ads, sibling
  portals).

## Wire shape

Two surfaces, two transports:

- Request/response over the port for one-shots (`query`,
  `transact`).
- Transferred `ReadableStream` for live subscriptions.

With this split, the host keeps no subscription-lifecycle state
(see "Subscriptions return streams" below).

### Request envelopes (outer → top, over port)

```ts
type Request =
  | { id: number, kind: 'query',     query: Query }
  | { id: number, kind: 'subscribe', query: Query }
  | { id: number, kind: 'transact',  body: Transaction };
```

`id` is an outer-allocated correlation ID for one-shots. The
outer holds the `Map<id, { resolve, reject }>` of pending
promises.

### Response envelopes (top → outer, over port)

```ts
type Response =
  | { id: number, kind: 'result', value: unknown }       // query / transact reply
  | { id: number, kind: 'stream', stream: ReadableStream } // subscribe reply
  | { id: number, kind: 'error',  error: { kind: string, message: string } };
```

For `subscribe`, the top transfers a `ReadableStream` (along with
the envelope) over the port. The outer receives the stream and
either consumes it itself or hands it to the inner via
`parent.tonk.subscribe`. Same-origin frames share heap, so the
hand-off from outer to inner is a property assignment, not a
second transfer.

### Subscriptions return streams

`tonk.subscribe(query)` returns `Promise<ReadableStream<Conclusion[]>>`.
The top frame:

1. Receives the `subscribe` envelope on the port.
2. Opens an SSE connection to `/api/repository/{space}/branch/{branch}/query`.
3. Wraps the SSE in a `ReadableStream` whose `pull` reads the next
   SSE frame and `enqueue`s a parsed `Conclusion[]`.
4. Wraps `cancel` so it aborts the SSE upstream when the stream
   is cancelled.
5. Posts `{ id, kind: 'stream', stream }` over the port with the
   stream in the transfer list.

When the inner stops reading — explicit `reader.cancel()`,
`reader.releaseLock()` + drop, or GC of all stream references —
cancellation propagates back through the transferred stream to
the top's underlying source, which aborts the SSE. There is no
abort envelope, no subscription map, no correlation ID for
streams.

Backpressure is inherited from `ReadableStream`: if the inner
consumes slowly, `pull` isn't called, the SSE isn't drained, and
the network buffer fills.

### Why streams instead of framed envelopes

The alternative is to keep subscriptions on the same port: `subscribe`
yields a sequence of `frame` envelopes correlated by `id`, with an
explicit `abort` envelope for teardown. Transferring a
`ReadableStream` instead has three consequences relevant here:

- Host bridge state holds only one-shot pending promises. Subscriptions
  carry no host-side state.
- Subscription teardown is implicit: dropping the stream propagates
  cancellation, which aborts the upstream SSE. No abort envelope, no
  registration that can be leaked.
- The guest API is `for await (const frame of stream)` rather than a
  callback or registration-based shape.

Constraints:

- Requires transferable `ReadableStream`. Available in Chrome,
  Firefox, and Safari 16.4+.
- A guest that stalls its reader stalls only its own SSE; the host's
  pull-side blocks waiting on the stream.

## Guest API surface

```js
// On window.tonk in the outer; reachable as parent.tonk from inner.
window.tonk = {
  async query(query)    { /* → Conclusion[] */ },
  async subscribe(query) { /* → ReadableStream<Conclusion[]> */ },
  async transact(body)  { /* → receipt */ },
};
```

Artifact author usage:

```js
// One-shot.
const rows = await parent.tonk.query(q);

// Live subscription.
const stream = await parent.tonk.subscribe(q);
for await (const frame of stream) {
  render(frame);
}
```

Async-iterator support comes from `ReadableStream`'s native
`[Symbol.asyncIterator]` (Chrome, Firefox, Safari).

## Web Awesome and host styles

Custom-element registries are per-`Window`, not per-origin. Any
component the host has registered on the top frame is not visible
to the outer or inner iframe; each frame has its own empty
registry.

The bootstrap registers the Web Awesome components the host
deems available to artifacts (`<wa-button>`, `<wa-input>`, etc.)
in the outer iframe's registry. Because outer and inner share an
origin, an inner-iframe registration referencing
`parent.customElements` upgrades elements declared in the
artifact's HTML.

The bootstrap also injects the host's Web Awesome stylesheet into
both the outer and inner documents, so artifact authors get the
same visual baseline without having to fetch or inline anything.
The exact set of registered components and the stylesheet payload
are settled during implementation.

Tonk-specific elements that perform their own SW-backed fetches
are not registered into the portal; they have no working data
path inside an opaque-origin iframe.

## Content-change strategy

Two layers can be invalidated independently:

- Artifact content changes (the live subscription's `content`
  claim on the branch updates):
  - The outer stays mounted; the bridge port is preserved. The
    outer's bootstrap receives a `tonk:reload` message from the
    top with the new HTML, removes the existing inner iframe,
    and creates a fresh inner iframe with the new `srcdoc`.
  - Subscriptions the old inner had open are torn down when its
    scripts and stream references go out of scope (same GC path
    as ordinary teardown).
- Attribute change (`source` / `space` / `branch` on the
  `<tonk-portal>` host):
  - Tear down the outer iframe. Allocate a fresh outer with a
    fresh bridge. Re-resolve. Same path as
    `disconnected_callback`.

The asymmetry reflects what changed: a content update doesn't
invalidate the bridge or the bootstrap, so they're kept. An
attribute change invalidates the data subscription itself, so
the whole flow restarts.

## Data flows

One host-side flow:

Content subscription. A live subscription to
`(the = xyz.tonk.artifact/content, of = <source>)` opened against
`/api/repository/{space}/branch/{branch}/query`. Frame size is
0 or 1 (cardinality-one claim).

On the first non-empty frame the host mounts the outer iframe.
On subsequent frames it posts `tonk:reload` to the existing
outer. Empty stream sets `data-state="empty"`; failure sets
`data-state="error"`.

In parallel, the bridge inside the outer runs whatever data
flows the artifact's scripts initiate via `parent.tonk.query` /
`.subscribe` / `.transact`. Those have their own lifecycle,
independent of the content subscription. The host content
subscription drives the artifact reload; the artifact's own
queries don't.

## Threat model

The trust boundary that matters is top ↔ outer. Everything
across it goes through `postMessage`.

What the artifact (inner) can do, given same-origin access to the
outer:

- Read and modify the outer's DOM.
- Read and modify `parent.tonk`, which is a property on the
  outer's `window`.
- Read the outer's bootstrap source via
  `parent.document.documentElement.outerHTML`.

The outer holds no secrets: it is a constant, inlined bootstrap.
Replacing `parent.tonk` only breaks the artifact that did so.
The outer's only stateful objects are the `MessagePort` reference
and the pending-promise map; if the artifact grabs the port and
posts garbage, the top-side message handler is what enforces
shape.

Top-side defences on the port:

- Validate every incoming envelope shape: `kind` is one of three
  known strings, `id` is a finite number, `query` is a
  serializable object.
- Reject unknown `kind` values without throwing.
- Treat `id` as scoped to a port; pending maps are per-port.

What the artifact *cannot* do:

- Reach `top.tonk`, `top.document`, `window.top.*` (opaque-origin
  isolation).
- Make a network request that reaches anything (no network at all
  in opaque-origin iframes).
- Persist anything across reload (no cookies, no storage).
- Navigate the top or open popups.

## Lifecycle events

| Event | When | Detail |
|---|---|---|
| `tonk-portal:connected` | Outer's port received and inner's `load` fired | `{ source }` |
| `tonk-portal:content` | Inner reloaded with new artifact content | `{ source }` |
| `tonk-portal:error` | Lookup / network / parse failure | `{ kind, message }` |

`data-state` is the canonical styling signal; events are for
diagnostics.

## Bootstrap behaviour (detail)

The outer's bootstrap script, top to bottom:

1. Create `const channel = new MessageChannel();`.
2. Set up `channel.port1.onmessage` to dispatch incoming
   envelopes by `id` against a pending-promise map. For
   `kind === 'stream'` envelopes, resolve the corresponding
   pending promise with the transferred `event.data.stream`.
3. Define `window.tonk = { query, subscribe, transact }`, each
   one allocating a fresh `id`, posting an envelope on `port1`,
   and returning a promise that the `onmessage` handler resolves.
4. Set up a `window.message` listener for `tonk:reload` from the
   top: remove the current inner iframe, create a new one, set
   its `srcdoc` to `event.data.html`.
5. Post `{ type: 'tonk:port' }` to `parent` with `[channel.port2]`
   in the transfer list.
6. Wait for an initial `tonk:reload` from the top carrying the
   first artifact HTML, then create the inner iframe.

The ordering matters: `window.tonk` is defined before the inner
is created, so once inner scripts run, `parent.tonk` is populated.

## Host-side bridge state

The `<tonk-portal>` element keeps, per instance:

- A reference to the outer iframe element.
- A reference to the `MessagePort` (the `port1` the outer
  transferred up — labelled `port2` in the outer's code, but
  it's the host's "port1" from the host's perspective).
- A `Map<id, Pending>` for one-shot correlation IDs (queries and
  transacts only). Subscriptions are zero state.
- The `AbortController` for the content subscription.

On attribute change / disconnect:

- Abort the content subscription.
- Remove the outer iframe (which removes the inner, which drops
  all guest-initiated streams, which cancels their SSEs).
- Drop the port reference, drop the pending map.

## Wrapper

There is no per-artifact wrapper. The outer's `srcdoc` is a
constant; the inner's `srcdoc` is the artifact's `content` claim,
unmodified.

```rust
const OUTER_BOOTSTRAP: &str = include_str!("bootstrap.html");

fn outer_srcdoc() -> &'static str {
    OUTER_BOOTSTRAP
}

fn inner_srcdoc(artifact_content: &str) -> &str {
    artifact_content
}
```

Both return `&str` because there is nothing to format. They are
kept as functions so a later revision can introduce header
injection without changing call sites.

## Crate layout

```
rust/tonk-portal/           # new
  Cargo.toml
  src/
    lib.rs                  # pub fn register() — wasm32
    element.rs              # CustomElement impl, lifecycle, content subscription
    bridge.rs               # MessagePort listener, envelope routing,
                            # ReadableStream construction for subscriptions
    bootstrap.html          # outer's srcdoc, include_str!'d
    resolve.rs              # query builder: content-by-DID
    state.rs                # data-state reflection (mirror of tonk-display)
    error.rs
```

Registered alongside `tonk_display::register()` in
`rust/tonk-ui/src/bin/ui.rs`.

`Cargo.toml` deps mirror `tonk-display`'s — `custom-elements`,
`js-sys`, `web-sys` (add `HtmlIFrameElement`, `MessageChannel`,
`MessagePort`, `MessageEvent`, `ReadableStream`), `wasm-bindgen`,
`wasm-bindgen-futures`, `tonk-concept` (for SSE helpers + error
types — `open_sse`, `ErrorDetail`), `tonk-schema`, `serde`,
`serde_json`.

## Implementation order

1. Skeleton `tonk-portal` crate + `register()`; observe
   attributes; reflect `data-state="loading"`; mount an empty
   outer iframe with the bootstrap.
2. Bridge handshake: outer posts port up, host receives it,
   round-trip a no-op envelope to verify wiring.
3. `parent.tonk.query` end-to-end: outer's `window.tonk.query`
   sends envelope, host POSTs to
   `/api/repository/{space}/branch/{branch}/query`, replies on
   the port, outer's promise resolves.
4. Content subscription: subscribe to
   `(the = xyz.tonk.artifact/content, of = <source>)`. Mount the
   inner iframe on first frame.
5. `tonk:reload` path: host posts new content to outer on
   subsequent frames; outer rebuilds the inner.
6. `parent.tonk.subscribe` returning a transferred
   `ReadableStream`. Host wraps SSE → stream; cancellation
   propagates.
7. `parent.tonk.transact` plumbing.
8. Empty-stream → `data-state="empty"`; failure → `data-state="error"`.
9. Register in `tonk-ui` shell.

## Open questions

1. `tonk.transact` authorization for untrusted artifacts.
   Forwarded verbatim in v1. Public deployment would require
   host-side gating — allowlists, per-artifact capabilities,
   user confirmation prompts.
2. Streaming binary results. `tonk.query` returns JSON. Artifacts
   that want images or large blobs as stream chunks would need a
   separate envelope shape. Out of scope for v1; the envelope's
   `value: unknown` leaves room.
3. Which Web Awesome components and which subset of the host's
   stylesheet to expose to artifacts. Settled during
   implementation.
