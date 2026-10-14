import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, writeFile, rm, cp, symlink } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, dirname, resolve } from 'node:path';
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

test('Desktop check detects a changed canonical manual before native regeneration', async () => {
  const root = await mkdtemp(join(tmpdir(), 'tonk-desktop-manual-'));
  const sourceRoot = fileURLToPath(new URL('../', import.meta.url));
  try {
    await mkdir(join(root, 'mcp/scripts'), { recursive: true });
    await mkdir(join(root, 'desktop/Sources/HarnessCore'), { recursive: true });
    for (const name of [
      'catalog.mjs',
      'browser.mjs',
      'primitive.mjs',
      'errors.mjs',
      'authoring.mjs',
      'authoring-content.mjs',
      'tools.json',
      'scripts/sync-desktop.mjs',
      'scripts/build-authoring-guides.mjs',
    ]) {
      await cp(join(sourceRoot, 'mcp', name), join(root, 'mcp', name));
    }
    await symlink(join(sourceRoot, 'mcp/node_modules'), join(root, 'mcp/node_modules'));
    const guidePath = 'rust/tonk-cli/src/guide.rs';
    const guide = await readFile(join(sourceRoot, guidePath), 'utf8');
    for (const path of [
      guidePath,
      ...[...guide.matchAll(/include_str!\("([^"]+)"\)/g)].map((m) =>
        resolve(dirname(join(root, guidePath)), m[1]).slice(root.length + 1),
      ),
    ]) {
      await mkdir(dirname(join(root, path)), { recursive: true });
      await cp(join(sourceRoot, path), join(root, path));
    }
    const run = (...args) =>
      spawnSync(process.execPath, [join(root, 'mcp/scripts/sync-desktop.mjs'), ...args], {
        encoding: 'utf8',
      });
    assert.equal(run().status, 0);
    assert.equal(run('--check').status, 0);
    const manual = join(root, 'rust/tonk-cli/src/guide-agent.md');
    await writeFile(manual, (await readFile(manual, 'utf8')) + '\nManual freshness sentinel.\n');
    const stale = run('--check');
    assert.equal(stale.status, 1);
    assert.match(stale.stderr, /Authoring bundle is stale/);
    assert.equal(run().status, 0);
    assert.equal(run('--check').status, 0);
    assert.match(
      await readFile(
        join(root, 'desktop/Sources/HarnessCore/TonkAgentContext.generated.swift'),
        'utf8',
      ),
      /Manual freshness sentinel/,
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
