import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { agentDebugBundle, bounded, probes, projectWorkerHealth, readProbe, redactDebug, workerAction, workerSnapshot } from "../assets/doctor.mjs";

test("doctor startup never registers a worker or arms boot recovery", () => {
    const html = readFileSync(new URL("../index.html", import.meta.url), "utf8");
    const scripts = [...html.matchAll(/<script(?: type="module")?>([\s\S]*?)<\/script>/g)].map(match => match[1]);
    const route = scripts.find(script => script.includes("globalThis.tonkDoctorRoute ="));
    const worker = scripts.find(script => script.includes("const serviceWorkersSupported"));
    const watchdog = scripts.find(script => script.includes('const RETRIES ='));
    for (const path of ["/doctor", "/doctor/"]) {
        const context = { location: { pathname: path } };
        runInNewContext(route, context);
        // navigator/document/timers deliberately absent: touching any is a failure.
        runInNewContext(worker, context);
        runInNewContext(watchdog, context);
        assert.equal(context.tonkDoctorRoute, true);
    }
    for (const path of ["/", "/doctorish", "/doctor/nested"]) {
        const context = { location: { pathname: path } };
        runInNewContext(route, context);
        assert.equal(context.tonkDoctorRoute, false);
    }
});

test("root and profile projections exclude authority and unknown fields", () => {
    const project = path => probes.find(probe => probe[1] === path)[2];
    const root = project("/api/identity/root")({
        status: "ready", rootDid: "did:root", deviceDid: "did:device",
        credentialId: "secret", delegationHex: "secret", encryptionKey: "secret",
        futureSecret: "secret",
    });
    assert.deepEqual(root, { status: "ready", rootDid: "did:root", deviceDid: "did:device" });
    const profile = project("/api/profile")({ profile: { operator: "did:operator", remote: { token: "secret" } }, space: [] });
    assert.equal(profile.profile.operator, "did:operator");
    assert.ok(!JSON.stringify(profile).includes("secret"));
    assert.deepEqual(project("/api/health")({ worker: "failed" }), { worker: "failed", log: null });
});

test("probes reject uncontrolled requests, HTTP failures and SPA fallbacks", async () => {
    const env = { navigator: {}, fetch() { assert.fail("must not fetch without controller"); } };
    await assert.rejects(readProbe("/api/account", value => value, env), /No controlling/);
    env.navigator.serviceWorker = { controller: {} };
    env.fetch = async () => new Response("sensitive error body", { status: 503 });
    await assert.rejects(readProbe("/api/account", value => value, env), /^Error: HTTP 503/);
    env.fetch = async () => new Response("<html>shell</html>", { headers: { "content-type": "text/html" } });
    await assert.rejects(readProbe("/api/account", value => value, env), /Expected JSON/);
    env.fetch = async () => Response.json({ status: "rootMissing" });
    assert.deepEqual(await readProbe("/api/account", value => value, env), { status: "rootMissing" });
});

test("a stalled probe times out without blocking successful probes", async () => {
    await assert.rejects(bounded(() => new Promise(() => {}), 5), /Timed out/);
    assert.equal(await bounded(() => 42), 42);
});

test("worker tools only touch the covering registration and report actual results", async () => {
    const calls = [];
    const registration = {
        scope: "https://tonk.test/", active: { scriptURL: "/service_worker.js", state: "activated" },
        unregister: async () => { calls.push("unregister"); return true; },
        update: async () => { calls.push("update"); },
    };
    const nav = { serviceWorker: {
        getRegistration: async (...args) => { assert.deepEqual(args, []); return registration; },
        getRegistrations: () => assert.fail("must not touch other registrations"),
    } };
    assert.match(await workerAction("unregister", nav), /Registration removed/);
    assert.match(await workerAction("update", nav), /Update check completed/);
    assert.deepEqual(calls, ["unregister", "update"]);
    assert.equal((await workerSnapshot(nav)).registration.active.state, "activated");
    registration.unregister = async () => false;
    assert.match(await workerAction("unregister", nav), /already removed/);
    registration.update = async () => { throw new Error("network unavailable"); };
    await assert.rejects(workerAction("update", nav), /network unavailable/);
});


test("agent bundle carries diagnostic failures, issue context and recent filtered logs", () => {
    const health = projectWorkerHealth({
        worker: "failed", build: "build123", startedAt: 1000,
        log: Array.from({ length: 205 }, (_, index) => ({
            t: 1000 + index, level: "error", message: `failure ${index}: delegationHex=deadbeef`,
            unknown: "excluded",
        })),
    });
    assert.equal(health.log.length, 200);
    assert.equal(health.log[0].t, 1005);
    assert.equal(health.log[199].level, "error");
    const bundle = agentDebugBundle({ capturedAt: "2026-09-09T00:00:00Z", diagnostics: {
        "Worker health": { status: "ok", value: health },
        Account: { status: "error", error: "HTTP 503", durationMs: 20 },
    } }, "  Opening a space hangs  ");
    const parsed = JSON.parse(bundle);
    assert.equal(parsed.issue, "Opening a space hangs");
    assert.equal(parsed.diagnostics.Account.error, "HTTP 503");
    assert.equal(parsed.diagnostics["Worker health"].value.build, "build123");
    assert.match(parsed.coverage, /reset when the worker restarts/);
    assert.match(parsed.coverage, /Earlier page-console history/);
    assert.ok(!bundle.includes("deadbeef"));
    assert.ok(!bundle.includes("excluded"));
    assert.match(bundle, /failure 204/);
});

test("debug filtering handles nested secrets, log credentials, and invite URLs", () => {
    const value = {
        credentialId: "opaque-id", nested: { private_key: "key-material" },
        message: 'GET https://user:pass@tonk.test/space?token=query-secret#invite-secret failed: Bearer bearer-secret {"delegationHex":"grant-secret"} encryption_key=enc-secret',
        jwt: "eyJhbGciOiJ9.eyJzdWIiOiJ9.signature",
        operator: "did:key:public-operator", error: "GET /api/profile HTTP 503",
    };
    const filtered = redactDebug(value);
    const text = JSON.stringify(filtered);
    for (const secret of ["opaque-id", "key-material", "user:pass", "query-secret", "invite-secret", "bearer-secret", "grant-secret", "enc-secret", "eyJhbGciOiJ9"]) {
        assert.ok(!text.includes(secret), `leaked ${secret}`);
    }
    assert.equal(filtered.operator, value.operator);
    assert.equal(filtered.error, value.error);
    assert.equal(value.credentialId, "opaque-id", "must not mutate the API response");
});

test("bundles remain copyable when worker logs are unavailable", () => {
    const bundle = JSON.parse(agentDebugBundle({ diagnostics: {
        "Worker health": { status: "error", error: "No controlling service worker" },
    } }));
    assert.match(bundle.issue, /Ask for the symptom/);
    assert.equal(bundle.diagnostics["Worker health"].error, "No controlling service worker");
    assert.equal(projectWorkerHealth({ log: [] }).log.length, 0);
    assert.equal(projectWorkerHealth({}).log, null);
});
