import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { decodeAddress, decodeDescription, encodeDescription, isLoopback, mungeOffer, synthesizeAnswer } from "../assets/rtc.mjs";

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
    credential: "c2hhcmVkLWNyZWRlbnRpYWwtdmFsdWU",
};
const encodeAddress = (value) => Buffer.from(JSON.stringify(value)).toString("base64url");

test("an address round-trips and is rejected when incomplete", () => {
    assert.deepEqual(decodeAddress(encodeAddress(address)), address);
    for (const missing of ["candidates", "fingerprint", "credential"]) {
        const broken = { ...address, [missing]: missing === "candidates" ? [] : undefined };
        assert.throws(() => decodeAddress(encodeAddress(broken)), /missing/);
    }
});

// Both sides use ONE string as ufrag AND password. That is what removes
// the round trip: get it wrong and the CLI's USERNAME check rejects
// every binding request, which is silent on the wire.
test("the synthesized answer uses the shared credential for both ICE fields", () => {
    const sdp = synthesizeAnswer(address);
    assert.match(sdp, new RegExp(`a=ice-ufrag:${address.credential}\r\n`));
    assert.match(sdp, new RegExp(`a=ice-pwd:${address.credential}\r\n`));
});

test("every published candidate reaches the synthesized answer", () => {
    const sdp = synthesizeAnswer(address);
    for (const candidate of address.candidates) {
        assert.match(sdp, new RegExp(`a=candidate:\\d+ 1 udp \\d+ ${candidate.host.replace(/\./g, "\\.")} ${candidate.port} typ host`));
    }
    assert.match(sdp, /a=end-of-candidates/);
});

// The CLI pins its answering DTLS role so it need not travel in the
// address; if that pin is ever removed these two disagree silently.
test("the synthesized answer hard-codes the DTLS role the CLI pins", () => {
    assert.match(synthesizeAnswer(address), /a=setup:active\r\n/);
});

test("munging replaces our own ICE credentials, not just the first", () => {
    const offer = "a=ice-ufrag:AbCd\r\na=ice-pwd:originalpassword\r\n"
        + "a=ice-ufrag:AbCd\r\na=ice-pwd:originalpassword\r\n";
    const munged = mungeOffer(offer, address.credential);
    assert.ok(!munged.includes("AbCd"), "a stale ufrag survived");
    assert.ok(!munged.includes("originalpassword"), "a stale password survived");
    assert.equal(munged.match(new RegExp(address.credential, "g")).length, 4);
});
