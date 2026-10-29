// The hosted agent end to end, with no person and no LLM host: this script
// is the MCP client, and Playwright is the person approving it.
//
//   TONK_E2E_ORIGIN           the provider origin, e.g. a PR preview's workers.dev URL
//   TONK_E2E_RESEND_API_KEY   reads the activation email back (deployed origins)
//   TONK_E2E_CAPTURED_EMAILS  or: a local access service that captures it
//   TONK_E2E_GRANT_SPACE      the space granted read-only (default "Welcome")
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
import { launch, signUp, approve } from './lib/browser.mjs';

const origin = new URL(process.env.TONK_E2E_ORIGIN ?? 'http://localhost:8787').origin;
const endpoint = new URL('/mcp', origin);
const run = process.env.GITHUB_RUN_ID
  ? `${process.env.GITHUB_RUN_ID}-${process.env.GITHUB_RUN_ATTEMPT ?? 1}`
  : randomBytes(4).toString('hex');
const granted = process.env.TONK_E2E_GRANT_SPACE ?? 'Welcome';
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

    await t.test('a fresh person signs up and confirms their email', async () => {
      const mail = inbox({
        run,
        resendKey: process.env.TONK_E2E_RESEND_API_KEY,
        captured: process.env.TONK_E2E_CAPTURED_EMAILS,
      });
      await signUp(person.page, { origin, address: mail.address, inbox: mail });
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

      await approve(person.page, { authorizationUrl, agent, grants: { [granted]: 'Read' } });
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

    // The token carries grant chains, not names, so the listed spaces are
    // told apart by what they allow: the space granted "Read" refuses a
    // write, and the agent's own space takes it.
    let spaces;
    await t.test('the agent reads its grants and cannot write a read-only one', async () => {
      const client = await connect();
      const tools = (await client.listTools()).tools.map((tool) => tool.name);
      for (const name of ['tonk_list_spaces', 'tonk_query', 'tonk_evaluate', 'tonk_guide']) {
        assert.ok(tools.includes(name), `tools/list lacks ${name}: ${tools}`);
      }
      const listed = (await call(client, 'tonk_list_spaces', {})).structuredContent.spaces;
      const readOnly = [],
        writable = [];
      for (const { subject } of listed) {
        await call(client, 'tonk_query', { space: subject, document: 'concept:\n' });
        const written = await client.callTool({
          name: 'tonk_evaluate',
          arguments: { space: subject, document: NOTE },
        });
        (written.isError ? readOnly : writable).push(subject);
      }
      const seen = JSON.stringify(listed);
      assert.equal(readOnly.length, 1, `expected one read-only space (${granted}): ${seen}`);
      assert.equal(writable.length, 1, `expected one writable space, the agent's: ${seen}`);
      spaces = { granted: readOnly[0], agent: writable[0] };
    });

    const labels = [];
    await t.test('concurrent writes from two sessions on one token all land', async () => {
      const [first] = sessions;
      const second = await connect();
      const writes = [first, second].flatMap((client, session) =>
        Array.from({ length: WRITES }, (_, index) => {
          const label = `${run} ${session} ${index}`;
          labels.push(label);
          return call(client, 'tonk_evaluate', { space: spaces.agent, document: note(label) });
        }),
      );
      await Promise.all(writes);
    });

    await t.test('a fresh session reads every write back (dialog-db#705)', async () => {
      // A new MCP session is a new space writer: nothing it reads can come
      // from the memory of the sessions that wrote.
      const reader = await connect();
      const read = JSON.stringify(
        (await call(reader, 'tonk_query', { space: spaces.agent, document: NOTES }))
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
