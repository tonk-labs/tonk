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
If both peers can choose all three up front, each can construct the
other's SDP locally and skip the round trip entirely.

They can. A browser **does** accept a local offer whose `ice-ufrag` and
`ice-pwd` have been rewritten, and uses them on the wire — measured in
Chromium, and what `libp2p-webrtc-websys` ships. See "Dialling with no
answer at all" below; that is the design to build, and it needs neither
of the two fact shapes described next.

The offer/answer path is still needed where direct dialling cannot
reach — across NAT, where the CLI has no UDP address a browser can send
to. There, two kinds of fact:

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

## Dialling with no answer at all

Yes, and it is the design to build. Measured, not reasoned about — see
`rust/tonk-rtc/examples/direct_dial.rs`, which is kept so the findings
can be re-checked against a later `webrtc` release.

The idea is libp2p's WebRTC-Direct: browsers refuse to let you munge
your own local SDP, but `setRemoteDescription` accepts any well-formed
SDP. So a browser can synthesise the CLI's half of the handshake from a
published address record, build its own offer locally, and dial — with
nothing travelling back. For that the CLI must stop needing the three
things an answer would carry.

What the measurements found, on `webrtc` 0.20.5:

| The CLI would need | Verdict |
| --- | --- |
| The peer's DTLS fingerprint | **Not needed.** `disable_certificate_fingerprint_verification(true)` works. The browser still checks the CLI against the published certhash, so the connection is one-way authenticated. |
| The peer's ICE candidates | **Not needed.** A peer-reflexive candidate is formed from the source address of the first check. |
| The peer's `ice-ufrag` | **Needed.** Username validation checks both halves; a placeholder is rejected outright. |
| The peer's `ice-pwd` | **Needed — because `set_lite(true)` does not suppress outgoing checks.** Despite lite mode the agent pings, signing with the remote password; a wrong one draws a `401` and the pair never validates. This contradicts the setting's own docs and is worth reporting upstream. |

Two supporting facts. Chromium dials a `127.0.0.1` remote candidate
without complaint, from `http` and `https` origins alike — that doubt
turned out to be nothing. And there is no UDP mux
(`SettingEngine::set_udp_network` is commented out as `/*todo:*/`), so
libp2p's trick of reading the peer's ufrag off the first STUN packet
before building a peer connection is not available without upstream
work.

Those last two rows say the CLI needs the peer's ICE credentials. It
does — but it does not need to *receive* them, because **the dialer
chooses them for both sides**.

`libp2p-webrtc-websys` munges its own `createOffer` output, replacing
`a=ice-ufrag` and `a=ice-pwd` with one random string, and synthesises
the peer's SDP with that same string in both fields. Measured in
Chromium: `setLocalDescription` accepts the rewrite, `localDescription`
reflects it, and the STUN `USERNAME` on the wire reads
`<ufrag>:<ufrag>`. One shared secret, symmetric, nothing to exchange.

The listener then reads that ufrag out of the first STUN packet and
builds a peer connection with `set_ice_credentials(ufrag, ufrag)`. That
needs a UDP mux — which the **0.17 line of the same `webrtc` crate
has** (`SettingEngine::set_udp_network(UDPNetwork::Muxed(..))`, a live
function there, and what `libp2p-webrtc` is built on). Its absence in
0.20.5 is a regression in the rewrite, not a property of the ecosystem.

So the address record collapses to what the CLI alone knows:

```
peer/<did>  dialable  { host, port, certhash }   ← CLI, on start
```

Zero round trips. No callback, no iframe, no popup, no button, and
nothing for the dialer to publish.

### What it costs

- Move to `webrtc` 0.17 and port `peer.rs` to its callback API.
- A UDP mux keyed on the STUN `USERNAME` (libp2p's is ~350 lines and
  `pub(crate)`, so it is a reimplementation, not a dependency).
- A persisted certificate, so the published certhash survives a restart.
- An application-layer handshake. DTLS is one-way authenticated here —
  the dialer verifies the CLI against the published certhash, the CLI
  verifies nothing — so possession of the address becomes the
  capability unless something on top checks the peer. libp2p uses Noise;
  tonk has DIDs and UCAN delegations already.

### The risk that decides it

**Safari is unverified, and Safari is why this project exists.** The
munge was measured in Chromium only; libp2p documents browser
WebRTC-Direct for Chrome and Firefox. If WebKit rejects a rewritten
local `ice-ufrag`, this path dies for exactly the browser it was meant
to serve, and the offer/answer-over-the-space design is the fallback.
Measuring that needs a Mac and about ten minutes.

It also does not survive NAT — the CLI must be reachable by UDP at the
published address, and the dialer cannot help it hole-punch without a
round trip. Same machine and same LAN, yes; anything wider needs the
offer/answer path as a fallback. One address record, two dial methods.


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
