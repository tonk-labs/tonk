import { test } from 'node:test';
import assert from 'node:assert/strict';
import { workerPrimitive } from './primitive.mjs';

const subject = 'did:key:z6Test';

test('portable primitive preserves multi-document input, cancellation and worker envelopes', async () => {
  const document = 'example!:\n  name: "one"\n---\nexample:\n';
  const envelope = { revision: { opaque: [1, 2] }, matches: [], effects: { commands: 1 } };
  const signal = new AbortController().signal;
  const calls = [];
  const backend = workerPrimitive({
    subject,
    request: async (path, options) => {
      calls.push({ path, ...options });
      return Response.json(envelope);
    },
  });
  for (const name of ['tonk_query', 'tonk_evaluate']) {
    assert.deepEqual(await backend.call(name, { document }, signal), envelope);
  }
  assert.deepEqual(
    calls.map((call) => call.path),
    [false, true].map(
      (transact) => `/api/repository/did%3Akey%3Az6Test/branch/main/evaluate?transact=${transact}`,
    ),
  );
  for (const call of calls) {
    assert.equal(call.body, document);
    assert.equal(call.signal, signal);
  }
});

test('read-only gate, UTF-8 limit, target override and cancellation reject before dispatch', async () => {
  let calls = 0;
  const backend = workerPrimitive({
    subject,
    allowsWrites: false,
    request: async () => {
      calls++;
    },
  });
  assert.deepEqual(backend.capabilities, ['tonk_query']);
  await assert.rejects(backend.call('tonk_evaluate', { document: 'x:' }), /read-only/);
  for (const args of [
    null,
    [],
    {},
    { document: ' ' },
    { document: 'é'.repeat(16001) },
    { document: 'x:', space: 'did:key:other' },
    { document: 'x:', branch: 'other' },
  ]) {
    await assert.rejects(backend.call('tonk_query', args));
  }
  await assert.rejects(backend.call('tonk_query', { document: 'x:' }, AbortSignal.abort()));
  assert.equal(calls, 0);
});

test('lost, malformed and oversized write responses remain uncertain and are never replayed', async () => {
  for (const response of [
    () => {
      throw Error('private transport token');
    },
    () => new Response('private response'),
    () => Response.json([]),
    () => Response.json({ data: 'x'.repeat(2_000_001) }),
    () => new Response('private response', { status: 503 }),
  ]) {
    let calls = 0;
    const backend = workerPrimitive({
      subject,
      request: async () => {
        calls++;
        return response();
      },
    });
    await assert.rejects(
      backend.call('tonk_evaluate', { document: 'example!:' }),
      (error) => /Do not repeat/.test(error.message) && !error.message.includes('private'),
    );
    assert.equal(calls, 1);
  }
});

test('oversized success and analysis-error streams stop reading and cancel without replay', async () => {
  for (const status of [200, 400]) {
    for (const name of ['tonk_query', 'tonk_evaluate']) {
      let chunks = 0;
      let cancelled = false;
      let calls = 0;
      const backend = workerPrimitive({
        subject,
        request: async () => {
          calls++;
          return new Response(
            new ReadableStream(
              {
                pull(controller) {
                  if (chunks === 8) return controller.close();
                  chunks++;
                  controller.enqueue(new Uint8Array(1_000_000).fill(32));
                },
                cancel() {
                  cancelled = true;
                },
              },
              { highWaterMark: 0 },
            ),
            { status, headers: { 'content-type': 'application/json' } },
          );
        },
      });
      await assert.rejects(
        backend.call(name, { document: 'example:' }),
        name === 'tonk_evaluate' ? /Do not repeat/ : /query/,
      );
      assert.equal(calls, 1);
      assert.equal(cancelled, true);
      assert.equal(chunks, 3, 'stop at the first chunk exceeding the 2 MB limit');
    }
  }
});

test('response byte boundary preserves split UTF-8 and the unchanged envelope', async () => {
  const envelope = { data: 'é'.repeat(999_994) + 'a' };
  const bytes = new TextEncoder().encode(JSON.stringify(envelope));
  assert.equal(bytes.byteLength, 2_000_000);
  const backend = workerPrimitive({
    subject,
    request: async () =>
      new Response(
        new ReadableStream({
          start(controller) {
            controller.enqueue(bytes.subarray(0, 10)); // Inside the first UTF-8 character.
            controller.enqueue(bytes.subarray(10));
            controller.close();
          },
        }),
        { headers: { 'content-type': 'application/json' } },
      ),
  });
  assert.deepEqual(await backend.call('tonk_query', { document: 'example:' }), envelope);
});

test('unused response bodies are cancelled and bounded analysis errors stay actionable', async () => {
  for (const status of [200, 503]) {
    let cancelled = false;
    const backend = workerPrimitive({
      subject,
      request: async () =>
        new Response(
          new ReadableStream(
            {
              cancel() {
                cancelled = true;
              },
            },
            { highWaterMark: 0 },
          ),
          { status, headers: { 'content-type': 'text/plain' } },
        ),
    });
    await assert.rejects(backend.call('tonk_evaluate', { document: 'example!:' }), /Do not repeat/);
    assert.equal(cancelled, true);
  }
  const backend = workerPrimitive({
    subject,
    request: async () =>
      Response.json({ error: { kind: 'analyze', message: 'Unknown concept' } }, { status: 400 }),
  });
  await assert.rejects(
    backend.call('tonk_evaluate', { document: 'example!:' }),
    /Notation rejected: Unknown concept/,
  );
});

test('primitive bundles for a browser host without Node or MCP dependencies', async () => {
  const { build } = await import('esbuild');
  const { fileURLToPath } = await import('node:url');
  const bundle = await build({
    entryPoints: [fileURLToPath(new URL('./primitive.mjs', import.meta.url))],
    platform: 'browser',
    format: 'esm',
    bundle: true,
    write: false,
    metafile: true,
  });
  assert.deepEqual(
    Object.keys(bundle.metafile.inputs)
      .map((path) => path.split('/').at(-1))
      .sort(),
    ['errors.mjs', 'primitive.mjs'],
  );
});
