// The hosted agent end to end, with no person and no LLM host: this script
// is the MCP client, and Playwright is the person approving it.
//
//   TONK_E2E_ORIGIN           the provider origin, e.g. a PR preview's workers.dev URL
//   TONK_E2E_CAPTURED_EMAILS  where activation emails are read back; defaults to the
//                             origin (a preview records mail to `.test` addresses)
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
import {
  launch,
  boot,
  passkeyOf,
  signUp,
  createSpace,
  syncedSpaces,
  approve,
} from './lib/browser.mjs';
import { showView } from './lib/view-host.mjs';

const origin = new URL(process.env.TONK_E2E_ORIGIN ?? 'http://localhost:8787').origin;
const endpoint = new URL('/mcp', origin);
const run = process.env.GITHUB_RUN_ID
  ? `${process.env.GITHUB_RUN_ID}-${process.env.GITHUB_RUN_ATTEMPT ?? 1}`
  : randomBytes(4).toString('hex');
// Two spaces the test makes: one granted read and write, one read.
const writableName = `e2e write ${run}`;
const readableName = `e2e read ${run}`;
const agent = `e2e ${run}`;
// Writes per session, sent all at once.
const WRITES = 8;
// What approving may upload before it answers: the agent space and its
// grants (30 to 60 measured), plus mounting the chosen spaces in a browser
// that never had them (about 100), but never the agent's library, which
// is about 190 more.
const APPROVAL_UPLOADS = 80;
const NEW_BROWSER_APPROVAL_UPLOADS = 180;

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
// How long a write the agent makes may take to reach an open view: the
// provider's `live.rs` tells the view's peer, which pulls.
const LIVE = 30_000;

test('the hosted agent connects, is approved, and reads and writes spaces', async (t) => {
  const mail = inbox({ run, base: process.env.TONK_E2E_CAPTURED_EMAILS || origin });
  let elsewhere;
  const person = await launch({
    headed: !!process.env.TONK_E2E_HEADED,
    trace: process.env.TONK_E2E_TRACE,
  });
  // Each step needs the one before it, so the first failure skips the rest
  // rather than burying its cause under theirs.
  let failed = false;
  const step = (name, run) =>
    t.test(name, { skip: failed && 'an earlier step failed' }, async () => {
      try {
        await run();
      } catch (error) {
        failed = true;
        throw error;
      }
    });
  const sessions = [];
  try {
    await step('/mcp refuses a request without a valid token', async () => {
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
    await step('a fresh person signs up, confirms their email and makes two spaces', async () => {
      await signUp(person.page, { origin, address: mail.address, inbox: mail });
      await createSpace(person.page, { origin, name: writableName });
      await createSpace(person.page, { origin, name: readableName });
      granted = await syncedSpaces(person.page, [writableName, readableName]);
    });

    // OAuth as a host runs it, approved in `browser`: the token.
    const authorize = async (browser, { name, grants, signIn, budget = APPROVAL_UPLOADS }) => {
      const callback = await loopback();
      try {
        let authorizationUrl;
        const auth = provider({
          redirectUrl: callback.url,
          name,
          navigate: (url) => (authorizationUrl = url),
        });
        const transport = new StreamableHTTPClientTransport(endpoint, { authProvider: auth });
        await assert.rejects(
          new Client({ name: 'tonk-e2e', version: '0' }).connect(transport),
          UnauthorizedError,
        );
        assert.ok(authorizationUrl, 'the client was never sent to /authorize');
        assert.equal(new URL(authorizationUrl).origin, origin);

        const cost = await approve(browser.page, { authorizationUrl, agent: name, grants, signIn });
        const answer = await callback.answered;
        // Approval has to finish while the request is still pending, on a
        // person's link rather than CI's: it may only push the grants, not
        // a whole space's history.
        const spent = cost.stop();
        console.log(`approval: ${JSON.stringify(spent)}`);
        assert.ok(
          spent.uploads <= budget,
          `approval made ${spent.uploads} uploads (budget ${budget})`,
        );
        assert.ok(answer.get('code'), `the callback carried no code: ${answer}`);
        // The whole callback, so the SDK checks `iss` (RFC 9207) too.
        await transport.finishAuth(answer);
        const token = auth.tokens()?.access_token;
        assert.ok(token, 'no access token after the code was redeemed');
        return token;
      } finally {
        await callback.close();
      }
    };

    let token;
    await step('OAuth runs to a token, approved on /settings/agent', async () => {
      token = await authorize(person, {
        name: agent,
        grants: { [writableName]: 'read & write', [readableName]: 'read' },
      });
    });

    const connect = async (bearer = token) => {
      const client = new Client({ name: 'tonk-e2e', version: '0' });
      await client.connect(
        new StreamableHTTPClientTransport(endpoint, {
          requestInit: { headers: { authorization: `Bearer ${bearer}` } },
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
    await step('the agent lists its grants, reads both and writes only one', async () => {
      spaces.writable = granted[writableName];
      spaces.readable = granted[readableName];
      const client = await connect();
      const tools = (await client.listTools()).tools.map((tool) => tool.name);
      for (const name of ['tonk_list_spaces', 'tonk_query', 'tonk_evaluate', 'tonk_guide']) {
        assert.ok(tools.includes(name), `tools/list lacks ${name}: ${tools}`);
      }
      const listed = (await call(client, 'tonk_list_spaces', {})).structuredContent.spaces;
      // The agent's own space is listed too, marked as such. The rest come
      // from the grants the approval kept in it.
      assert.equal(
        listed.filter((space) => space.home).length,
        1,
        `listed ${JSON.stringify(listed)}`,
      );
      assert.deepEqual(
        Object.fromEntries(
          listed
            .filter((space) => !space.home)
            .map(({ subject, name, access }) => [subject, { name, access }]),
        ),
        {
          [spaces.writable]: { name: writableName, access: 'read-write' },
          [spaces.readable]: { name: readableName, access: 'read' },
        },
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
    await step('concurrent writes from two sessions on one token all land', async () => {
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

    await step('a fresh session reads every write back (dialog-db#705)', async () => {
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

    // A view, as a host shows `tonk_open_space`: first as a host that
    // frames nothing (route B, the view's own peer in a Web Worker), then
    // as one that frames the provider (route A, the provider's page with
    // its service worker as the peer). Each must show what the agent
    // wrote, then a write it makes while the view is open.
    for (const [route, frames] of [
      ['B', false],
      ['A', true],
    ]) {
      await step(
        `a shown view renders the space and follows a write (route ${route})`,
        async () => {
          const client = await connect();
          const open = (await client.listTools()).tools.find(
            (tool) => tool.name === 'tonk_open_space',
          );
          assert.ok(open, 'tools/list lacks tonk_open_space');
          const resource = await client.readResource({ uri: open._meta.ui.resourceUri });
          // The notes' directory page: every e2e-note, each with its label.
          const result = await call(client, 'tonk_open_space', {
            space: spaces.writable,
            path: '/e2e-note',
          });
          const view = await showView({
            client,
            result,
            resource,
            frames,
            headed: !!process.env.TONK_E2E_HEADED,
          });
          try {
            const started = Date.now();
            await view.shows(labels.slice(0, 3));
            console.log(`route ${route}: shown in ${Date.now() - started} ms`);
            assert.ok(view.calls.includes('tonk_view_access'), `the view called ${view.calls}`);
            const framed = view
              .frames()
              .some((frame) => frame.url().startsWith(`${origin}/view-frame`));
            assert.equal(
              framed,
              frames,
              frames ? 'the view fell back to route B' : 'the view framed the provider',
            );
            const live = `${run} live ${route}`;
            const wrote = Date.now();
            await call(client, 'tonk_evaluate', { space: spaces.writable, document: note(live) });
            await view.shows([live], LIVE);
            console.log(`route ${route}: a write showed in ${Date.now() - wrote} ms`);
          } finally {
            if (view.violations.length)
              console.log(`route ${route} CSP: ${view.violations.join('\n')}`);
            await view.close();
          }
        },
      );
    }

    // What a person who made their spaces earlier meets: a browser with
    // no account on it, so the approval page signs them in first and has
    // to list spaces this browser has never seen.
    await step(
      'the same person approves another agent from a browser with no account on it',
      async () => {
        await boot(person.page, origin);
        elsewhere = await launch({
          headed: !!process.env.TONK_E2E_HEADED,
          passkey: await passkeyOf(person),
        });
        const again = await authorize(elsewhere, {
          name: `${agent} elsewhere`,
          grants: { [readableName]: 'read' },
          signIn: mail.address,
          budget: NEW_BROWSER_APPROVAL_UPLOADS,
        });
        const listed = (await call(await connect(again), 'tonk_list_spaces', {})).structuredContent
          .spaces;
        assert.deepEqual(
          listed.filter((space) => !space.home).map(({ subject, access }) => ({ subject, access })),
          [{ subject: granted[readableName], access: 'read' }],
          `listed ${JSON.stringify(listed)}`,
        );
      },
    );
  } finally {
    await Promise.allSettled(sessions.map((client) => client.close()));
    await person.close({ keepTrace: failed });
    await elsewhere?.close();
  }
});
