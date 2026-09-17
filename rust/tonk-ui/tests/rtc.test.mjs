import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import {
    DEFAULT_PORT,
    SHARED_FINGERPRINT,
    decodeAddress,
    decodeDescription,
    encodeDescription,
    freshCredential,
    isLoopback,
    localAddress,
    mungeOffer,
    synthesizeAnswer,
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
        assert.match(credential, /^[A-Za-z0-9_-]+$/, "must survive an SDP line unescaped");
    }
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

test("a local address needs nothing published", () => {
    const address = localAddress();
    // The whole property: no fetch, no paste, no fragment. A page that
    // knows only that tonk might be running can build this.
    assert.equal(address.candidates.length, 1);
    assert.equal(address.candidates[0].host, "127.0.0.1");
    assert.equal(address.candidates[0].port, DEFAULT_PORT);
    assert.equal(address.fingerprint, SHARED_FINGERPRINT);
});

test("a conjured local address synthesizes a usable answer", () => {
    const sdp = synthesizeAnswer(localAddress(), "credential");
    assert.match(sdp, /a=fingerprint:sha-256 08:EC:/);
    assert.match(sdp, /127\.0\.0\.1 51247 typ host/);
    assert.match(sdp, /a=ice-ufrag:credential/);
    assert.match(sdp, /a=setup:active/);
});

test("a non-default port still yields a complete address", () => {
    const address = localAddress(9999);
    assert.equal(address.candidates[0].port, 9999);
    assert.equal(address.fingerprint, SHARED_FINGERPRINT);
});
