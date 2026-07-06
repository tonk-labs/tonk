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
a page-level singleton (`plan/tonk-host.md`) but drifted into being
re-mounted at every call site. `<tonk-repository>` and `<tonk-branch>`
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
- `meta@profile` — `profile` is a **reserved repo token** meaning the
  profile-as-repository endpoint (`/api/profile/branch/…`), distinct
  from any `did:key`. It routes to the profile endpoint, not to a named
  repository.

`allow` additionally accepts two reserved tokens:

- `*` — reach anything (the privileged case; today's `cross_repo=true`).
- `self` — reach exactly this site's own `with` (the sealed case).

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
<tonk-site with="meta@profile" allow="*">…</tonk-site>

<!-- a sealed space site, opened at /space/{id} -->
<tonk-site with="main@did:key:zAlice" allow="self">…</tonk-site>

<!-- a nested router site within the same space -->
<tonk-site with="main@did:key:zAlice" allow="self" path={rest}>…</tonk-site>
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
Privilege never leaks downward — a bare-ish `<tonk-site allow="self">`
inside a `<tonk-site allow="*">` parent is sealed, even though its
parent is privileged. Each site re-declares its own reach.

### Consumers (`<tonk-display>`, `ui-sync-status`, …)

Consumers gain an **optional** `with` attribute. `allow` is meaningless
on a consumer (consumers grant no reach) and is not read there.

- **Absent `with`** → inherit the enclosing site's `with`. This is the
  common case: the vast majority of displays in the profile carry no
  `with` and render against `meta@profile`.
- **Present `with`** → a *request* for that location, honored only if
  the enclosing site's `allow` permits it. Inside a `allow="*"` profile
  site the request is honored; inside a sealed `allow="self"` space
  site the same request is denied.

```html
<!-- inherits the enclosing site's context -->
<tonk-display entity={x} model=tonk:foo view=tonk:view/label>

<!-- hub sync chip reaching into a specific space; honored because the
     enclosing profile site is allow="*" -->
<ui-sync-status with="main@did:key:zBob"></ui-sync-status>
```

### `<tonk-host>`

Becomes a boot-time singleton again (its original design intent).
Installed once at app startup, attached to the document; consumer
events bubble to it. Removed from every template. The sealed guest
keeps its own `<tonk-host>` relay proxy (`rust/tonk-guest/src/guest_host.rs`)
unchanged — the guest still needs an ancestor to catch consumer events
and relay them over `window.tonk`.

## Enforcement

The trust model is unchanged in shape and already shipped; only the
granularity and the deny behavior change.

- The **guest enforces nothing.** `guest_host` relays the descendant's
  requested `(space, branch)` verbatim over `window.tonk`. A malicious
  guest that skipped enforcement would gain nothing, so enforcement in
  the guest is worthless.
- The **trusted host side decides.** `PortalState`'s envelope
  dispatcher receives the relayed request and checks it against the
  site's `allow` before handing it to the real `<tonk-host>`.

The single chokepoint is `forwarded_route` in
`rust/tonk-portal/src/bridge.rs`. Migration:

1. Replace `PortalState.cross_repo: bool` with the parsed `allow`
   list. `connect_portal` (`shared.rs`) takes the `allow` spec instead
   of a `bool`; `<tonk-site>` derives it from its `allow` attribute
   instead of `site.rs:248` passing a hardcoded `true`.
2. `forwarded_route` matches the requested `(branch, repo)` against the
   `allow` list:
   - request matches an `allow` entry (`*`, `self`==the site's `with`,
     or an explicit `branch@repo`) → honor it, relay onward.
   - request is absent → use the site's own `with` (the pinned
     default). Unchanged.
   - request is present but **not** permitted → return a **typed
     denial**, and `post_error` back over the port so the guest's
     consumer renders a failure state. This replaces today's silent
     coercion (`return (None, None, false)`), which serves the pinned
     context under a mismatched assumption.

The typed denial (`Denied { requested: BranchRepo }`) is deliberately
not collapsed to `None`: it is the seam for a future *capability
request* flow, where an un-listed request prompts to extend `allow`
rather than simply failing. Today it errors; the shape leaves room to
escalate later without touching the chokepoint again.

### Behavior change to watch

Flipping `connect_portal`'s hardcoded `true` to an `allow`-derived
value **tightens** every existing site. Today all sites are effectively
`allow="*"`. After migration, a space site is `allow="self"` and will,
for the first time, deny a guest that forwards an off-repo route. Land
the typed-denial path with logging first to surface anything currently
relying on the silent-coercion pin (there should be nothing, but the
capability exists today).

## Migration

1. **Grammar + parser** — a `Location` (`branch@repo`, with `profile`
   reserved) and an `Allow` list (`*`, `self`, explicit locations).
   Parse at connect; error on malformed. Lives host-side (likely
   `rust/tonk-host` or a shared crate both `tonk-host` and
   `tonk-portal` depend on).
2. **`<tonk-host>` singleton** — install once at boot; drop the
   per-call-site mounts.
3. **`<tonk-site with= allow=>`** — read both attributes (required),
   register with `with` as handshake context, pass `allow` to
   `connect_portal`.
4. **`forwarded_route`** — `allow` match + typed denial + error-back.
5. **Consumers** — optional `with`; absent inherits, present requests.
   Fold the annotator behavior onto the consumer (or keep one thin
   internal annotator that reads `with`).
6. **Templates** — rewrite `profile.yaml`'s ~8 triples to bare
   consumers (inheriting `meta@profile`) or `with=`-carrying chips;
   rewrite the two `<tonk-site>`-wrapped-in-triple routes to
   `<tonk-site with= allow= path=>`; update `ui.rs` / `hub.rs` /
   `space_sealed.rs` mint sites.
7. **Retire** `<tonk-repository>` / `<tonk-branch>` as an authoring
   surface once no template references them.

## Final surface

Three elements, two attributes:

- **`<tonk-site with="…" allow="…">`** — context + reach boundary +
  router + isolation. Both attributes required.
- **consumers** (`<tonk-display>`, `ui-sync-status`, …) — optional
  `with`; absent inherits the enclosing site, present is a request
  gated by that site's `allow`.
- **`<tonk-host>`** — boot singleton, invisible in templates; guest
  keeps its relay proxy.

`<tonk-repository>` and `<tonk-branch>` disappear from templates. The
enforcement seam (`forwarded_route`) is reused, migrated from an
all-or-nothing bool to an `allow` ACL, with silent coercion replaced by
an explicit typed denial that doubles as the future escalation hook.
