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

Host-relative `window.fetch` calls cross the same isolation boundary. The parent
browser-normalizes the path once, then applies a method-aware default-deny
allowlist before it constructs or sends a request. Sealed content may read the
public guest asset namespaces and deployment discovery and call
repository/profile data-plane routes inside the portal's `with` / `allow`
reach. `/api/language-server` is an author-facing alias, not a worker-global
endpoint: a portal with one trusted `with` reach resolves it to that exact
repository/profile + branch route and stamps a portal client identity so server
state and diagnostics cannot cross clients. An ambiguous portal without `with`,
or an explicit scoped LSP path outside the allowed reach, is denied before
fetch. Account, profile roster, repository lifecycle, global site/sync,
inspection, and undeclared worker routes are likewise not reachable through
this relay.

Guest-supplied `X-Tonk-Site`, `X-Tonk-Path`, `X-Tonk-Hash`, `X-Tonk-Build`, and
`X-Tonk-Lsp-Client` values are discarded as direct authority. Authorized
worker requests receive trusted host context, while each LSP relay prepends its
host-minted random segment to a bounded canonical descendant chain. A caller
cannot replace an authorized ancestor, and same-scope nested siblings remain in
distinct worker sessions. Malformed, ambiguous, non-canonical, and over-depth
values fail closed or collapse only to the current relay's own principal.
Public asset/discovery requests receive none of those internal headers.

The sealed runtime captures its host relay function synchronously before any
authored markup or script runs, then passes that function directly into guest
Wasm through a Rust-only setter. Nested portals call the retained capability
instead of the mutable authored `window.fetch`. Trusted descendant-principal
headers therefore never cross an authored fetch wrapper, so same-scope sibling
code cannot learn and replay a legitimate child's principal even though both
guests execute inside the same sealed realm.

This deliberately means the sealed Hub does not load or mutate the browser
profile roster. Its neutral **account** cell forwards an ordinary navigation to
the trusted top-level `/settings` route; listing, switching, and adding accounts
happen there until equivalent account chrome exists outside the guest realm.
The portal still relays the exact typed stale-build signal through every nested
host layer so the trusted top document can show its existing update prompt.

## Modules

- [`element`](src/element.rs): the `<tonk-portal>` custom element: lifecycle, iframe ownership, `srcdoc` painting, and the `reset` / `error` prototype shims.
- [`bridge`](src/bridge.rs): the iframe bootstrap, the page-level `hello` listener and registry, port binding, and the envelope dispatcher relaying to host consumer events.
- [`query`](src/query.rs): wire-query construction for no-argument bridge calls.
