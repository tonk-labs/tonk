// The browser half of `tonk rtc connect`.
//
// This route deliberately has no Wasm, custom-element, or service-worker
// dependency — the same premise as `doctor.mjs`. `RTCPeerConnection`
// is `[Exposed=Window]` and does NOT exist in a service worker, so the
// peer connection lives in a page. The ordinary app uses these same
// dial/relay primitives through rtc-carrier.mjs: Dialog's signed effects
// and iroh remain in the worker, connected by a private MessagePort.
//
// The ceremony:
//
//   1. `tonk` opens /rtc#offer=<b64>&callback=<loopback url>
//   2. this page answers the offer
//   3. it delivers the answer to that loopback listener
//   4. `tonk` sets the remote description; the channel opens
//
// Step 3 is the interesting one, and it has two paths — see the
// delivery note in `mountRtc`. What rules out the obvious approaches:
// a `fetch` to `http://127.0.0.1` from an `https` page is mixed content
// in Safari and gated by Local Network Access in Chrome, and navigating
// THIS document away would destroy the peer connection it just
// negotiated. So delivery happens in some other browsing context.

const VERSION = 1;
const CHANNEL_LABEL = "tonk";

const encoder = new TextEncoder();
const decoder = new TextDecoder();

function fromBase64Url(text) {
    const padded = text.replaceAll("-", "+").replaceAll("_", "/")
        .padEnd(text.length + ((4 - (text.length % 4)) % 4), "=");
    const binary = atob(padded);
    return decoder.decode(Uint8Array.from(binary, (c) => c.charCodeAt(0)));
}

function toBase64Url(text) {
    const bytes = encoder.encode(text);
    let binary = "";
    for (const byte of bytes) binary += String.fromCharCode(byte);
    return btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
}

/** Read a session description envelope, refusing anything that is not the expected half. */
export function decodeDescription(encoded, expectedRole) {
    const envelope = JSON.parse(fromBase64Url(encoded));
    if (envelope.version !== VERSION) {
        throw new Error(
            `this link carries a version ${envelope.version} description; this page speaks version ${VERSION}`,
        );
    }
    if (envelope.role !== expectedRole) {
        throw new Error(`expected an ${expectedRole} but the link carries an ${envelope.role}`);
    }
    if (typeof envelope.sdp !== "string" || envelope.sdp === "") {
        throw new Error("the description carries no SDP");
    }
    return envelope.sdp;
}

/** Wrap an SDP for the trip back to the terminal. */
export function encodeDescription(role, sdp) {
    return toBase64Url(JSON.stringify({ version: VERSION, role, sdp }));
}

// The answer may only be delivered to a loopback listener. The offer
// arrives in a URL fragment, so anyone who can get you to open a link
// chooses this value; without the check the page would re-post its
// answer to any origin an attacker named.
//
// This is the narrow version of a question that gets sharper as these
// channels carry more: a fragment-delivered offer is an unauthenticated
// request to open a channel INTO this browser. Fine for a chat echo.
// Not fine once the channel speaks dialog effects.
export function isLoopback(url) {
    try {
        const parsed = new URL(url);
        return parsed.protocol === "http:"
            && (parsed.hostname === "127.0.0.1" || parsed.hostname === "localhost"
                || parsed.hostname === "[::1]");
    } catch {
        return false;
    }
}

// How long to wait for ICE gathering before shipping what we have.
//
// This ceremony sends one description and then closes the signalling
// channel, so candidates gathered after this point are lost — hence the
// wait. But waiting UNBOUNDED is a hang: `iceGatheringState` is not
// guaranteed to reach "complete" promptly (a slow or unreachable STUN
// server, an odd network stack, a container with no multicast), and the
// SDP already carries every candidate found so far. Ship those.
const GATHER_MS = 3000;

/** Resolve once ICE gathering finishes, or once GATHER_MS has passed. */
function gathered(connection) {
    if (connection.iceGatheringState === "complete") return Promise.resolve();
    return new Promise((resolve) => {
        const done = () => {
            clearTimeout(timer);
            connection.removeEventListener("icegatheringstatechange", check);
            connection.removeEventListener("icecandidate", candidate);
            resolve();
        };
        const timer = setTimeout(done, GATHER_MS);
        const check = () => {
            if (connection.iceGatheringState === "complete") done();
        };
        // The state-change event can be missed if gathering completes
        // between the read above and this listener attaching; a null
        // candidate is the other end-of-gathering signal.
        const candidate = (event) => {
            if (!event.candidate) done();
        };
        connection.addEventListener("icegatheringstatechange", check);
        connection.addEventListener("icecandidate", candidate);
    });
}

// ---- direct dial -------------------------------------------------
//
// The other direction, and the interesting one: the CLI publishes an
// address and this page dials it with NOTHING travelling back.
//
// An SDP answer carries a DTLS fingerprint, ICE credentials and
// candidates. All three are in the published address, so this page can
// build the CLI's half of the handshake itself. The piece that makes it
// work in a browser: `setLocalDescription` accepts a `createOffer`
// result whose `a=ice-ufrag` and `a=ice-pwd` have been rewritten, and
// the browser uses them on the wire (measured in Chromium; it is what
// `libp2p-webrtc-websys` ships). So both sides use ONE shared string as
// ufrag and password, and there is nothing left to exchange.
//
// This page picks the ICE credential itself, fresh per dial, which is
// what lets one published address serve many dials — concurrently and
// after a reconnect. ICE separates peers by ufrag, so a fixed one in
// the address would mean exactly one connection ever.
//
// Caveat carried from the CLI side: DTLS ends up one-way authenticated.
// This page verifies the CLI against the published fingerprint; the CLI
// cannot verify this page, because a browser's certificate is minted
// per page load. Nothing at this layer decides who may connect — every
// invocation over the channel carries a signed UCAN and is verified
// before any work is done, so reaching the port grants nothing.

/** Read the address record the CLI published. */
export function decodeAddress(encoded) {
    if (typeof encoded !== "string" || encoded.length > 16384) throw new Error("invalid or oversized WebRTC address");
    const padded = encoded.replaceAll("-", "+").replaceAll("_", "/")
        .padEnd(encoded.length + ((4 - (encoded.length % 4)) % 4), "=");
    const address = JSON.parse(decoder.decode(
        Uint8Array.from(atob(padded), (c) => c.charCodeAt(0)),
    ));
    return validateAddress(address);
}

/** Validate at the SDP boundary as well as when reading encoded addresses. */
export function validateAddress(address) {
    if (!address?.fingerprint || !address.candidates?.length) {
        throw new Error("the address is missing candidates or a fingerprint");
    }
    if ((address.version ?? 1) !== 1) throw new Error(`unsupported WebRTC address version ${address.version}`);
    if (!/^sha-256 (?:[0-9a-f]{2}:){31}[0-9a-f]{2}$/i.test(address.fingerprint)) {
        throw new Error("invalid SHA-256 certificate fingerprint");
    }
    if (!Array.isArray(address.candidates) || address.candidates.length > 16) throw new Error("expected 1–16 candidates");
    for (const { host, port } of address.candidates) {
        const ipv4 = typeof host === "string" && /^(?:0|[1-9][0-9]{0,2})(?:\.(?:0|[1-9][0-9]{0,2})){3}$/.test(host)
            && host.split(".").every((part) => Number(part) <= 255);
        let ipv6 = false;
        if (typeof host === "string" && /^[0-9a-f:.]+$/i.test(host) && host.includes(":")) {
            try { ipv6 = new URL(`http://[${host}]/`).hostname.startsWith("["); } catch { /* invalid literal */ }
        }
        if ((!ipv4 && !ipv6) || !Number.isInteger(port) || port < 1 || port > 65535) {
            throw new Error("candidate requires an IP literal and a nonzero UDP port");
        }
    }
    return address;
}

/**
 * The label of a channel carrying iroh datagrams rather than text.
 * Mirrors `tonk_rtc::peer::DATAGRAM_LABEL`.
 */
export const DATAGRAM_LABEL = "tonk-iroh";

/**
 * Channel options for that label.
 *
 * Unreliable and unordered, and this is not an optimization: QUIC
 * supplies its own reliability and ordering, and a channel that also
 * retransmits fights it — head-of-line blocking appears in a protocol
 * designed to avoid it. Quiet networks hide it entirely; it shows up
 * under loss as latency that grows instead of recovering.
 */
export function datagramChannel() {
    return { label: DATAGRAM_LABEL, ordered: false, maxRetransmits: 0 };
}

/** The phrase both ends derive the rendezvous from. Mirrors `tonk_rtc::rendezvous::RENDEZVOUS`. */
export const RENDEZVOUS = "tonk/rtc/rendezvous/v1";

/** Where the derived certificate is served. */
export const RENDEZVOUS_CERT_URL = "/rendezvous.der";

/**
 * The port the phrase derives.
 *
 * SHA-256 is a WebCrypto primitive, so this needs nothing imported and
 * nothing shipped — it is the same arithmetic `tonk_rtc::rendezvous`
 * does, and a test pins the two against each other. The dynamic range
 * is 49152..=65535, so nothing here collides with a registered service.
 */
export async function rendezvousPort(phrase = RENDEZVOUS) {
    const digest = new Uint8Array(
        await crypto.subtle.digest("SHA-256", new TextEncoder().encode(`${phrase}#port`)),
    );
    return 49152 + (((digest[0] << 8) | digest[1]) % 16384);
}

/**
 * How many ports a rendezvous spans. Mirrors `rendezvous::SPAN`.
 *
 * One port per phrase would mean one listener per machine: a second
 * `tonk` finds it taken and fails. A span lets each program hold its own
 * port and stay findable, because knowing the phrase means knowing the
 * whole range.
 */
export const RENDEZVOUS_SPAN = 16;

/**
 * Every port a listener for `phrase` may have taken.
 *
 * Starts at `rendezvousPort` so the span agrees with the single-port
 * derivation by construction: the first listener on a machine takes that
 * slot, and a dialer trying it first usually stops there.
 */
export async function rendezvousPorts(phrase = RENDEZVOUS) {
    const base = await rendezvousPort(phrase);
    const ports = [];
    for (let i = 0; i < RENDEZVOUS_SPAN && base + i <= 65535; i += 1) ports.push(base + i);
    return ports;
}

/**
 * The fingerprint of a DER-encoded certificate, in SDP form.
 *
 * A fingerprint is the hash of a whole certificate, and a certificate
 * carries a signature — so deriving one here would mean an ASN.1
 * builder and an RFC 6979 signer, because WebCrypto's own ECDSA signs
 * with a random nonce and could never reproduce the bytes. Hashing a
 * certificate the origin already serves is one call and no library.
 */
export async function rendezvousFingerprint(der) {
    const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", der));
    const octets = [...digest].map((b) => b.toString(16).padStart(2, "0").toUpperCase());
    return `sha-256 ${octets.join(":")}`;
}

/**
 * The address of a `tonk` listening on this machine.
 *
 * Nothing is exchanged with the listener to get here: the candidate is
 * loopback, the port is a hash of a published phrase, and the
 * fingerprint is a hash of a certificate this origin serves.
 */
export async function localAddress(url = RENDEZVOUS_CERT_URL) {
    const response = await fetch(url);
    if (!response.ok) throw new Error(`could not read ${url}: ${response.status}`);

    const [ports, fingerprint] = await Promise.all([
        rendezvousPorts(),
        rendezvousFingerprint(new Uint8Array(await response.arrayBuffer())),
    ]);
    // Every port in the span, as candidates. ICE already races a
    // candidate list and keeps the pair that answers, so the scan is
    // the connectivity check it would run anyway rather than a loop
    // this code has to write — and a listener on any slot is found in
    // one dial instead of sixteen.
    return {
        candidates: ports.map((port) => ({ host: "127.0.0.1", port })),
        fingerprint,
    };
}

/**
 * A fresh ICE credential for one dial.
 *
 * Used as this page's own ufrag AND password, and as both of the CLI's
 * in the description synthesized below — one value, four fields. That
 * symmetry is what removes the round trip: the CLI reads this off the
 * first STUN packet and needs nothing else.
 */
export function freshCredential() {
    const bytes = crypto.getRandomValues(new Uint8Array(24));
    let binary = "";
    for (const byte of bytes) binary += String.fromCharCode(byte);
    // ICE uses the base64 alphabet, not base64url. Twenty-four bytes yield
    // exactly 32 characters with no padding (192 bits of entropy).
    return btoa(binary);
}

/** Build the CLI's side of the handshake from its published address. */
export function synthesizeAnswer(address, credential) {
    validateAddress(address);
    if (!/^[A-Za-z0-9_+/-]{4,256}$/.test(credential)) throw new Error("invalid ICE credential");
    // Firefox's SDP parser requires uppercase fingerprint octets. Native
    // certificates may advertise lowercase; normalize only the SDP spelling,
    // not the saved route record or the certificate bytes it authenticates.
    const fingerprint = `sha-256 ${address.fingerprint.slice(8).toUpperCase()}`;
    const [first] = address.candidates;
    const family = first.host.includes(":") ? "IP6" : "IP4";
    const candidates = address.candidates
        .map((c, index) => `a=candidate:${index + 1} 1 udp 2130706431 ${c.host} ${c.port} typ host`)
        .join("\r\n");
    // `a=setup:active` is hard-coded because the CLI pins its answering
    // DTLS role, so it does not have to travel in the address.
    return `v=0\r\n`
        + `o=- 0 0 IN ${family} ${first.host}\r\n`
        + `s=-\r\nt=0 0\r\n`
        + `a=fingerprint:${fingerprint}\r\n`
        + `a=group:BUNDLE 0\r\n`
        + `m=application ${first.port} UDP/DTLS/SCTP webrtc-datachannel\r\n`
        + `c=IN ${family} ${first.host}\r\n`
        + `a=setup:active\r\na=mid:0\r\na=sendrecv\r\n`
        + `a=sctp-port:5000\r\na=max-message-size:65536\r\n`
        + `a=ice-ufrag:${credential}\r\n`
        + `a=ice-pwd:${credential}\r\n`
        + `${candidates}\r\na=end-of-candidates\r\n`;
}

/** Rewrite our own ICE credentials to the shared one. */
export function mungeOffer(sdp, credential) {
    return sdp
        .replace(/a=ice-ufrag:.*/g, `a=ice-ufrag:${credential}`)
        .replace(/a=ice-pwd:.*/g, `a=ice-pwd:${credential}`);
}

/**
 * Dial the CLI. Resolves with the open data channel.
 *
 * The deadline covers offer creation and SDP setup as well as ICE. Every
 * failure closes the peer connection; cancellation also releases listeners.
 */
export async function dial(address, credential = freshCredential(), channelInit = {}, {
    signal, timeoutMs = 15000, PeerConnection = globalThis.RTCPeerConnection,
} = {}) {
    validateAddress(address);
    signal?.throwIfAborted();
    const connection = new PeerConnection({ iceServers: [] });
    const { label = CHANNEL_LABEL, ...init } = channelInit;
    let channel;
    try {
        channel = connection.createDataChannel(label, init);
        await new Promise((resolve, reject) => {
            let finished = false;
            const done = (error) => {
                if (finished) return;
                finished = true;
                clearTimeout(timer);
                channel.removeEventListener("open", opened);
                channel.removeEventListener("close", closed);
                connection.removeEventListener("connectionstatechange", changed);
                signal?.removeEventListener("abort", aborted);
                error ? reject(error) : resolve();
            };
            const opened = () => done();
            const closed = () => done(new Error("the data channel closed before opening"));
            const aborted = () => done(signal.reason ?? new Error("dial cancelled"));
            const changed = () => {
                if (["failed", "closed"].includes(connection.connectionState)) {
                    done(new Error(`the connection went to "${connection.connectionState}"`));
                }
            };
            const timer = setTimeout(() => done(new Error("the CLI did not answer within the dial deadline")), timeoutMs);
            channel.addEventListener("open", opened);
            channel.addEventListener("close", closed);
            connection.addEventListener("connectionstatechange", changed);
            signal?.addEventListener("abort", aborted, { once: true });
            if (signal?.aborted) { aborted(); return; }
            (async () => {
                const offer = await connection.createOffer();
                if (finished) return;
                await connection.setLocalDescription({ type: "offer", sdp: mungeOffer(offer.sdp, credential) });
                if (finished) return;
                await connection.setRemoteDescription({ type: "answer", sdp: synthesizeAnswer(address, credential) });
                if (!finished && channel.readyState === "open") opened();
            })().catch(done);
        });
        return { connection, channel };
    } catch (error) {
        channel?.close();
        connection.close();
        throw error;
    }
}

// The palette restated as literals, the way the CLI's own callback
// confirmation page does: a bare route loads no app stylesheet, so there
// is nothing to derive tokens from.
const STYLE = `
:root { color-scheme: light dark; --page:#e8e6e4; --ink:#38182a; --on-ink:#f7f6f5;
        --soft:#5b4953; --frost:#f7f6f5; --ring:rgb(56 24 42 / 85%); }
@media (prefers-color-scheme: dark) {
  :root { --page:#161313; --ink:#e2dfdd; --on-ink:#221c1d; --soft:#c8c3bf;
          --frost:#1b1718; --ring:rgb(226 223 221 / 55%); }
}
body { margin:0; background:var(--page); color:var(--ink);
       font:13.5px/1.5 "IBM Plex Sans", system-ui, sans-serif; }
.rtc { max-width:640px; margin:0 auto; padding:48px 16px 64px;
       display:flex; flex-direction:column; gap:12px; }
.rtc h1 { font:600 13px/1 "IBM Plex Sans Condensed", system-ui, sans-serif;
          letter-spacing:.02em; text-transform:lowercase; margin:0; }
.rtc__status { color:var(--soft); margin:0; }
.rtc__go, .rtc__compose button { height:36px; padding:0 14px; border:0; cursor:pointer;
       background:var(--ink); color:var(--on-ink); text-transform:lowercase;
       font:600 13px/1 "IBM Plex Sans Condensed", system-ui, sans-serif; }
.rtc__go[disabled] { opacity:.5; cursor:default; }
.rtc__go { align-self:flex-start; }
.rtc__log { list-style:none; margin:0; padding:8px; min-height:160px; max-height:50vh;
       overflow-y:auto; background:var(--frost); box-shadow:0 0 0 1px var(--ring);
       font:12px/1.6 "IBM Plex Mono", ui-monospace, monospace; }
.rtc__line::before { color:var(--soft); }
.rtc__line--me::before { content:"you  "; }
.rtc__line--them::before { content:"tonk "; }
.rtc__compose { display:flex; gap:1px; }
.rtc__compose input { flex:1; height:36px; padding:0 10px; border:0; min-width:0;
       background:var(--frost); color:var(--ink); box-shadow:0 0 0 1px var(--ring);
       font:13.5px/1 "IBM Plex Sans", system-ui, sans-serif; }
`;

export async function mountRtc(root = document.body) {
    if (!document.querySelector("#tonk-rtc-styles")) {
        const style = document.createElement("style");
        style.id = "tonk-rtc-styles";
        style.textContent = STYLE;
        document.head.appendChild(style);
    }

    root.innerHTML = `
<main class="rtc">
  <h1>tonk rtc</h1>
  <p class="rtc__status" id="status">reading the offer…</p>
  <button class="rtc__go" id="go" hidden>connect</button>
  <ol class="rtc__log" id="log"></ol>
  <form class="rtc__compose" id="compose" hidden>
    <input id="message" autocomplete="off" placeholder="type a message for the terminal" />
    <button type="submit">send</button>
  </form>
</main>`;

    const status = root.querySelector("#status");
    const log = root.querySelector("#log");
    const go = root.querySelector("#go");
    const compose = root.querySelector("#compose");
    const message = root.querySelector("#message");

    const say = (text) => { status.textContent = text; };
    const append = (who, text) => {
        const line = document.createElement("li");
        line.className = `rtc__line rtc__line--${who}`;
        line.textContent = text;
        log.appendChild(line);
        log.scrollTop = log.scrollHeight;
    };

    const fields = new URLSearchParams(window.location.hash.slice(1));
    // Strip the offer from history before anything else: it should not
    // survive in the back stack or leak through a shared URL.
    history.replaceState(null, "", window.location.pathname + window.location.search);

    const offerField = fields.get("offer");
    const callback = fields.get("callback");
    const addressField = fields.get("address");

    // The direct-dial form. Nothing goes back to the CLI, so there is
    // no callback, no iframe, no popup and no button.
    if (addressField) {
        let address;
        try {
            address = decodeAddress(addressField);
        } catch (error) {
            say(`this link is not a usable address: ${error.message}`);
            return;
        }
        say("dialing…");
        try {
            const { channel } = await dial(address);
            channel.addEventListener("message", (event) => append("them", event.data));
            channel.addEventListener("close", () => say("the terminal closed the channel."));
            say("connected. type below; it prints in the terminal.");
            compose.hidden = false;
            message.focus();
            compose.addEventListener("submit", (event) => {
                event.preventDefault();
                const text = message.value.trim();
                if (!text || channel.readyState !== "open") return;
                channel.send(text);
                append("me", text);
                message.value = "";
            });
        } catch (error) {
            say(`could not reach the terminal: ${error.message}`);
        }
        return;
    }

    if (!offerField || !callback) {
        say("open this page from `tonk rtc connect` — it carries the offer.");
        return;
    }
    if (!isLoopback(callback)) {
        say("this link asks to deliver its answer somewhere other than your own machine. refusing.");
        return;
    }

    let offer;
    try {
        offer = decodeDescription(offerField, "offer");
    } catch (error) {
        say(`this link is not a usable offer: ${error.message}`);
        return;
    }

    // Delivery has two paths, tried in that order.
    //
    // An IFRAME form POST to the loopback listener needs no user
    // gesture, so the common case has no button at all. It is a
    // nested-context navigation, which faces two gates: mixed content
    // (loopback is "potentially trustworthy" per Secure Contexts, so it
    // should be exempt — Chrome and Firefox agree; WebKit is stricter
    // and unverified) and Chrome's Local Network Access (which covers
    // nested-context navigations; it did not fire when this was
    // measured, but it is being rolled out).
    //
    // A POPUP is a TOP-LEVEL navigation, exempt from both, and the same
    // mechanism `tonk account login` already relies on. It costs a user
    // gesture, hence a button.
    //
    // The page cannot read a cross-origin iframe, so it cannot observe
    // the iframe path failing directly — but it does not need to. No
    // data channel within FALLBACK_MS means the answer did not arrive.
    let session = await negotiate(offer, { say, append, compose, message });
    const discardIframe = deliverByIframe(callback, session.sdp);
    say("answering…");

    if (await session.opened(FALLBACK_MS)) return;

    // The iframe was blocked, or the terminal went away. Offer the path
    // that needs a click. A fresh connection, because the first one has
    // been running ICE against a peer that never answered.
    discardIframe();
    session.close();
    say("your browser would not deliver the answer on its own. press connect.");
    go.hidden = false;
    go.addEventListener("click", () => {
        go.disabled = true;
        // Opened SYNCHRONOUSLY: a popup opened after an `await` has lost
        // the click's transient activation and browsers block it.
        const popup = window.open("", "tonk-rtc-answer", "width=420,height=260");
        if (!popup) {
            say("your browser blocked the popup. allow popups for this site and press connect again.");
            go.disabled = false;
            return;
        }
        negotiate(offer, { say, append, compose, message })
            .then((retry) => {
                say("returning the answer to your terminal…");
                // A popup navigation is a GET, so the answer rides in
                // the fragment and the listener's bridge page re-posts
                // it same-origin. Fragments are never sent over the
                // network, so no SDP reaches a log.
                popup.location = `${callback}#answer=${encodeDescription("answer", retry.sdp)}`;
            })
            .catch((error) => {
                popup.close();
                say(`connection failed: ${error.message}`);
                go.disabled = false;
            });
    });
}

// How long to wait for the iframe path before offering the popup.
const FALLBACK_MS = 6000;

// POST the answer straight to the loopback listener from a hidden
// iframe. No fragment and no bridge page: a form submission carries a
// body, which a popup's GET navigation cannot.
function deliverByIframe(callback, sdp) {
    const sink = document.createElement("iframe");
    sink.hidden = true;
    sink.name = "tonk-rtc-sink";
    document.body.appendChild(sink);

    const form = document.createElement("form");
    form.method = "post";
    form.action = callback;
    form.target = sink.name;
    form.hidden = true;
    const field = document.createElement("input");
    field.type = "hidden";
    field.name = "answer";
    field.value = encodeDescription("answer", sdp);
    form.appendChild(field);
    document.body.appendChild(form);
    form.submit();

    // The listener serves one answer and shuts down, so nothing else is
    // coming through here.
    return () => {
        sink.remove();
        form.remove();
    };
}

// Answer the offer and wire the channel up for when it arrives.
//
// Returns the answer SDP to deliver, a bounded wait for the channel
// opening, and a teardown — the caller may have to abandon this attempt
// and start another one down a different delivery path.
async function negotiate(offer, ui) {
    const { say, append, compose, message } = ui;

    // No ICE servers: the CLI is on this machine, so host candidates
    // pair up directly. Add STUN here when the peers move apart.
    const connection = new RTCPeerConnection({ iceServers: [] });

    // The CLI creates the channel; this side adopts it. Registering the
    // listener before `setRemoteDescription` means the event cannot be
    // missed.
    const channelReady = new Promise((resolve, reject) => {
        connection.addEventListener("datachannel", (event) => {
            if (event.channel.label === CHANNEL_LABEL) resolve(event.channel);
        });
        connection.addEventListener("connectionstatechange", () => {
            if (["failed", "closed"].includes(connection.connectionState)) {
                reject(new Error(`the connection went to "${connection.connectionState}"`));
            }
        });
    });

    say("answering…");
    await connection.setRemoteDescription({ type: "offer", sdp: offer });
    await connection.setLocalDescription(await connection.createAnswer());
    await gathered(connection);

    // The channel cannot arrive until the terminal has the answer, so
    // this runs alongside delivery rather than blocking it.
    channelReady.then(async (channel) => {
        channel.addEventListener("message", (event) => append("them", event.data));
        channel.addEventListener("close", () => say("the terminal closed the channel."));
        if (channel.readyState !== "open") {
            await new Promise((resolve) => channel.addEventListener("open", resolve, { once: true }));
        }
        say("connected. type below; it prints in the terminal.");
        compose.hidden = false;
        message.focus();
        compose.addEventListener("submit", (event) => {
            event.preventDefault();
            const text = message.value.trim();
            if (!text || channel.readyState !== "open") return;
            channel.send(text);
            append("me", text);
            message.value = "";
        });
    }).catch((error) => say(`connection failed: ${error.message}`));

    return {
        sdp: connection.localDescription.sdp,
        // Resolves true once the channel is open, false once `ms`
        // has passed without it. Never rejects: a timeout here means
        // "try the other delivery path", not "something went wrong".
        opened: (ms) =>
            Promise.race([
                channelReady.then(() => true, () => false),
                new Promise((resolve) => setTimeout(() => resolve(false), ms)),
            ]),
        close: () => connection.close(),
    };
}

/**
 * Relay a data channel to a `MessagePort`, and back.
 *
 * The page half of the worker boundary. `RTCPeerConnection` is
 * `[Exposed=Window]`, so the connection lives here while the iroh
 * endpoint lives in the worker; this is the pipe, and it interprets
 * nothing.
 *
 * Every datagram crosses as a transferred `ArrayBuffer` — neutered
 * rather than cloned, because this carries far more traffic than an
 * application-level bridge and a structured clone per packet would put
 * a memcpy in the data path.
 *
 * Closing is explicit: a closed channel posts `null`, because a closed
 * port fires no event and the worker would otherwise hold a route to
 * nowhere.
 */
export function relay(channel, port, { heartbeatMs = 5000, leaseMs = 15000, now = Date.now } = {}) {
    channel.binaryType = "arraybuffer";
    let closed = false;
    let outstanding = 0;
    let lastPong = now();
    const post = (data, transfer = []) => {
        try { port.postMessage(data, transfer); } catch { close(); }
    };
    const incoming = (event) => {
        if (event.data instanceof ArrayBuffer && outstanding < 64) {
            outstanding += 1;
            post(event.data, [event.data]);
        }
    };
    const outgoing = (event) => {
        if (event.data === null) { close(); return; }
        if (event.data === "ack") { outstanding = Math.max(0, outstanding - 1); return; }
        if (event.data === "ping") { post("pong"); return; }
        if (event.data === "pong") { lastPong = now(); return; }
        if (event.data instanceof ArrayBuffer) {
            // Bound both the MessagePort flight window and the SCTP send
            // buffer. Congestion is datagram loss, never a growing queue.
            post("ack");
            if (channel.readyState === "open" && channel.bufferedAmount < 256 * 1024) {
                try { channel.send(event.data); } catch { /* packet loss */ }
            }
        }
    };
    const close = () => {
        if (closed) return;
        closed = true;
        clearInterval(heartbeat);
        channel.removeEventListener("message", incoming);
        channel.removeEventListener("close", close);
        channel.removeEventListener("error", close);
        port.removeEventListener("message", outgoing);
        try { port.postMessage(null); } catch { /* already gone */ }
        port.close();
        channel.close();
    };
    const heartbeat = setInterval(() => {
        if (now() - lastPong >= leaseMs) close();
        else post("ping");
    }, heartbeatMs);
    channel.addEventListener("message", incoming);
    channel.addEventListener("close", close);
    channel.addEventListener("error", close);
    port.addEventListener("message", outgoing);
    port.start();
    return close;
}

/** Normal app integration: only the controlling worker may request a dial.
 * Each request has its own reply port, so registration and its acknowledgment
 * cannot be confused with another tab, attempt, or worker generation.
 */
export function serveCarrierRequests(workers = navigator.serviceWorker, page = globalThis, {
    dialPeer = dial, Channels = MessageChannel,
} = {}) {
    const active = new Map();
    const stop = () => {
        for (const carrier of active.values()) carrier.close();
        active.clear();
    };
    const message = async (event) => {
        const request = event.data;
        if (event.source !== workers.controller || request?.v !== 1 || request.type !== "tonk-rtc-dial") return;
        const reply = event.ports?.[0];
        if (!reply) return;
        active.get(request.peer)?.close();
        if (active.size >= 4) {
            reply.postMessage({ v: 1, type: "error", detail: "this page already carries four peers" });
            reply.close();
            return;
        }
        const abort = new AbortController();
        let connection, dispose, timer, closed = false, offered = false;
        const carrier = { close() {
            if (closed) return;
            closed = true;
            abort.abort(new Error("carrier replaced or page closed"));
            clearTimeout(timer);
            dispose?.();
            connection?.close();
            reply.close();
            if (active.get(request.peer) === carrier) active.delete(request.peer);
        } };
        active.set(request.peer, carrier);
        // Listen before ICE starts so a worker deadline or superseded request
        // can cancel a dial that has not returned a data channel yet.
        reply.addEventListener("message", (answer) => {
            if (offered && answer.data?.v === 1 && answer.data.type === "ready") {
                clearTimeout(timer);
                reply.close();
            } else carrier.close();
        });
        reply.start();
        try {
            const opened = await dialPeer(validateAddress(request.address), freshCredential(), datagramChannel(), { signal: abort.signal });
            connection = opened.connection;
            if (abort.signal.aborted) {
                // Cancellation may have run just after dial resolved,
                // before this continuation acquired its handles.
                opened.channel.close();
                connection.close();
                return;
            }
            const { port1, port2 } = new Channels();
            dispose = relay(opened.channel, port1);
            opened.channel.addEventListener("close", () => carrier.close(), { once: true });
            connection.addEventListener("connectionstatechange", () => {
                if (["failed", "closed", "disconnected"].includes(connection.connectionState)) carrier.close();
            });
            timer = setTimeout(() => carrier.close(), 5000);
            offered = true;
            reply.postMessage({ v: 1, type: "carrier" }, [port2]);
        } catch (error) {
            if (!abort.signal.aborted) {
                try { reply.postMessage({ v: 1, type: "error", detail: error.message }); } catch { /* worker gone */ }
            }
            carrier.close();
        }
    };
    workers.addEventListener("message", message);
    workers.addEventListener("controllerchange", stop);
    page.addEventListener("pagehide", stop);
    return () => {
        stop();
        workers.removeEventListener("message", message);
        workers.removeEventListener("controllerchange", stop);
        page.removeEventListener("pagehide", stop);
    };
}
