# A WebRTC data channel between `tonk` and a browser tab

Status: proof of concept, landed behind a feature flag. Not wired to sync.

## What exists

`tonk rtc connect` (built with `--features rtc`) negotiates a WebRTC data
channel with a browser tab and relays lines of text in both directions.
Type in the terminal, it appears in the page; type in the page, it
appears in the terminal. That is the whole behaviour — the point is the
channel, not what rides on it.

Three pieces:

| Where | What |
| --- | --- |
| `rust/tonk-rtc/src/signal.rs` | The session-description envelope both halves agree on, and its URL-safe encoding. Target-agnostic, plain serde data. |
| `rust/tonk-rtc/src/peer.rs` | The native peer (webrtc-rs 0.20). Offers, then talks. |
| `rust/tonk-rtc/src/loopback.rs` | Same-machine signalling: a loopback listener the browser navigates back to. |
| `rust/tonk-ui/assets/rtc.mjs` | The browser half, on the bare `/rtc` route. |
| `rust/tonk-cli/src/rtc.rs` | The `tonk rtc connect` command and the stdin/stdout relay. |

## Why WebRTC and not the loopback server the CLI already has

Because Safari will not let a page on `https://tonk.network` open a
`fetch` or a WebSocket to `http://127.0.0.1`: that is mixed content, and
now a Local Network Access prompt besides. A *navigation* to loopback is
still permitted — which is exactly why the account-login callback works
— but a navigation is one-shot and destroys the page that made it.

So for a live, bidirectional channel between a browser page and a local
process, WebRTC is not an optimisation. It is the only portable option.
NAT traversal, the usual reason to reach for it, is a bonus that matters
later.

## The ceremony

```
  tonk                                        browser
  ----                                        -------
  bind loopback listener
  create offer, gather ICE
  open <page>/rtc#offer=…&callback=…  ─────>  answer the offer
  receive answer on loopback          <─────  POST it from a hidden iframe
  set remote description
  ═════════════════ data channel ═════════════════
```

The offer travels in a URL **fragment**, which is never sent over the
network, so no SDP reaches a server log.

### Delivering the answer is the whole problem

The page must reach a listener on `http://127.0.0.1` while STAYING
ALIVE — it owns the `RTCPeerConnection`, so navigating it away destroys
the channel it just negotiated. That rules out the obvious approaches:

| | |
| --- | --- |
| `fetch` / `sendBeacon` to loopback | Mixed content in Safari; Local Network Access in Chrome. This is the reason the whole feature is WebRTC and not a local HTTP server. |
| Navigate this document | Destroys the peer connection. |
| **Hidden iframe form POST** | A nested-context navigation. Needs no user gesture and carries a body, so no fragment and no bridge page. Faces mixed content (loopback is "potentially trustworthy" per Secure Contexts, so it should be exempt) and Chrome's Local Network Access (which does cover nested-context navigations). |
| **Popup** | A top-level navigation, exempt from both, and the same mechanism `tonk account login` already relies on. Costs a user gesture, hence a button. A popup navigation is a GET, so the answer rides in the fragment and the listener's bridge page re-posts it same-origin. |

The page tries the **iframe first** and falls back to the popup. It
cannot read a cross-origin iframe, so it cannot observe that path
failing — but it does not need to: no data channel within six seconds
means the answer did not arrive.

Measured in Chromium from a genuine secure context (`isSecureContext ==
true`, self-signed cert): both `iframe.src` and a form POST targeting an
iframe reached the loopback listener, with no mixed-content or Local
Network Access complaint. **Safari is unverified** — there is no Safari
on the machine this was built on, and WebKit is historically stricter
about `http://localhost` than the spec requires. The fallback exists so
that answer does not have to be known to ship.

A popup opened after an `await` has lost the click's transient
activation and browsers block it, so the fallback path opens
`about:blank` synchronously in the handler and points it at the callback
once the answer is ready.

## Constraints discovered along the way

**`RTCPeerConnection` is `[Exposed=Window]`.** It does not exist in a
service worker. Tonk's replica and sync engine live in a service worker
(`tonk-worker`), so a WebRTC sync transport can never be a drop-in
sibling of the S3 one: the peer connection must live in the page, and
every invocation has to cross a page↔worker `MessagePort`. That hop —
not the transport — is the hard part of "add the CLI as a remote".
`tonk-worker/src/router/bridge.rs` already implements that shape for the
sealed iframe, and is the thing to generalise.

The channel's lifetime problem comes with it: the service worker
outlives any page, but a channel owned by a tab dies with the tab.

**`PeerConnectionBuilder` discards mDNS candidates by default.**
`webrtc`'s builder defaults to `MulticastDnsMode::Disabled`, which drops
*remote* mDNS candidates — and Chrome and Safari emit only mDNS host
candidates (`<uuid>.local`) by default, to avoid leaking private IPs to
a page. Against that default a browser's answer arrives with every
candidate dropped and no connection can form, on any machine. The peer
therefore passes a `SettingEngine::default()`, whose own default is
`QueryOnly`. The consequence to remember: a same-machine connection
depends on mDNS resolution working on the host. It does on an ordinary
desktop; it does not inside a container with no multicast.

**ICE is not trickled.** The signalling channel here carries one message
each way and then closes, so the peer waits for gathering to complete
and ships a single description with every candidate in it. On loopback
that wait is milliseconds. This is a property of the *channel*, not of
the peer — a channel that stays open makes `on_ice_candidate` the thing
to implement and the gather-wait the thing to delete.

**The remote surface is small.** Dialog's whole remote protocol is five
effects — `archive::{Get,Put,Import}` and `memory::{Resolve,Publish}` —
dispatched through `dialog_network::Network`, a `#[derive(Site)]` table
of `{ s3, ucan, fs }`. Adding an `rtc` field there is the shape of "CLI
as a remote". Small surface; the difficulty is all in the page↔worker
hop above.

## What does not survive contact with a second machine

Loopback signalling. It works only because the browser and the CLI share
a host, and no amount of care changes that.

The intended replacement is **signalling over the space itself**:
descriptions written as facts, read by whichever peer is listening. It
bootstraps over the existing remote, needs no new infrastructure, and
works between peers that have never shared a host. WebRTC then becomes
the direct fast path negotiated over the slow shared one.

Nothing in `tonk-cli/src/rtc.rs` reaches into how the bytes travel — it
depends on bind / hand out an offer / wait for an answer — so this is a
second implementation, not a rewrite.

### Shape

An SDP offer is not an address. It is one half of a stateful, pairwise
handshake carrying a DTLS fingerprint, ICE credentials and candidates.
If both peers could choose all three up front, each could construct the
other's SDP locally and skip the round trip entirely — `rtc`'s
`SettingEngine` supports exactly that (`set_ice_credentials`, documented
for "signalless WebRTC"). **But the browser API has no way to set
`ice-ufrag`/`ice-pwd`.** A certificate can be pre-generated
(`RTCPeerConnection.generateCertificate`), the ICE credentials cannot.
So with a browser as one peer, one round trip is the floor. Worth
re-verifying against current browsers before designing around it.

That argues for two kinds of fact rather than one:

- **Presence, cardinality one per peer** — `peer/<did> dialable
  { fingerprint, candidates, at }`. Superseding is exactly right: a new
  tab or a restarted CLI replaces the stale record. Note what is *not*
  here: ICE credentials. This says "I exist and here is how to reach my
  host", not "here is a connection".
- **Negotiation, per pair** — `session/<dialer>/<dialee>` with `offer`
  and `answer`, each cardinality one, written by the respective side.
  Superseding the offer *is* reconnect-or-ICE-restart, so "the current
  attempt wins" comes for free.

### Open questions

- **Presence is a lie by default.** A browser cannot be dialed
  passively; a tab has to be open to answer. A cardinality-one
  `dialable` fact from a tab that closed thirty seconds ago still reads
  as current. Needs a heartbeat or a TTL on `at`, and dialers must
  tolerate dialing a corpse and retrying when the record is superseded.
  This is the hard part, not the SDP plumbing.
- **Bootstrap latency.** Each connection costs a sync round trip through
  the S3 remote. Fine for establishing a long-lived channel; not fine
  for fast reconnects.
- **Browser candidates are mDNS-obfuscated.** A browser's `dialable`
  record carries `<uuid>.local`, which resolves only on the same LAN.
  Anything wider needs STUN in the record so there are `srflx`
  candidates too.
- **Who may offer.** Today a fragment-delivered offer is an
  unauthenticated request to open a channel into someone's browser.
  Harmless for a chat echo; not harmless once the channel speaks dialog
  effects. Signalling through the space fixes this incidentally — only
  peers who can already write the space can offer — which is a reason to
  get there before the payload grows.

Signalling through the space also removes the delivery problem above
entirely: no loopback hop means no iframe, no popup, no gesture and no
button.

## Running it

Against a dev server:

```sh
nix develop            # then, in one shell:
dev:web                # trunk on http://127.0.0.1:8080

cargo run -p tonk-cli --features rtc -- rtc connect \
  --via http://127.0.0.1:8080/rtc
```

Against production, `--via` defaults to `https://tonk.network/rtc`, so
`tonk rtc connect` is enough. `--no-open` prints the URL instead of
launching a browser.

## Tests

- `cargo test -p tonk-rtc` — envelope round-trips and rejections, the
  offer actually describing a data channel, host candidates gathering
  without STUN, and the loopback listener's full POST round trip.
- `cargo test -p tonk-cli --features rtc` — `--via` validation (scheme,
  credentials, a caller-supplied fragment that must not collide with
  ours).
- `node --test rust/tonk-ui/tests/rtc.test.mjs` — `/rtc` registering no
  service worker, envelope round-trips in the browser's encoding, and
  the loopback-only check on the callback URL.

The full ceremony was additionally driven end to end against headless
Chromium during development (real `RTCPeerConnection` against real
webrtc-rs, popup and all). That harness is not checked in: CI has no
browser runner for this crate, and standing one up is its own piece of
work.

## Cost

The `rtc` feature is off by default. `webrtc` pulls an ICE/DTLS/SCTP
stack that roughly doubles `tonk-cli`'s dependency graph, and a shipped
`tonk` has no use for it until the channel carries sync.
