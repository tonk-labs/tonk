// The browser half of `tonk rtc connect`.
//
// This route deliberately has no Wasm, custom-element, or service-worker
// dependency — the same premise as `doctor.mjs`. That is not only about
// iteration speed: `RTCPeerConnection` is `[Exposed=Window]` and does
// NOT exist in a service worker, so the peer connection has to live in
// the page no matter how this grows. When these channels eventually
// carry dialog's remote effects, the worker will have to reach them
// through a page<->worker MessagePort rather than owning them.
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
