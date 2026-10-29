// The hosted agent end to end, with no person and no LLM host: this script
// is the MCP client, and Playwright is the person approving it.
//
//   TONK_E2E_ORIGIN           the provider origin, e.g. a PR preview's workers.dev URL
//   TONK_E2E_RESEND_API_KEY   reads the activation email back (deployed origins)
//   TONK_E2E_CAPTURED_EMAILS  or: a local access service that captures it
//   TONK_E2E_GRANT_SPACE      the space granted read and write (default "Welcome")
//   TONK_E2E_HEADED=1         watch the browser
//   TONK_E2E_TRACE            where to write a Playwright trace when a step fails
//
// See "Testing end to end" in plans/2026-10-28-hosted-agent-peer.md.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { randomBytes } from 'node:crypto';
import {
  Client,
  StreamableHTTPClientTransport,
  UnauthorizedError,
} from '@modelcontextprotocol/client';
import { loopback, provider } from './lib/oauth.mjs';
import { inbox } from './lib/inbox.mjs';
import { launch, signUp, createSpace, syncedSpaces, approve } from './lib/browser.mjs';

const origin = new URL(process.env.TONK_E2E_ORIGIN ?? 'http://localhost:8787').origin;
const endpoint = new URL('/mcp', origin);
const run = process.env.GITHUB_RUN_ID
  ? `${process.env.GITHUB_RUN_ID}-${process.env.GITHUB_RUN_ATTEMPT ?? 1}`
  : randomBytes(4).toString('hex');
// The onboarding space, granted read and write; and one the test makes,
// granted read.
const writableName = process.env.TONK_E2E_GRANT_SPACE ?? 'Welcome';
const readableName = `e2e read ${run}`;
const agent = `e2e ${run}`;
// Writes per session, sent all at once.
const WRITES = 8;

const NOTE = `attribute!: &e2e-note-label
  description: Which run, session and write made this note
  the: e2e.note/label
  as: text
concept!: &e2e-note
  description: A note the end-to-end test writes
  with:
    label: e2e-note-label
`;
const note = (label) =>
  `e2e-note!:\n  this: urn:e2e:${label.replaceAll(' ', ':')}\n  label: "${label}"\n`;
const NOTES = 'e2e-note:\n  label: ?label\n';

test('the hosted agent connects, is approved, and reads and writes spaces', async (t) => {
  const callback = await loopback();
  const person = await launch({
    headed: !!process.env.TONK_E2E_HEADED,
    trace: process.env.TONK_E2E_TRACE,
  });
  let failed = true;
  const sessions = [];
  try {
    await t.test('/mcp refuses a request without a valid token', async () => {
      for (const authorization of [undefined, 'Bearer not-a-token']) {
        const response = await fetch(endpoint, {
          method: 'POST',
          headers: {
            'content-type': 'application/json',
            accept: 'application/json, text/event-stream',
            ...(authorization && { authorization }),
          },
          body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/list' }),
        });
        assert.equal(response.status, 401, `with ${authorization ?? 'no token'}`);
        assert.match(
          response.headers.get('www-authenticate') ?? '',
          /resource_metadata="[^"]+\/\.well-known\/oauth-protected-resource\/mcp"/,
        );
      }
    });

    // Subjects by name, of the spaces the person grants.
    let granted;
    await t.test('a fresh person signs up, confirms their email and makes a space', async () => {
      const mail = inbox({
        run,
        resendKey: process.env.TONK_E2E_RESEND_API_KEY,
        captured: process.env.TONK_E2E_CAPTURED_EMAILS,
      });
      await signUp(person.page, { origin, address: mail.address, inbox: mail });
      await createSpace(person.page, { origin, name: readableName });
      granted = await syncedSpaces(person.page, [writableName, readableName]);
    });

    let token;
    await t.test('OAuth runs to a token, approved on /settings/agent', async () => {
      let authorizationUrl;
      const auth = provider({
        redirectUrl: callback.url,
        name: agent,
        navigate: (url) => (authorizationUrl = url),
      });
      const transport = new StreamableHTTPClientTransport(endpoint, { authProvider: auth });
      await assert.rejects(
        new Client({ name: 'tonk-e2e', version: '0' }).connect(transport),
        UnauthorizedError,
      );
      assert.ok(authorizationUrl, 'the client was never sent to /authorize');
      assert.equal(new URL(authorizationUrl).origin, origin);

      await approve(person.page, {
        authorizationUrl,
        agent,
        grants: { [writableName]: 'Read and write', [readableName]: 'Read' },
      });
      const answer = await callback.answered;
      assert.ok(answer.get('code'), `the callback carried no code: ${answer}`);
      await transport.finishAuth(answer.get('code'));
      token = auth.tokens()?.access_token;
      assert.ok(token, 'no access token after the code was redeemed');
    });

    const connect = async () => {
      const client = new Client({ name: 'tonk-e2e', version: '0' });
      await client.connect(
        new StreamableHTTPClientTransport(endpoint, {
          requestInit: { headers: { authorization: `Bearer ${token}` } },
        }),
      );
      sessions.push(client);
      return client;
    };
    const call = async (client, name, args) => {
      const result = await client.callTool({ name, arguments: args });
      assert.notEqual(result.isError, true, `${name}: ${JSON.stringify(result.content)}`);
      return result;
    };

    // The agent sees exactly the spaces it was granted, and each allows
    // what it was granted.
    const spaces = {};
    await t.test('the agent lists its grants, reads both and writes only one', async () => {
      spaces.writable = granted[writableName];
      spaces.readable = granted[readableName];
      const client = await connect();
      const tools = (await client.listTools()).tools.map((tool) => tool.name);
      for (const name of ['tonk_list_spaces', 'tonk_query', 'tonk_evaluate', 'tonk_guide']) {
        assert.ok(tools.includes(name), `tools/list lacks ${name}: ${tools}`);
      }
      const listed = (await call(client, 'tonk_list_spaces', {})).structuredContent.spaces;
      assert.deepEqual(
        listed.map((space) => space.subject).sort(),
        [spaces.writable, spaces.readable].sort(),
        `listed ${JSON.stringify(listed)}`,
      );
      for (const space of [spaces.writable, spaces.readable]) {
        await call(client, 'tonk_query', { space, document: 'concept:\n' });
      }
      const refused = await client.callTool({
        name: 'tonk_evaluate',
        arguments: { space: spaces.readable, document: NOTE },
      });
      assert.equal(refused.isError, true, `${readableName} was granted Read but took a write`);
      await call(client, 'tonk_evaluate', { space: spaces.writable, document: NOTE });
    });

    const labels = [];
    await t.test('concurrent writes from two sessions on one token all land', async () => {
      const [first] = sessions;
      const second = await connect();
      const writes = [first, second].flatMap((client, session) =>
        Array.from({ length: WRITES }, (_, index) => {
          const label = `${run} ${session} ${index}`;
          labels.push(label);
          return call(client, 'tonk_evaluate', { space: spaces.writable, document: note(label) });
        }),
      );
      await Promise.all(writes);
    });

    await t.test('a fresh session reads every write back (dialog-db#705)', async () => {
      // A new MCP session is a new space writer: nothing it reads can come
      // from the memory of the sessions that wrote.
      const reader = await connect();
      const read = JSON.stringify(
        (await call(reader, 'tonk_query', { space: spaces.writable, document: NOTES }))
          .structuredContent,
      );
      const lost = labels.filter((label) => !read.includes(label));
      assert.deepEqual(lost, [], `${lost.length} of ${labels.length} writes were lost`);
    });
    failed = false;
  } finally {
    await Promise.allSettled(sessions.map((client) => client.close()));
    await callback.close();
    await person.close({ keepTrace: failed });
  }
});
