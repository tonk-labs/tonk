import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const generator = fileURLToPath(new URL('./scripts/sync-desktop.mjs', import.meta.url));
test('Desktop generation is deterministic, detects drift, and executes the canonical worker boundary', async () => {
  const root = await mkdtemp(join(tmpdir(), 'tonk-desktop-generation-'));
  try {
    await mkdir(join(root, 'Sources/HarnessCore'), { recursive: true });
    const run = (...args) =>
      spawnSync(process.execPath, [generator, ...args, root], { encoding: 'utf8' });
    assert.equal(run().status, 0);
    const contextPath = join(root, 'Sources/HarnessCore/TonkAgentContext.generated.swift');
    const first = await readFile(contextPath, 'utf8');
    assert.equal(run().status, 0);
    assert.equal(await readFile(contextPath, 'utf8'), first);
    assert.equal(run('--check').status, 0);
    const script = first.match(/public static let workerScript = (#+)"""\n([\s\S]*?)\n"""\1/)[2];
    const call = new (Object.getPrototypeOf(async function () {}).constructor)(
      'subject',
      'tool',
      'argumentsJSON',
      'fetch',
      script,
    );
    const requests = [];
    const fetch = async (path, options) => {
      requests.push({ path, options });
      return Response.json({
        revision_before: null,
        revision_after: null,
        commits: { claims: 0 },
        matches_after: [],
      });
    };
    for (const tool of ['tonk_query', 'tonk_evaluate']) {
      const result = await call(
        'did:key:z123',
        tool,
        JSON.stringify({ document: 'concept:' }),
        fetch,
      );
      assert.ok(result.response, result.error);
    }
    assert.deepEqual(
      requests.map((x) => x.path),
      [
        '/api/repository/did%3Akey%3Az123/branch/main/evaluate?transact=false',
        '/api/repository/did%3Akey%3Az123/branch/main/evaluate?transact=true',
      ],
    );
    const override = await call(
      'did:key:z123',
      'tonk_evaluate',
      JSON.stringify({ document: 'concept:', space: 'did:key:z456' }),
      fetch,
    );
    assert.ok(override.error);
    assert.equal(requests.length, 2);
    let writes = 0;
    const lost = await call(
      'did:key:z123',
      'tonk_evaluate',
      JSON.stringify({ document: 'concept:' }),
      async () => {
        writes++;
        throw Error('lost reply');
      },
    );
    assert.match(lost.error, /Do not repeat/);
    assert.equal(writes, 1);
    await writeFile(contextPath, first + '\n// stale\n');
    assert.equal(run('--check').status, 1);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
