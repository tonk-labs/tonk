import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { Client, StreamableHTTPClientTransport } from '@modelcontextprotocol/client';
import { authoringGuide } from '../authoring.mjs';
import { startWorker } from '../worker-process.mjs';
import { createAgentTools } from '../browser.mjs';
import { createTonkHTTP } from '../http.mjs';

assert.ok(
  process.env.TONK_WORKER_BINARY,
  'Build tonk-worker-host and set TONK_WORKER_BINARY; integration tests must not skip.',
);

test('generic MCP client authors and queries the ordinary native worker', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'tonk-authoring-'));
  const worker = await startWorker({
    binary: process.env.TONK_WORKER_BINARY,
    dataDirectory: directory,
  });
  let app;
  let client;
  try {
    const created = await (
      await worker.request('/api/repository/authoring-test', {
        method: 'PUT',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ branch: { main: {} } }),
      })
    ).json();
    const subject = created.name.startsWith('did:') ? created.name : 'did:key:' + created.name;
    const agent = createAgentTools({
      subject,
      request: (path, options) => worker.request(path, options),
    });
    app = createTonkHTTP(async () => ({
      capabilities: agent.tools.map((tool) => tool.name),
      call: agent.call,
    }));
    client = new Client({ name: 'native-core-integration', version: '1' });
    await client.connect(
      new StreamableHTTPClientTransport(new URL('http://localhost/mcp'), {
        fetch: (url, options) => app.fetch(new Request(url, options)),
      }),
    );
    assert.deepEqual((await client.listTools()).tools.map((tool) => tool.name).sort(), [
      'tonk_evaluate',
      'tonk_guide',
      'tonk_query',
    ]);
    const backend = {
      async call(name, args) {
        const result = await client.callTool({ name, arguments: args });
        assert.notEqual(result.isError, true, JSON.stringify(result.content));
        return result.structuredContent;
      },
    };
    await backend.call('tonk_evaluate', {
      document: await readFile(
        new URL('../fixtures/authoring/tasks.notation', import.meta.url),
        'utf8',
      ),
    });
    const guide = authoringGuide({ topic: 'views' }).text;
    const example = (name) => {
      const marker = '<!-- tested-example: ' + name + ' -->';
      const block = guide.split(marker)[1]?.match(/```yaml tonk=parse\n([\s\S]*?)```/);
      assert.ok(block, 'missing executable guide example ' + name);
      return block[1];
    };
    await backend.call('tonk_evaluate', {
      document: example('task-views') + '\n' + example('space-home'),
    });
    const home = await backend.call('tonk_query', { document: 'name:\n  this: id:tonk/space\n' });
    assert.equal(home.matches_after[0].results[0].fields.entity, 'space:home');
    const homeView = await backend.call('tonk_query', {
      document: 'xyz.tonk.view:\n  this: space:home\n  ui: _\n',
    });
    assert.match(homeView.matches_after[0].results[0].fields.ui, /model=integration-task/);
    const views = await backend.call('tonk_query', {
      document: 'xyz.tonk.view:\n  this: integration-task\n  ui: _\n  directory: _\n',
    });
    const fields = views.matches_after[0].results[0].fields;
    assert.match(fields.ui, /checked=\{done\}/);
    assert.match(fields.directory, /model=integration-task view=ui/);
    const result = await backend.call('tonk_query', {
      document: 'integration-task:\n  title: ?title\n  done: ?done\n',
    });
    const matches = result.matches_after ?? result.matches_before;
    assert.equal(matches[0].results.length, 3);
    const site = await worker.request(
      '/api/repository/' + encodeURIComponent(subject) + '/branch/main/site',
      {
        method: 'POST',
        headers: { 'content-type': 'application/json', 'x-tonk-client-id': 'authoring-test' },
        body: JSON.stringify({ path: '/integration-task' }),
      },
    );
    assert.equal(site.status, 200, await site.text());
    const root = await worker.request(
      '/api/repository/' + encodeURIComponent(subject) + '/branch/main/site',
      {
        method: 'POST',
        headers: { 'content-type': 'application/json', 'x-tonk-client-id': 'authoring-test' },
        body: JSON.stringify({ path: '/' }),
      },
    );
    assert.equal(root.status, 200, await root.text());
    const values = matches[0].results.map((row) => row.fields.done);
    assert.deepEqual(values.sort(), [false, false, true]);
    await backend.call('tonk_query', {
      document: 'integration-task!:\n  this: urn:test:task:one\n  title: "Must not persist"\n',
    });
    const unchanged = await backend.call('tonk_query', {
      document: 'integration-task:\n  this: urn:test:task:one\n',
    });
    assert.equal(unchanged.matches_after[0].results[0].fields.title, 'First task');
    await backend.call('tonk_evaluate', {
      document: authoringGuide({ topic: 'spaces' })
        .text.split('<!-- tested-example: rename-space -->')[1]
        .match(/```yaml tonk=parse\n([\s\S]*?)```/)[1]
        .replace('urn:example:authorized-space', subject)
        .replace('New space name', 'Workflow renamed'),
    });
    const renamed = await backend.call('tonk_query', {
      document: 'tonk/repository:\n  this: ' + subject + '\n',
    });
    assert.equal(renamed.matches_after[0].results[0].fields.name, 'Workflow renamed');
    await backend.call('tonk_evaluate', {
      document: 'library/install!:\n  component: https://tonk.foundation/library/notebook.yaml\n  time: 1.0\n',
    });
    const library = await backend.call('tonk_query', {
      document: 'concept:\n  name: "notebook/named"\n',
    });
    assert.equal(library.matches_after[0].results.length, 1);
  } finally {
    await client?.close();
    await app?.close();
    await worker.close();
    await rm(directory, { recursive: true, force: true });
  }
});
