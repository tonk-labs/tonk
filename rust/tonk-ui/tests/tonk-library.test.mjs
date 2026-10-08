import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const source = readFileSync(
  new URL("../../tonk-core/assets/library/tonk/tonk.js", import.meta.url),
  "utf8",
);

// A fresh copy of the module, loaded with `window.tonk` as given.
let copies = 0;
async function load(tonk) {
  globalThis.tonk = tonk;
  return import(`data:text/javascript;copy=${copies++},${encodeURIComponent(source)}`);
}

// Record requests and answer each with `reply(url, init)`.
function serve(reply) {
  const requests = [];
  globalThis.fetch = async (url, init) => {
    requests.push({ url, init });
    return reply(url, init);
  };
  return requests;
}

const json = value => new Response(JSON.stringify(value));
const SPACE = { context: { with: "main@did:key:zSpace", repo: "did:key:zSpace", branch: "main" } };

test("a query asks the page's own branch and answers with its rows", async () => {
  const { query } = await load(structuredClone(SPACE));
  const requests = serve(() => json([{ this: "a", fields: {} }]));

  const rows = await query({ terms: { this: "a" } });

  assert.deepEqual(rows, [{ this: "a", fields: {} }]);
  assert.equal(requests[0].url, "/api/repository/did:key:zSpace/branch/main/query");
  assert.equal(requests[0].init.method, "POST");
  assert.deepEqual(JSON.parse(requests[0].init.body), { terms: { this: "a" } });
});

test("a call names another branch as text or as `with`", async () => {
  const { query, transact } = await load(structuredClone(SPACE));
  const requests = serve(() => json([]));

  await query({ terms: {} }, "draft@profile:tonk");
  await transact({ claims: [] }, { with: "side@did:key:zOther" });

  assert.equal(requests[0].url, "/api/repository/profile:tonk/branch/draft/query");
  assert.equal(requests[1].url, "/api/repository/did:key:zOther/branch/side/transact");
});

test("the same query asked twice at once is asked of the worker once", async () => {
  const { query } = await load(structuredClone(SPACE));
  const requests = serve(() => json([]));

  await Promise.all([query({ terms: {} }), query({ terms: {} })]);
  await query({ terms: {} });

  assert.equal(requests.length, 2, "one for the pair, one for the later call");
});

test("a refused transaction rejects with what the worker said", async () => {
  const { transact } = await load(structuredClone(SPACE));
  serve(() => new Response(JSON.stringify({ error: { message: "not a member" } }), { status: 403 }));

  await assert.rejects(transact({ claims: [] }), /not a member/);
});

test("evaluate sends the document, committed unless told not to", async () => {
  const { evaluate } = await load(structuredClone(SPACE));
  const requests = serve(() => json({ ok: true }));

  await evaluate({ document: "note!:\n  text: hi\n" });
  await evaluate({ document: "note:\n", transact: false });

  assert.equal(requests[0].url, "/api/repository/did:key:zSpace/branch/main/evaluate");
  assert.equal(requests[0].init.body, "note!:\n  text: hi\n");
  assert.equal(requests[1].url, "/api/repository/did:key:zSpace/branch/main/evaluate?transact=false");
});

test("a subscription yields every matching row after each change", async () => {
  const { subscribe } = await load(structuredClone(SPACE));
  const frame = said => new TextEncoder().encode(`data: ${JSON.stringify(said)}\n\n`);
  const one = { this: "a", fields: { this: "a", title: "one" } };
  const two = { this: "b", fields: { this: "b", title: "two" } };
  const renamed = { this: "a", fields: { this: "a", title: "uno" } };
  let feed;
  const requests = serve(
    () => new Response(new ReadableStream({ start: controller => (feed = controller) })),
  );

  const reader = subscribe({ terms: {} }).getReader();
  const next = async said => {
    while (!feed) await new Promise(resolve => setTimeout(resolve));
    feed.enqueue(frame(said));
    return (await reader.read()).value;
  };

  assert.deepEqual(await next({ kind: "snapshot", conclusions: [one] }), [one]);
  assert.deepEqual(await next({ kind: "delta", asserted: [two], retracted: [] }), [one, two]);
  assert.deepEqual(await next({ kind: "delta", asserted: [renamed], retracted: [one] }), [two, renamed]);
  // A retraction that matches no kept row: the asserted row still
  // replaces the entity's row for the same field.
  const drifted = { this: "b", fields: { this: "b", title: "stale" } };
  const fresh = { this: "b", fields: { this: "b", title: "dos" } };
  assert.deepEqual(await next({ kind: "delta", asserted: [fresh], retracted: [drifted] }), [renamed, fresh]);
  assert.deepEqual(await next([one]), [one], "a bare array replaces the rows");

  assert.equal(requests[0].init.headers.accept, "text/event-stream");
  await reader.cancel();
  assert.equal(requests[0].init.signal.aborted, true, "cancelling ends the request");
});

test("importing puts the four on window.tonk without replacing what is there", async () => {
  const own = () => "mine";
  const tonk = { ...structuredClone(SPACE), query: own };

  const module = await load(tonk);

  assert.equal(tonk.query, own);
  assert.equal(tonk.subscribe, module.subscribe);
  assert.equal(tonk.transact, module.transact);
  assert.equal(tonk.evaluate, module.evaluate);
});
