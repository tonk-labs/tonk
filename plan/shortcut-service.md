# Shortcut service — short invite links via same-origin redirects

Status: implemented (service, SW bypass, CLI mint/claim, worker mint).
Invite URLs embed the whole serialized UCAN chain
(`?access=<base58 chain>#seed`), which makes them enormous and hostile
to chat clients and QR codes. Instead of changing the invite format,
the **shortcut service** shortens any same-origin URL: the long URL's
path + query is stored under its blake3 hash, and the short link is

```
https://tonk.spot/@/{blake3-hash}#{seed}
```

`GET /@/{hash}` answers with a permanent redirect whose `Location` is
the stored string — **relative, no hostname** — and the browser does
the rest: RFC 7231 §7.1.2 makes a redirect whose `Location` carries no
fragment inherit the fragment of the original URI, so the `#seed`
rides across client-side and never reaches any server. The claim flow
is completely untouched; the recipient lands on the ordinary
`/join?access=…#seed` URL.

## Why permissionless is sound

Earlier iterations of this design (see the `archive/announcement-invites`
branch and PR #586's history) gated publishing behind a
`/announcement/publish` capability. The shortcut service needs none of
that, for two reasons that hold together:

1. **The stored half is non-secret by construction.** An audience-open
   invite's chain is unredeemable without the ephemeral private key,
   whose seed lives in the fragment — which browsers never send and
   the service never stores. Harvesting every stored target yields
   nothing claimable.
2. **A relative redirect cannot leave the origin.** The service stores
   path + query only — never a scheme or authority — so it is useless
   as an open redirector. Validation rejects absolute URLs,
   protocol-relative (`//host`) references, fragments (an explicit
   fragment in `Location` would defeat the inheritance), control
   characters (header injection), and anything over 8 KiB.

What remains is "free storage of ≤8 KiB strings", bounded by a rate
limit (ops, below) — a pastebin nobody would bother with.

## Interface

- **`PUT /@[?ttl=<days>]`** — body is the path + query string.
  Responds with the base58 blake3 hash of the body. Stored at
  `tonk/link/{hash}` in the existing bucket via the Worker's R2
  binding, stamped with `expires` custom metadata: `min(ttl, 20)`
  days, defaulting to 7 when `ttl` is absent. Day granularity is
  deliberate — it mirrors what R2 lifecycle rules (the physical
  cleanup) can express, so the API promises nothing the storage
  can't keep. The key is derived from the content, so a repeated PUT
  can only re-store identical bytes — it acts as an idempotent
  expiry refresh, never a repoint.
- **`GET /@/{hash}`** — `301` with the stored string as a relative
  `Location`; `Cache-Control: public, max-age=min(remaining, 86400)`
  so no cache outlives the logical expiry. `404` for unknown *or
  expired* hashes, `400` for segments that aren't base58 of 32 bytes.

Core validation lives in `tonk-access-service/src/shortcut.rs`
(platform-independent, shared with the native test server); client
glue (URL splitting, short-link assembly, manual `Location`
resolution) in `tonk-invite/src/shortcut.rs`.

## Integration points

- **Service worker bypass** (`tonk-ui/assets/service_worker.js`): the
  SW serves the app shell cache-first for navigations, which would
  swallow the redirect for any user who already has the SW installed.
  `/@` and `/@/*` requests are not intercepted at all — the browser
  talks to the edge worker directly and applies redirect + fragment
  semantics natively.
- **Minting**: the worker's `POST /api/repository/{repo}/invite`
  shortens when the caller supplied `base_url` (the UI always sends
  `window.origin`; the hardcoded default base is never PUT to, so
  tests and offline mints stay network-free). The CLI shortens in the
  `tonk invite` command layer — after `invite::mint` — so
  `share`'s compose-then-append flow keeps receiving the long URL.
  Everywhere, a failed PUT degrades to the long URL with a warning:
  the long form is fully functional.
- **Claiming**: browsers need nothing. `tonk join` detects `/@/`
  links, resolves the `Location` by hand with redirects disabled, and
  splices the fragment back on
  (`tonk_invite::shortcut::resolve_location`).

## Follow-ups

- **Shorten share links**: `share_concept`/`share_display`/`share_view`
  append `name=`/`then=` to the minted URL *after* composing; those
  parameters must be inside the stored target (query params on the
  short link itself are dropped by the redirect — only the fragment
  inherits). Shorten as the final composition step in `share.rs`.
- **`run_invite`/share-view path**: the in-app share view assembles
  the long URL from facts in a YAML template; shortening there means
  the handler PUTs and stores the hash fact instead. Same reseed
  constraints as any seeded-command change.
- **Ops**: a Cloudflare rate-limit rule on `PUT /@` (the only abuse
  lever a permissionless store leaves open is write volume), and a
  lifecycle rule deleting `tonk/link/`-prefixed objects 21 days
  after upload — R2 has no per-object TTL and rules are day-granular
  with lazy (≤ ~24 h) deletion, so expiry is enforced logically at
  read and the rule (one day past `MAX_TTL_DAYS`) does the physical
  cleanup.
- **Share-link lifetimes**: shortcuts now expire (≤ 20 days), so a
  shortened share link pasted into a document eventually dies;
  re-publishing the same target is idempotent (same hash) and
  refreshes the expiry. If durable share links are ever needed,
  that's a deliberate `ttl` policy question, not a format change.
