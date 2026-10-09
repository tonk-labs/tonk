import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { fileURLToPath } from 'node:url';
import { Client, StreamableHTTPClientTransport } from '@modelcontextprotocol/client';
import { startNativeRuntime, startNativeAccount } from './native.mjs';

const binary = process.env.TONK_MCP_RUNTIME;
test('private account adapter isolates identities, locks state and refuses development reuse', {
  skip: binary ? false : 'Set TONK_MCP_RUNTIME to the freshly built tonk-mcp-runtime binary.',
}, async () => {
  const dataDirectory = await mkdtemp(join(tmpdir(), 'tonk-hosted-account-'));
  const otherDirectory = await mkdtemp(join(tmpdir(), 'tonk-hosted-other-'));
  let account, other;
  try {
    account = await startNativeAccount({ binary, dataDirectory });
    other = await startNativeAccount({ binary, dataDirectory: otherDirectory });
    assert.notEqual(account.deviceDid, other.deviceDid);
    assert.equal(account.call, undefined);
    assert.equal(account.capabilities, undefined);
    await assert.rejects(account.listSpaces(), /Tool unavailable/);
    await assert.rejects(account.openSpace(account.deviceDid), /Tool unavailable/);
    const expected = { deviceDid: account.deviceDid, rootDid: null };
    assert.deepEqual(await account.status(), expected);
    await assert.rejects(startNativeAccount({ binary, dataDirectory }), /stopped/);
    await assert.rejects(account.authorize({ delegationHex: 'invalid' }), /not readable|not hex/);
    await assert.rejects(account.authorize({}, ''), /expectedAccount/);
    assert.deepEqual(await account.status(), expected);
    await account.close();
    account = await startNativeAccount({ binary, dataDirectory });
    assert.deepEqual(await account.status(), expected);
    await account.close();
    account = await startNativeAccount({ binary, dataDirectory, enableSpaces: true });
    await assert.rejects(account.listSpaces(), /no account is linked/);
    await assert.rejects(account.openSpace(account.deviceDid), /no account is linked/);
    await account.close();
    await assert.rejects(startNativeRuntime({ binary, dataDirectory }), /stopped/);
  } finally {
    await account?.close(); await other?.close();
    await rm(dataDirectory, { recursive: true, force: true });
    await rm(otherDirectory, { recursive: true, force: true });
  }
});
test('TCP HTTP MCP builds a real native reading list and retains an update after restart', {
  skip: binary ? false : 'Set TONK_MCP_RUNTIME to the freshly built tonk-mcp-runtime binary.',
  timeout: 30000,
}, async () => {
  const dataDirectory = await mkdtemp(join(tmpdir(), 'tonk-native-mcp-'));
  let service, stopped, client;
  async function connect() {
    service = spawn(process.execPath, [fileURLToPath(new URL('./dev-server.mjs', import.meta.url)), binary, dataDirectory], {
      env: { ...process.env, PORT: '0' }, stdio: ['ignore', 'ignore', 'pipe'],
    });
    stopped = once(service, 'exit');
    const endpoint = await new Promise((resolve, reject) => {
      let output = '';
      const timer = setTimeout(() => reject(new Error('HTTP listener did not become ready')), 10000);
      service.once('error', error => { clearTimeout(timer); reject(error); });
      service.once('exit', () => { clearTimeout(timer); reject(new Error('HTTP listener exited before readiness')); });
      service.stderr.on('data', chunk => {
        output = (output + chunk).slice(-4000);
        const match = output.match(/Tonk development MCP: (http:\/\/127\.0\.0\.1:\d+\/mcp)/);
        if (match) { clearTimeout(timer); resolve(match[1]); }
      });
    });
    client = new Client({ name: 'native-smoke', version: '1.0.0' });
    await client.connect(new StreamableHTTPClientTransport(new URL(endpoint)));
  }
  async function call(name, args) {
    const response = await client.callTool({ name, arguments: args });
    assert.notEqual(response.isError, true, JSON.stringify(response.content));
    return response.structuredContent;
  }
  async function disconnect() {
    await client?.close();
    client = undefined;
    if (service) { service.kill('SIGTERM'); await stopped; service = undefined; }
  }
  try {
    await connect();
    const document = `concept!: &book
  description: "A reading-list entry"
  with:
    title:
      description: "Book title"
      the: example.reading/title
      as: text
      cardinality: one
    finished:
      description: "Whether the book has been finished"
      the: example.reading/finished
      as: boolean
      cardinality: one

book!:
  this: urn:book:one
  title: "The Left Hand of Darkness"
  finished: false

book!:
  this: urn:book:two
  title: "A Wizard of Earthsea"
  finished: false

book!:
  this: urn:book:three
  title: "The Dispossessed"
  finished: false
`;
    const preview = await call('tonk_preview', { document });
    await call('tonk_apply', { document, expectedRevision: preview.revision });
    const shown = await call('tonk_show_query', { document: 'book:\n' });
    assert.equal(shown.matches[0].results.length, 3);
    const update = 'book!:\n  this: urn:book:one\n  finished: true\n';
    const next = await call('tonk_preview', { document: update });
    await call('tonk_apply', { document: update, expectedRevision: next.revision });
    const stale = await client.callTool({ name: 'tonk_apply', arguments: { document: update, expectedRevision: next.revision } });
    assert.equal(stale.isError, true);
    assert.match(stale.content[0].text, /changed since preview/);
    const libraryPreview = await call('tonk_install_library', { component: 'prose' });
    assert.equal(libraryPreview.committed, false);
    const libraryInstall = await call('tonk_install_library', { component: 'prose', expectedRevision: libraryPreview.revision });
    assert.equal(libraryInstall.committed, true);
    await disconnect();
    await connect();
    const libraryAgain = await call('tonk_install_library', { component: 'prose' });
    assert.equal(libraryAgain.alreadyInstalled, true);
    assert.equal(libraryAgain.committed, false);
    const persisted = await call('tonk_show_query', { document: 'book:\n' });
    assert.equal(persisted.matches[0].results.find(row => row.this === 'urn:book:one').fields.finished, true);
    const notebookPreview = await call('tonk_install_library', { component: 'notebook' });
    await call('tonk_install_library', { component: 'notebook', expectedRevision: notebookPreview.revision });
    const notebookDoc = `notebook!:
  this: urn:notebook:display
  title: "Visible notebook"
  block: {N9: urn:block:last, N1: urn:block:first}
notebook/block!:
  this: urn:block:first
  notebook: urn:notebook:display
  source: "# First block"
notebook/block!:
  this: urn:block:last
  notebook: urn:notebook:display
  source: "Second **block**"
`;
    const notePreview = await call('tonk_preview', { document: notebookDoc });
    await call('tonk_apply', { document: notebookDoc, expectedRevision: notePreview.revision });
    const notebook = await call('tonk_show_notebook', { entity: 'urn:notebook:display' });
    assert.equal(notebook.title, 'Visible notebook');
    assert.equal(notebook.markdown, '# First block\n\nSecond **block**');
    assert.equal(notebook.unplacedBlocks, 0);
    assert.equal(notebook.readOnly, true);
    assert.equal(notebook.url, undefined); // A local-only test space has no hosted URL.
    const updatedDocument = 'notebook/block!:\n  this: urn:block:last\n  source: "Updated body"\n';
    const editPreview = await call('tonk_preview', { document: updatedDocument });
    await call('tonk_apply', { document: updatedDocument, expectedRevision: editPreview.revision });
    const refreshed = await call('tonk_show_notebook', { entity: 'urn:notebook:display' });
    assert.equal(refreshed.markdown, '# First block\n\nUpdated body');
    await disconnect();
    await assert.rejects(startNativeAccount({ binary, dataDirectory }), /stopped/);
  } finally { await disconnect(); await rm(dataDirectory, { recursive: true, force: true }); }
});
