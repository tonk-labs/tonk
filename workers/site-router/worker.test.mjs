import assert from "node:assert/strict";
import { test } from "node:test";

import router, { previewOf, previewOrigin } from "./worker.mjs";

const env = (overrides = {}) => ({
    PREVIEW_ORIGIN: "https://pr-{number}-tonk-access-service-preview.tonk.workers.dev",
    DEV: { fetch: async request => new Response(`dev:${new URL(request.url).host}`) },
    ...overrides,
});

const SPACE = "b5uayihhuxoyestthq53muwjzfsnpb6lsawlmd3wyvuhwlienuqkadty";

test("it names the pull request a preview's app and sites belong to", () => {
    assert.equal(previewOf("pr-33.tonk.foundation"), 33);
    assert.equal(previewOf("profile-pr33.tonk.foundation"), 33);
    assert.equal(previewOf(`${SPACE}-pr1047.tonk.foundation`), 1047);
});

test("it leaves the dev deployment's own sites alone", () => {
    assert.equal(previewOf("profile.tonk.foundation"), null);
    assert.equal(previewOf(`${SPACE}.tonk.foundation`), null);
    // Not a number, so not a preview.
    assert.equal(previewOf("pr-next.tonk.foundation"), null);
    assert.equal(previewOf("profile-prx.tonk.foundation"), null);
});

test("it sends a dev site to the dev worker", async () => {
    const response = await router.fetch(new Request("https://profile.tonk.foundation/space-origin.html"), env());
    assert.equal(await response.text(), "dev:profile.tonk.foundation");
});

test("it sends a preview's request to that preview, path and query kept", async t => {
    const seen = [];
    t.mock.method(globalThis, "fetch", async (request, init) => {
        seen.push({ url: request.url, host: request.headers.get("x-forwarded-host"), method: request.method, init });
        return new Response("preview", { headers: { "content-type": "text/plain" } });
    });
    const response = await router.fetch(
        new Request(`https://${SPACE}-pr33.tonk.foundation/guest/x.js?v=1`, { method: "GET" }),
        env(),
    );
    assert.equal(await response.text(), "preview");
    assert.deepEqual(seen, [
        {
            url: "https://pr-33-tonk-access-service-preview.tonk.workers.dev/guest/x.js?v=1",
            host: `${SPACE}-pr33.tonk.foundation`,
            method: "GET",
            init: { redirect: "manual" },
        },
    ]);
    assert.equal(previewOrigin(33, env()), "https://pr-33-tonk-access-service-preview.tonk.workers.dev");
});

test("it passes a preview's redirect back for the browser to follow", async t => {
    t.mock.method(globalThis, "fetch", async () =>
        new Response(null, { status: 301, headers: { location: "/join?x=1" } }),
    );
    const response = await router.fetch(new Request("https://pr-33.tonk.foundation/@/abc"), env());
    assert.equal(response.status, 301);
    assert.equal(response.headers.get("location"), "/join?x=1");
});
