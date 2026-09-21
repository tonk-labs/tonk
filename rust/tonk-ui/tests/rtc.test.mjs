import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import {
    decodeAddress,
    decodeDescription,
    encodeDescription,
    freshCredential,
    isLoopback,
    RENDEZVOUS,
    localAddress,
    rendezvousFingerprint,
    rendezvousPort,
    rendezvousPorts,
    RENDEZVOUS_SPAN,
    mungeOffer,
    synthesizeAnswer,
    validateAddress,
    dial,
    relay,
    serveCarrierRequests,
} from "../assets/rtc.mjs";

const html = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const scripts = [...html.matchAll(/<script(?: type="module")?>([\s\S]*?)<\/script>/g)].map(match => match[1]);
const routeScript = scripts.find(script => script.includes("globalThis.tonkRtcRoute ="));
const workerScript = scripts.find(script => script.includes("const serviceWorkersSupported"));
const watchdogScript = scripts.find(script => script.includes("const RETRIES ="));

test("/rtc is a bare route: no worker registration, no boot recovery", () => {
    for (const path of ["/rtc", "/rtc/"]) {
        const context = { location: { pathname: path } };
        runInNewContext(routeScript, context);
        // navigator/document/timers deliberately absent: touching any is a failure.
        runInNewContext(workerScript, context);
        runInNewContext(watchdogScript, context);
        assert.equal(context.tonkRtcRoute, true);
        assert.equal(context.tonkBareRoute, true);
    }
    for (const path of ["/", "/rtcish", "/rtc/nested"]) {
        const context = { location: { pathname: path } };
        runInNewContext(routeScript, context);
        assert.equal(context.tonkRtcRoute, false);
    }
});

test("the doctor route keeps working after being folded into tonkBareRoute", () => {
    const context = { location: { pathname: "/doctor" } };
    runInNewContext(routeScript, context);
    assert.equal(context.tonkDoctorRoute, true);
    assert.equal(context.tonkBareRoute, true);
    assert.equal(context.tonkRtcRoute, false);
});

test("descriptions round-trip through the URL-safe envelope", () => {
    const sdp = "v=0\r\no=- 1 2 IN IP4 127.0.0.1\r\na=candidate:1 1 udp 1 127.0.0.1 1 typ host\r\n";
    const encoded = encodeDescription("answer", sdp);
    assert.match(encoded, /^[A-Za-z0-9_-]+$/, "the encoding must survive a URL fragment unescaped");
    assert.equal(decodeDescription(encoded, "answer"), sdp);
});

test("the wrong half, the wrong version and an empty SDP are all refused", () => {
    const offer = encodeDescription("offer", "v=0\r\n");
    assert.throws(() => decodeDescription(offer, "answer"), /expected an answer/);

    const future = Buffer.from(JSON.stringify({ version: 99, role: "offer", sdp: "v=0" }))
        .toString("base64url");
    assert.throws(() => decodeDescription(future, "offer"), /version 99/);

    const empty = Buffer.from(JSON.stringify({ version: 1, role: "offer", sdp: "" }))
        .toString("base64url");
    assert.throws(() => decodeDescription(empty, "offer"), /no SDP/);
});

// The offer arrives in a URL fragment, so whoever sends the link chooses
// the callback. Without this the page would re-post its answer wherever
// it was told to.
test("the answer may only be delivered to a loopback listener", () => {
    for (const good of ["http://127.0.0.1:54321", "http://localhost:8080/", "http://[::1]:9000"]) {
        assert.ok(isLoopback(good), `rejected ${good}`);
    }
    for (const bad of [
        "https://evil.test/collect",
        "http://evil.test/collect",
        "http://127.0.0.1.evil.test/",
        "javascript:alert(1)",
        "not a url",
        "",
    ]) {
        assert.ok(!isLoopback(bad), `accepted ${bad}`);
    }
});

const address = {
    candidates: [{ host: "127.0.0.1", port: 46660 }, { host: "192.0.2.2", port: 41445 }],
    fingerprint: "sha-256 " + Array(32).fill("AB").join(":"),
};
const CREDENTIAL = "c2hhcmVkLWNyZWRlbnRpYWwtdmFsdWU";
const encodeAddress = (value) => Buffer.from(JSON.stringify(value)).toString("base64url");

test("an address round-trips and is rejected when incomplete", () => {
    assert.deepEqual(decodeAddress(encodeAddress(address)), address);
    for (const missing of ["candidates", "fingerprint"]) {
        const broken = { ...address, [missing]: missing === "candidates" ? [] : undefined };
        assert.throws(() => decodeAddress(encodeAddress(broken)), /missing/);
    }
});

test("route validation rejects unsupported versions and SDP injection", () => {
    for (const bad of [
        { ...address, version: 2 },
        { ...address, fingerprint: `${address.fingerprint}\r\na=setup:passive` },
        { ...address, candidates: [{ host: "127.0.0.1\r\na=ice-pwd:bad", port: 1234 }] },
        { ...address, candidates: [{ host: "example.com", port: 1234 }] },
        { ...address, candidates: [{ host: "127.0.0.1", port: 65536 }] },
        { ...address, candidates: [{ host: "127.0.0.1", port: 0 }] },
        { ...address, candidates: Array(17).fill(address.candidates[0]) },
    ]) {
        assert.throws(() => validateAddress(bad));
        assert.throws(() => synthesizeAnswer(bad, CREDENTIAL));
    }
    assert.throws(() => synthesizeAnswer(address, "good\r\na=bad"));
    const ipv6 = { ...address, candidates: [{ host: "::1", port: 45678 }] };
    assert.match(synthesizeAnswer(ipv6, CREDENTIAL), /c=IN IP6 ::1/);
});

test("SDP normalizes native lowercase fingerprints without changing the saved route", () => {
    const lower = { ...address, fingerprint: address.fingerprint.toLowerCase() };
    const encoded = encodeAddress(lower);
    assert.match(synthesizeAnswer(lower, CREDENTIAL), new RegExp(`a=fingerprint:${address.fingerprint}`));
    assert.deepEqual(decodeAddress(encoded), lower);
    assert.equal(encodeAddress(lower), encoded);
});

const dispatch = (target, type, fields = {}) => target.dispatchEvent(Object.assign(new Event(type), fields));

class FakeChannel extends EventTarget {
    readyState = "connecting";
    bufferedAmount = 0;
    sent = [];
    send(data) { this.sent.push(data); }
    close() {
        if (this.readyState === "closed") return;
        this.readyState = "closed";
        dispatch(this, "close");
    }
}

function fakeConnection(behavior = "open") {
    const instances = [];
    class PeerConnection extends EventTarget {
        channel = new FakeChannel();
        connectionState = "new";
        constructor() { super(); instances.push(this); }
        createDataChannel(label, options) { this.options = options; return this.channel; }
        async createOffer() {
            if (behavior === "throw") throw new Error("offer failed");
            if (behavior === "hang") return new Promise(() => {});
            return { sdp: "a=ice-ufrag:old\r\na=ice-pwd:old\r\n" };
        }
        async setLocalDescription(offer) { this.offer = offer; }
        async setRemoteDescription(answer) {
            this.answer = answer;
            if (behavior === "fail") {
                this.connectionState = "failed";
                dispatch(this, "connectionstatechange");
            } else {
                this.channel.readyState = "open";
                dispatch(this.channel, "open");
            }
        }
        close() { this.connectionState = "closed"; this.channel.close(); }
    }
    return { PeerConnection, instances };
}

test("dial uses the supplied port and certificate and keeps an opened channel", async () => {
    const { PeerConnection } = fakeConnection();
    const result = await dial(address, CREDENTIAL, { ordered: false, maxRetransmits: 0 }, { PeerConnection });
    assert.equal(result.channel.readyState, "open");
    assert.deepEqual(result.connection.options, { ordered: false, maxRetransmits: 0 });
    assert.match(result.connection.answer.sdp, /127\.0\.0\.1 46660/);
    assert.ok(result.connection.answer.sdp.includes(address.fingerprint));
    result.connection.close();
});

test("dial closes all resources on SDP failure, ICE failure, timeout, and abort", async () => {
    for (const behavior of ["throw", "fail", "hang"]) {
        const { PeerConnection, instances } = fakeConnection(behavior);
        await assert.rejects(dial(address, CREDENTIAL, {}, { PeerConnection, timeoutMs: 10 }));
        assert.equal(instances[0].connectionState, "closed", behavior);
        assert.equal(instances[0].channel.readyState, "closed", behavior);
    }
    const { PeerConnection, instances } = fakeConnection("hang");
    const controller = new AbortController();
    const pending = dial(address, CREDENTIAL, {}, { PeerConnection, signal: controller.signal });
    controller.abort(new Error("superseded"));
    await assert.rejects(pending, /superseded/);
    assert.equal(instances[0].connectionState, "closed");
    await assert.rejects(dial(address, CREDENTIAL, {}, { PeerConnection, signal: controller.signal }), /superseded/);
    assert.equal(instances.length, 1, "an already-cancelled dial allocated a connection");
});

class FakePort extends EventTarget {
    messages = [];
    closed = false;
    postMessage(data, transfer) { this.messages.push({ data, transfer }); }
    start() {}
    close() { this.closed = true; }
}

test("relay bounds MessagePort flight and SCTP backlog without copying datagrams", (t) => {
    const channel = new FakeChannel();
    channel.readyState = "open";
    const port = new FakePort();
    const close = relay(channel, port);
    t.after(close);
    for (let i = 0; i < 100; i += 1) dispatch(channel, "message", { data: new ArrayBuffer(10) });
    assert.equal(port.messages.length, 64);
    assert.equal(port.messages[0].data, port.messages[0].transfer[0]);
    dispatch(port, "message", { data: "ack" });
    dispatch(channel, "message", { data: new ArrayBuffer(10) });
    assert.equal(port.messages.length, 65);
    channel.bufferedAmount = 256 * 1024;
    dispatch(port, "message", { data: new ArrayBuffer(10) });
    assert.equal(channel.sent.length, 0);
    channel.bufferedAmount = 0;
    dispatch(port, "message", { data: new ArrayBuffer(10) });
    assert.equal(channel.sent.length, 1);
    dispatch(port, "message", { data: null });
    assert.equal(port.closed, true);
    assert.equal(channel.readyState, "closed");
    const count = port.messages.length;
    dispatch(channel, "message", { data: new ArrayBuffer(10) });
    assert.equal(port.messages.length, count, "closed relay retained listeners");
});

test("relay expires a silent worker and releases its callbacks", (t) => {
    t.mock.timers.enable({ apis: ["setInterval"] });
    let now = 0;
    const channel = new FakeChannel();
    const port = new FakePort();
    const close = relay(channel, port, { heartbeatMs: 5, leaseMs: 15, now: () => now });
    t.after(close);
    now = 5;
    t.mock.timers.tick(5);
    assert.equal(port.messages[0].data, "ping");
    now = 15;
    t.mock.timers.tick(10);
    assert.equal(port.closed, true);
});

function nextMessage(port) {
    return new Promise((resolve) => port.addEventListener("message", resolve, { once: true }));
}

test("normal app carrier service accepts only its controller and closes on replacement", async (t) => {
    const workers = new EventTarget();
    workers.controller = {};
    const page = new EventTarget();
    const { PeerConnection, instances } = fakeConnection();
    const stop = serveCarrierRequests(workers, page, {
        dialPeer: (address, credential, init, options) => dial(address, credential, init, { ...options, PeerConnection }),
    });
    t.after(stop);
    const { port1, port2 } = new MessageChannel();
    t.after(() => { port1.close(); port2.close(); });
    const data = { v: 1, type: "tonk-rtc-dial", peer: "did:key:test", address };
    dispatch(workers, "message", { source: {}, data, ports: [port2] });
    assert.equal(instances.length, 0, "a non-controller opened a carrier");
    const response = nextMessage(port1);
    dispatch(workers, "message", { source: workers.controller, data, ports: [port2] });
    const event = await response;
    assert.equal(event.data.type, "carrier");
    assert.equal(event.ports.length, 1);
    t.after(() => event.ports[0].close());
    port1.postMessage({ v: 1, type: "ready" });
    assert.equal(instances[0].channel.readyState, "open");
    dispatch(workers, "controllerchange");
    assert.equal(instances[0].connectionState, "closed");
});

test("a page reports carrier failure to the waiting command", async (t) => {
    const workers = new EventTarget();
    workers.controller = {};
    const stop = serveCarrierRequests(workers, new EventTarget(), {
        dialPeer: async () => { throw new Error("local access was denied"); },
    });
    t.after(stop);
    const { port1, port2 } = new MessageChannel();
    t.after(() => { port1.close(); port2.close(); });
    const response = nextMessage(port1);
    dispatch(workers, "message", { source: workers.controller,
        data: { v: 1, type: "tonk-rtc-dial", peer: "test", address }, ports: [port2] });
    assert.deepEqual((await response).data, { v: 1, type: "error", detail: "local access was denied" });
});

test("a late dial completion cannot leak a connection after page cancellation", async (t) => {
    const workers = new EventTarget();
    workers.controller = {};
    const page = new EventTarget();
    let complete;
    const stop = serveCarrierRequests(workers, page, {
        dialPeer: () => new Promise((resolve) => { complete = resolve; }),
    });
    t.after(stop);
    const reply = new FakePort();
    dispatch(workers, "message", { source: workers.controller,
        data: { v: 1, type: "tonk-rtc-dial", peer: "test", address }, ports: [reply] });
    dispatch(page, "pagehide");
    const { PeerConnection } = fakeConnection();
    const connection = new PeerConnection();
    complete({ connection, channel: connection.channel });
    await new Promise(setImmediate);
    assert.equal(connection.connectionState, "closed");
    assert.equal(reply.messages.length, 0, "a cancelled request handed a carrier to the worker");
});

test("an unacknowledged registration closes its carrier within five seconds", async (t) => {
    t.mock.timers.enable({ apis: ["setTimeout", "setInterval"] });
    const workers = new EventTarget();
    workers.controller = {};
    const { PeerConnection } = fakeConnection();
    const connection = new PeerConnection();
    connection.channel.readyState = "open";
    const stop = serveCarrierRequests(workers, new EventTarget(), {
        dialPeer: async () => ({ connection, channel: connection.channel }),
        Channels: class { port1 = new FakePort(); port2 = new FakePort(); },
    });
    t.after(stop);
    const reply = new FakePort();
    dispatch(workers, "message", { source: workers.controller,
        data: { v: 1, type: "tonk-rtc-dial", peer: "test", address }, ports: [reply] });
    await new Promise(setImmediate);
    assert.equal(reply.messages[0].data.type, "carrier");
    t.mock.timers.tick(5000);
    assert.equal(connection.connectionState, "closed");
    assert.equal(reply.closed, true);
});

test("worker cancellation aborts ICE before it returns a carrier", async (t) => {
    const workers = new EventTarget();
    workers.controller = {};
    let signal, complete;
    const stop = serveCarrierRequests(workers, new EventTarget(), {
        dialPeer: (_address, _credential, _init, options) => {
            signal = options.signal;
            return new Promise(resolve => { complete = resolve; });
        },
    });
    t.after(stop);
    const reply = new FakePort();
    dispatch(workers, "message", { source: workers.controller,
        data: { v: 1, type: "tonk-rtc-dial", peer: "test", address }, ports: [reply] });
    assert.equal(signal.aborted, false);
    dispatch(reply, "message", { data: { v: 1, type: "cancel" } });
    assert.equal(signal.aborted, true);
    const { PeerConnection } = fakeConnection();
    const connection = new PeerConnection();
    complete({ connection, channel: connection.channel });
    await new Promise(setImmediate);
    assert.equal(connection.connectionState, "closed");
    assert.equal(reply.messages.length, 0);
});

// Both sides use ONE string as ufrag AND password. That is what removes
// the round trip: get it wrong and the CLI's USERNAME check rejects
// every binding request, which is silent on the wire.
test("the synthesized answer uses the dial's credential for both ICE fields", () => {
    const sdp = synthesizeAnswer(address, CREDENTIAL);
    assert.match(sdp, new RegExp(`a=ice-ufrag:${CREDENTIAL}\r\n`));
    assert.match(sdp, new RegExp(`a=ice-pwd:${CREDENTIAL}\r\n`));
});

// One address must serve many dials, so the credential cannot live in
// the address: ICE separates peers by ufrag, and a fixed one would mean
// exactly one connection ever, concurrently or sequentially.
test("every dial mints its own credential", () => {
    const minted = new Set(Array.from({ length: 50 }, () => freshCredential()));
    assert.equal(minted.size, 50, "credentials repeated across dials");
    for (const credential of minted) {
        assert.ok(credential.length >= 22, "shorter than RFC 5245 allows for an ICE password");
        assert.match(credential, /^[A-Za-z0-9+/]+$/, "must use the ICE character alphabet");
    }
});

test("ICE credentials do not substitute the base64url alphabet", (t) => {
    t.mock.method(globalThis.crypto, "getRandomValues", (bytes) => bytes.fill(255));
    assert.equal(freshCredential(), "/".repeat(32));
});

test("every published candidate reaches the synthesized answer", () => {
    const sdp = synthesizeAnswer(address, CREDENTIAL);
    for (const candidate of address.candidates) {
        assert.match(sdp, new RegExp(`a=candidate:\\d+ 1 udp \\d+ ${candidate.host.replace(/\./g, "\\.")} ${candidate.port} typ host`));
    }
    assert.match(sdp, /a=end-of-candidates/);
});

// The CLI pins its answering DTLS role so it need not travel in the
// address; if that pin is ever removed these two disagree silently.
test("the synthesized answer hard-codes the DTLS role the CLI pins", () => {
    assert.match(synthesizeAnswer(address, CREDENTIAL), /a=setup:active\r\n/);
});

test("munging replaces our own ICE credentials, not just the first", () => {
    const offer = "a=ice-ufrag:AbCd\r\na=ice-pwd:originalpassword\r\n"
        + "a=ice-ufrag:AbCd\r\na=ice-pwd:originalpassword\r\n";
    const munged = mungeOffer(offer, CREDENTIAL);
    assert.ok(!munged.includes("AbCd"), "a stale ufrag survived");
    assert.ok(!munged.includes("originalpassword"), "a stale password survived");
    assert.equal(munged.match(new RegExp(CREDENTIAL, "g")).length, 4);
});

test("the span is as wide as Rust says, and starts where the port does", async () => {
    // Read out of the Rust rather than copied, for the same reason the
    // port test does it: a span the two halves disagree about means a
    // browser that cannot find a listener which is running perfectly.
    const rust = readFileSync(
        new URL("../../tonk-rtc/src/rendezvous.rs", import.meta.url),
        "utf8",
    );
    const [, span] = /SPAN: u16 = (\d+)/.exec(rust);

    assert.equal(RENDEZVOUS_SPAN, Number(span), "the two halves disagree about the span");

    const ports = await rendezvousPorts(RENDEZVOUS);
    assert.equal(ports.length, RENDEZVOUS_SPAN);
    assert.equal(ports[0], await rendezvousPort(RENDEZVOUS), "the span starts at the derived port");
    assert.equal(ports.at(-1), ports[0] + RENDEZVOUS_SPAN - 1, "and runs contiguously");
});

test("a local address offers every port in the span", async () => {
    // One candidate per slot: ICE races them and keeps the pair that
    // answers, so a listener on any slot is found in one dial. A single
    // candidate would only ever reach the first `tonk` on a machine.
    const der = new Uint8Array([1, 2, 3]);
    const address = await localAddress(
        `data:application/octet-stream;base64,${Buffer.from(der).toString("base64")}`,
    );
    assert.equal(address.candidates.length, RENDEZVOUS_SPAN);
    assert.ok(address.candidates.every((c) => c.host === "127.0.0.1"));
});

test("the port is derived from the phrase, matching Rust", async () => {
    // Pinned against `tonk_rtc::rendezvous`, read out of the Rust rather
    // than copied here, so the two derivations cannot drift apart.
    const rust = readFileSync(
        new URL("../../tonk-rtc/src/rendezvous.rs", import.meta.url),
        "utf8",
    );
    const [, phrase] = /RENDEZVOUS: &str = "([^"]+)"/.exec(rust);

    assert.equal(RENDEZVOUS, phrase, "the two halves disagree about the phrase");
    assert.equal(await rendezvousPort(phrase), 55991);

    const port = await rendezvousPort(phrase);
    assert.ok(port >= 49152 && port <= 65535, `${port} is outside the dynamic range`);
});

test("the fingerprint is the hash of the served certificate", async () => {
    const der = readFileSync(
        new URL("../../tonk-rtc/assets/rendezvous.der", import.meta.url),
    );
    const fingerprint = await rendezvousFingerprint(new Uint8Array(der));

    // One WebCrypto call and no crypto library: the whole reason the
    // certificate is served rather than rebuilt here.
    assert.match(fingerprint, /^sha-256 ([0-9A-F]{2}:){31}[0-9A-F]{2}$/);
});

test("a local address is loopback plus two derived values", async () => {
    const der = readFileSync(
        new URL("../../tonk-rtc/assets/rendezvous.der", import.meta.url),
    );
    // `der.buffer` is Node's pooled allocation and is bigger than the
    // file, which would hash to something else entirely. A real `fetch`
    // hands back an exactly-sized buffer; this makes the fake do the
    // same rather than silently disagreeing with the browser.
    const exact = der.buffer.slice(der.byteOffset, der.byteOffset + der.byteLength);
    globalThis.fetch = async () => ({ ok: true, arrayBuffer: async () => exact });

    const address = await localAddress();
    assert.equal(address.candidates[0].host, "127.0.0.1");
    assert.equal(address.candidates[0].port, await rendezvousPort());
    assert.equal(address.fingerprint, await rendezvousFingerprint(new Uint8Array(der)));

    const sdp = synthesizeAnswer(address, "credential");
    assert.match(sdp, /a=setup:active/);
    assert.match(sdp, /127\.0\.0\.1 55991 typ host/);
});
