import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { ACCESS, errorMessage, isLoopback, sessionRequest } from "../assets/agent.mjs";

test("the approval page starts no worker and arms no boot recovery", () => {
    const html = readFileSync(new URL("../index.html", import.meta.url), "utf8");
    const scripts = [...html.matchAll(/<script(?: type="module")?>([\s\S]*?)<\/script>/g)].map(match => match[1]);
    const route = scripts.find(script => script.includes("globalThis.tonkDoctorRoute ="));
    const worker = scripts.find(script => script.includes("const serviceWorkersSupported"));
    const watchdog = scripts.find(script => script.includes("const RETRIES ="));
    const context = { location: { pathname: "/settings/agent" } };
    runInNewContext(route, context);
    // navigator/document/timers deliberately absent: touching any is a failure.
    runInNewContext(worker, context);
    runInNewContext(watchdog, context);
    assert.equal(context.tonkAgentRoute, true);
    assert.equal(context.tonkDoctorRoute, false);
    for (const path of ["/settings", "/settings/link", "/settings/agent/x"]) {
        const other = { location: { pathname: path } };
        runInNewContext(route, other);
        assert.equal(other.tonkStandaloneRoute, false, path);
    }
});

test("only spaces given some access are requested", () => {
    assert.deepEqual(ACCESS.map(([, label]) => label), ["No access", "Read", "Read and write"]);
    assert.deepEqual(sessionRequest("  Claude ", [
        { key: "a", access: "none" },
        { key: "b", access: "read" },
        { key: "c", access: "write" },
    ]), { name: "Claude", spaces: [{ key: "b", access: "read" }, { key: "c", access: "write" }] });
});

test("errors read from either service's body", () => {
    assert.equal(errorMessage({ error: { kind: "forbidden", message: "sign in" } }, 403), "sign in");
    assert.equal(errorMessage({ error: "invalid_request", error_description: "gone" }, 410), "gone");
    assert.equal(errorMessage(null, 502), "Request failed (HTTP 502)");
});

test("loopback callbacks are recognised", () => {
    for (const host of ["localhost", "127.0.0.1", "[::1]"]) assert.ok(isLoopback(host), host);
    for (const host of ["claude.ai", "chatgpt.com", undefined]) assert.ok(!isLoopback(host), host);
});
