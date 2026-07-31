# Routing attributes — collapse `<tonk-host>`/`<tonk-repository>`/`<tonk-branch>` into `with` + `allow`

## Problem

Every data-binding site in the UI is authored as a four-element
stack:

```html
<tonk-host>
  <tonk-repository name={id}>
    <tonk-branch name="main">
      <tonk-display …>
    </tonk-branch>
  </tonk-repository>
</tonk-host>
```

The profile view alone repeats this triple more than a dozen times
— sync chips, labels, hub cards, the fab, the pause-sync banner (see
`rust/tonk-core/assets/library/profile.yaml`). The routing route
`<tonk-host><tonk-repository name={id}><tonk-branch name=main><tonk-site path={rest}>`
does it once more.

Three of the four elements exist only to carry two strings (a
repository and a branch) to the IO owner. `<tonk-host>` is meant to be
a page-level singleton but drifted into being re-mounted at every call
site. `<tonk-repository>` and `<tonk-branch>`
are pure passive annotators — no state, no IO — that stamp
`detail.space`/`detail.branch` on the bubble.

Meanwhile the interesting behavior — which repositories a subtree is
*allowed* to reach — is expressed today as an all-or-nothing
`cross_repo: bool` on `PortalState` (`rust/tonk-portal/src/bridge.rs`),
granted unconditionally to every `<tonk-site>` (`site.rs:248` passes
`true`). A sealed space site can currently reach any repo; nothing
enforces isolation, and an off-repo request from a non-privileged
portal is *silently coerced* to the pinned context rather than denied.

## Proposal

Two attributes, carried directly on the elements that need them, using
a grammar that mirrors the iframe `sandbox` attribute (space-separated
tokens). The three routing elements disappear as an authoring surface.

- **`with="<branch>@<repo>"`** — the context a subtree operates with.
- **`allow="<token> …"`** — the set of contexts a site permits its
  descendants to reach.

### The `branch@repo` grammar

A location is written `<branch>@<repo>`:

- `main@did:key:zAlice` — the `main` branch of Alice's repository.
- `did:key:zAlice` — a bare repo means that repo's **default branch**.
- `meta@profile:tonk` — a `profile:<name>` repo token means the
  profile-as-repository endpoint (`/api/profile/branch/…`), distinct
  from any `did:key`. The name (`tonk`, the profile the worker opens)
  is carried for forward compatibility; the endpoint is singular
  today. There is no bare `profile` token — it parses as an error
  pointing at `profile:<name>`. Future: address the profile by its
  `did:key` like any repository (the top window can learn it at
  boot), retiring the prefix entirely.

`allow` additionally accepts one reserved token:

- `*` — reach anything (the privileged case; the old `cross_repo=true`).

The sealed case is the embedder listing the site's own `with`
explicitly (`allow="main@{id}"` next to `with="main@{id}"`) — the same
binding wired into two attributes. There is deliberately no `self`
sentinel: whoever can write `with` can write the same value into
`allow`, and dropping the sentinel keeps the allow list a plain set of
locations with nothing to resolve against.

Both attributes parse to a structured form **at connect time**, not
per-request. A malformed `with`/`allow` is a visible error at mount,
never a silent deny at query time (parse, don't validate).

### `<tonk-site>` — the context + reach boundary

`<tonk-site>` becomes the sole routing element. It carries **both**
attributes, and **both are required** — a site missing either is
malformed and renders an error at connect. There is no inheritance and
no defaulting for a site's own attributes: every site is fully
self-describing.

```html
<!-- top / profile site, minted by the host at boot -->
<tonk-site with="meta@profile:tonk" allow="*">…</tonk-site>

<!-- a sealed space site, opened at /space/{id} -->
<tonk-site with="main@did:key:zAlice" allow="main@did:key:zAlice">…</tonk-site>

<!-- a nested router site within the same space -->
<tonk-site with="main@{id}" allow="main@{id}" path={rest}>…</tonk-site>
```

Requiring both eliminates every "what does absence mean on a site"
branch we would otherwise need (inherit vs. root-default vs. error).
The recursive-router case restates its parent's `with`/`allow`; since
those values are template-interpolated from route params (`{id}`),
this is wiring the same binding into two attributes, not hand-typed
repetition. We deliberately do **not** add a `with="inherit"`
sentinel — it would reintroduce the inheritance ambiguity the
required-both rule removes.

`with` and `allow` are **independent**. A nested site inherits nothing
from its parent: it declares its own `with` and its own `allow`.
Privilege never leaks downward — a site allowing only its own location
inside a `<tonk-site allow="*">` parent is sealed, even though its
parent is privileged. Each site re-declares its own reach.

### Consumers (`<tonk-display>`, `ui-sync-status`, …)

Consumers gain an **optional** `with` attribute. `allow` is meaningless
on a consumer (consumers grant no reach) and is not read there.

- **Absent `with`** → inherit the enclosing site's `with`. This is the
  common case: the vast majority of displays in the profile carry no
  `with` and render against `meta@profile:tonk`.
- **Present `with`** → a *request* for that location, honored only if
  the enclosing site's `allow` permits it. Inside a `allow="*"` profile
  site the request is honored; inside a sealed space site (allow =
  exactly its own location) the same request is denied.

```html
<!-- inherits the enclosing site's context -->
<tonk-display entity={x} model=tonk:foo view=tonk:view/label>

<!-- hub sync chip reaching into a specific space; honored because the
     enclosing profile site is allow="*" -->
<ui-sync-status with="main@did:key:zBob"></ui-sync-status>
```

### `<tonk-host>` — removed entirely

There is no `<tonk-host>` element at all, not even a boot singleton.
The element has zero DOM semantics: it is only an ancestor event
target for the bubbling consumer events, a holder of `HostState`
(subscription registry + query LRU), and the installer of the
navigate / idle-sync listeners (both already standalone installs).
All of that becomes a boot-time `tonk_host::install()`: the operation
listeners attach to `document` (bubbling events reach it for free),
state lives in a `thread_local`, and a document-level
`MutationObserver` on the `with` attribute drives the existing
depth-staggered subscription refresh (replacing the routing elements'
`attributeChangedCallback` → `tonk-context-refresh` flow).

The same reasoning removes the guest's `<tonk-host>` relay proxy
(`rust/tonk-guest/src/guest_host.rs`) — and with it the whole
per-operation envelope relay. The guest installs the REAL host IO
surface (`tonk_host::install_io()`, operation listeners + `with`
observer, no top-page effects): consumer events are serviced in the
guest document over plain `fetch`/SSE, and the portal bootstrap's
`window.fetch` override relays each request to the outer frame as
HTTP. Events never cross the iframe boundary; only HTTP does.
`window.tonk` remains sugar for app code. A consumer with no `with`
in its tree falls back to the portal's pinned context, delivered by
the bridge as `context.with`. `<tonk-host>` then ceases to exist
everywhere — custom-element registry, templates, and the guest
content minted by `site_content.rs`.

Consumers resolve their context from the nearest ancestor (including
self) carrying a `with` attribute, innermost wins; one resolver in
`tonk-host`, shared by the real host's handlers (reading from
`event.target` at handle time) and the guest relay's route
forwarding. This keeps grouped wrappers expressible as a plain
`<span with="main@{subject}">…</span>` and generalizes
`tonk-workspace`'s `repo_from_ancestor`.

## Enforcement

The trust model is unchanged in shape and already shipped; only the
granularity and the deny behavior change.

- The **guest enforces nothing.** Its element IO is plain `fetch`
  against explicit `/api/…` URLs (built from its `with` resolution or
  the pinned `context.with`), relayed verbatim; `window.tonk` sugar
  forwards its resolved location string. A malicious guest that
  skipped local checks would gain nothing, so enforcement in the
  guest is worthless.
- The **trusted host side decides**, at two chokepoints in
  `rust/tonk-portal/src/bridge.rs`: `handle_host_fetch` gates every
  relayed branch data-plane URL (`/api/repository/{repo}/branch/…`,
  `/api/profile/branch/…`) against the portal's `with`/`allow` —
  covering the elements' fetches and raw guest `fetch()` calls alike —
  and `forwarded_route` gates the `window.tonk` envelope path the
  same way.

Migration at `forwarded_route`:

1. Replace `PortalState.cross_repo: bool` with the parsed `allow`
   list. `connect_portal` (`shared.rs`) takes the `allow` spec instead
   of a `bool`; `<tonk-site>` derives it from its `allow` attribute
   instead of `site.rs:248` passing a hardcoded `true`.
2. `forwarded_route` matches the requested location against the
   `allow` list:
   - request matches an `allow` entry (`*` or an explicit
     `branch@repo`) → honor it, relay onward.
   - request is absent → use the site's own `with` (the pinned
     default), stamped explicitly — there are no ambient DOM
     ancestors to fall back on.
   - request is present but **not** permitted (or malformed) → return
     a **typed refusal**, and `post_error` back over the port so the
     guest's consumer renders a failure state. This replaces today's
     silent coercion (`return (None, None, false)`), which serves the
     pinned context under a mismatched assumption.

The typed refusal (`Refused::Denied { requested: Location }`) is
deliberately not collapsed to `None`: it is the seam for a future
*capability request* flow, where an un-listed request prompts to
extend `allow` rather than simply failing. Today it errors; the shape
leaves room to escalate later without touching the chokepoint again.

### Behavior change to watch

Flipping `connect_portal`'s hardcoded `true` to an `allow`-derived
value **tightens** every existing site. Today all sites are effectively
`allow="*"`. After migration, a space site allows only its own
location and will, for the first time, deny a guest that forwards an
off-repo route. Land the typed-denial path with logging first to
surface anything currently relying on the silent-coercion pin (there
should be nothing, but the capability exists today).

## Migration

1. **Grammar + parser** — a `Location` (`branch@repo`, with
   `profile:<name>` for the profile endpoint) and an `Allow` list
   (`*` or explicit locations). Parse at connect; error on malformed.
   Lives in `rust/tonk-host/src/location.rs` (target-independent,
   natively tested).
2. **De-element `tonk-host`** — `tonk_host::install()` at boot
   (document listeners + navigate + idle-sync + `with` observer);
   delete the `TonkHost` element and the guest proxy element (the
   guest installs its relay on `document` at `start()`); drop the
   `<tonk-host>` wrapper from `site_content.rs` guest content and
   every per-call-site mount.
3. **`<tonk-site with= allow=>`** — read both attributes (required),
   register with `with` as handshake context, pass `allow` to
   `connect_portal`.
4. **`forwarded_route`** — `allow` match + typed denial + error-back.
5. **Consumers** — optional `with`; absent inherits, present requests.
   Context resolves via the shared nearest-`[with]`-ancestor resolver
   at handle time; the bubble-phase annotator elements are deleted.
6. **Templates** — rewrite `profile.yaml`'s ~8 triples to bare
   consumers (inheriting `meta@profile`) or `with=`-carrying chips;
   rewrite the two `<tonk-site>`-wrapped-in-triple routes to
   `<tonk-site with= allow= path=>`; update `ui.rs` / `hub.rs` /
   `space_sealed.rs` mint sites.
7. **Retire** `<tonk-repository>` / `<tonk-branch>` as an authoring
   surface once no template references them.

## Final surface

One routing element, two attributes:

- **`<tonk-site with="…" allow="…">`** — context + reach boundary +
  router + isolation. Both attributes required.
- **consumers** (`<tonk-display>`, `ui-sync-status`, …) — optional
  `with` (on themselves or a plain wrapper element); absent inherits
  the enclosing site, present is a request gated by that site's
  `allow`.

`<tonk-host>`, `<tonk-repository>`, and `<tonk-branch>` disappear —
from templates and from the custom-element registry. The host is a
boot-time `install()`; the guest relay is a document-level listener
set installed at guest `start()`. The
enforcement seam (`forwarded_route`) is reused, migrated from an
all-or-nothing bool to an `allow` ACL, with silent coercion replaced by
an explicit typed denial that doubles as the future escalation hook.

Also removed in passing: the host's query-response LRU. The reactor
already multiplexes subscriptions per `(branch, query)` hash — the hot
path — and the resolved-response cache had an incomplete invalidation
story (flushed on this page's writes only, never on sync pulls, other
tabs, or worker-side commands). One-shots are same-machine SW
round-trips; if a mount stampede ever shows up, reintroduce just the
in-flight promise-coalescing layer, which is small and staleness-free.

## Future ideas (deliberately not bundled here)

- **Profile by DID.** The `profile:<name>` token exists because the
  profile endpoint (`/api/profile/branch/…`) is a separate namespace
  from `/api/repository/{repo}`. If the worker serves the profile
  repository under its own `did:key` (and the top window reads that
  DID at boot, e.g. off `GET /api/profile`), the prefix retires and a
  profile location becomes an ordinary `meta@did:key:…`.
- **Cross-source `<tonk-display>`.** A display resolves view + model +
  entity data all against one context. `ui-sync-status` exists only
  because we could not say "take the concept and view from the
  profile, subscribe to the data in the target repo". A per-facet
  source (e.g. `with` for the data subscription, a separate location
  for view/model resolution) would let a stdlib view render foreign
  data without a bespoke host element per case.
- **Data plane on the space URL.** Instead of rewriting a location
  into `/api/repository/{repo}/branch/{branch}/{query|transact|…}`,
  send the operation to the space's own URL
  (`/space/did:key:…/branch/main/`) and let method + content type
  distinguish query / subscribe / transact (SSE `Accept` already
  distinguishes subscribe today). Then a location IS a URL prefix,
  and a `<tonk-site>` resolves its route paths by ordinary URL
  resolution against its own `with` — no separate rewrite layer.
  The URL builder (`url.rs`) and the worker route table are the only
  seams; the `Location` grammar is unchanged by it.
