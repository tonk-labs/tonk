// Sponsored agents' peers held open while their views are shown in a chat.
//
// A view needs a worker that stays up: the runtime in the app subscribes to
// queries and expects them to stream. Each open agent runs one
// `tonk-agent-worker serve` process over a copy of its directory. The copy
// keeps the agent store's checkpoint away from storage a process has open.
// While an agent is open, every tool call for it goes to that process, so two
// processes never act as the same agent at once. Views write as the agent.
import { spawn } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { once } from 'node:events';
import { cp, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, join } from 'node:path';
import { TonkToolError } from '../errors.mjs';

const AGENT = /^[0-9a-f]{32}$/;
const uncertain = 'Evaluation could not be confirmed. Do not repeat it; query the space first.';

// `exclusive(task)` runs `task` in the agent store's queue, so the copy is
// never taken while another process writes the agent's directory.
export function createAgentViews({
  binary,
  directory,
  remote,
  exclusive,
  start = startServedAgent,
  idle = 10 * 60_000,
  capacity = 8,
  now = Date.now,
}) {
  const open = new Map();
  let closed = false;

  const sweepIdle = () =>
    Promise.all(
      [...open]
        .filter(([, entry]) => entry.ready && entry.active === 0 && now() - entry.used >= idle)
        .map(([agent]) => stop(agent)),
    );
  const sweep = setInterval(sweepIdle, 30_000);
  sweep.unref?.();

  function stop(agent) {
    const entry = open.get(agent);
    if (!entry) return Promise.resolve();
    open.delete(agent);
    return entry.started.then(
      (served) => served.close(),
      () => {},
    );
  }

  function openAgent(agent) {
    if (closed) throw new TonkToolError('Agent views are shutting down.');
    if (!AGENT.test(agent)) throw new TonkToolError('Unknown agent.');
    const existing = open.get(agent);
    if (existing) {
      existing.used = now();
      return existing.started;
    }
    if (open.size >= capacity) throw new TonkToolError('Too many open views. Try again shortly.');
    const entry = { active: 0, used: now(), ready: false };
    entry.started = exclusive(async () => {
      const live = await mkdtemp(join(tmpdir(), 'tonk-agent-view-'));
      try {
        await cp(join(directory, 'agents', agent), live, {
          recursive: true,
          filter: (source) => basename(source) !== 'peer.lock',
        });
        const served = await start({ binary, directory: live, remote });
        return withUse(entry, served, live);
      } catch (error) {
        await rm(live, { recursive: true, force: true });
        throw error;
      }
    });
    entry.started.then(
      () => {
        entry.ready = true;
      },
      () => {
        if (open.get(agent) === entry) open.delete(agent);
      },
    );
    open.set(agent, entry);
    return entry.started;
  }

  // Count in-flight requests and open streams, so an agent with a view still
  // streaming is never closed as idle.
  function withUse(entry, served, live) {
    const begin = () => {
      entry.active++;
      entry.used = now();
      let done = false;
      return () => {
        if (done) return;
        done = true;
        entry.active--;
        entry.used = now();
      };
    };
    return {
      async call(call, args) {
        const end = begin();
        try {
          return await served.call(call, args);
        } finally {
          end();
        }
      },
      async request(path, options = {}) {
        const end = begin();
        let response;
        try {
          response = await served.request(path, options);
          response = await publishWrite(served, path, options, response);
        } catch (error) {
          end();
          throw error;
        }
        if (!response.body) {
          end();
          return response;
        }
        const reader = response.body.getReader();
        const body = new ReadableStream({
          async pull(controller) {
            try {
              const { done, value } = await reader.read();
              if (done) {
                end();
                controller.close();
              } else controller.enqueue(value);
            } catch (error) {
              end();
              controller.error(error);
            }
          },
          cancel(reason) {
            end();
            return reader.cancel(reason);
          },
        });
        return new Response(body, { status: response.status, headers: response.headers });
      },
      async close() {
        await served.close();
        await rm(live, { recursive: true, force: true });
      },
    };
  }

  return {
    open: openAgent,
    // The open agent, or nothing: a call for an agent with no view runs as
    // a one-shot process instead.
    running(agent) {
      const entry = open.get(agent);
      return entry ? entry.started : undefined;
    },
    stop,
    // Close agents with no view in use for `idle`; runs on a timer.
    sweepIdle,
    async close() {
      closed = true;
      clearInterval(sweep);
      await Promise.all([...open.keys()].map(stop));
    },
  };
}

// Publish a write from a view at once, rather than at the next background
// sync, and say so when publishing could not be confirmed. Never replay it.
async function publishWrite(served, path, options, response) {
  const url = new URL(path, 'http://worker');
  const match = url.pathname.match(
    /^\/api\/repository\/([^/]+)\/branch\/main\/(transact|evaluate)$/,
  );
  const writes =
    match &&
    options.method === 'POST' &&
    (match[2] === 'transact' || url.searchParams.get('transact') !== 'false');
  if (!writes || !response.ok) return response;
  const pushed = await served.request('/api/repository/' + match[1] + '/branch/main/sync/push', {
    method: 'POST',
  });
  await pushed.body?.cancel();
  if (pushed.ok) return response;
  await response.body?.cancel();
  return Response.json(
    { error: 'Applied locally; remote push could not be confirmed. Read before retrying.' },
    { status: 503 },
  );
}

// One `tonk-agent-worker serve` process. Its worker answers only requests
// carrying this process's token; tool calls go as JSON lines on stdin.
export async function startServedAgent({ binary, directory, remote }) {
  const token = randomBytes(32).toString('base64url');
  const child = spawn(binary, [directory, remote, 'serve'], {
    stdio: ['pipe', 'pipe', 'ignore'],
    env: { PATH: process.env.PATH, TONK_WORKER_HOST_TOKEN: token, DO_NOT_TRACK: '1' },
  });
  const pending = new Map();
  let next = 0,
    buffer = '',
    exited = false;
  const stopped = once(child, 'exit').then(() => {
    exited = true;
    for (const { reject } of pending.values()) reject(Error('Served agent stopped'));
    pending.clear();
  });
  child.stdin.on('error', () => {});
  const address = await new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      child.kill();
      reject(new TonkToolError('The agent could not open its views.'));
    }, 60_000);
    stopped.then(() => {
      clearTimeout(timer);
      reject(new TonkToolError('The agent could not open its views.'));
    });
    child.stdout.setEncoding('utf8');
    child.stdout.on('data', (chunk) => {
      buffer += chunk;
      for (let end; (end = buffer.indexOf('\n')) >= 0; ) {
        const line = buffer.slice(0, end);
        buffer = buffer.slice(end + 1);
        // The worker logs to stdout too; only marked lines are read.
        let value;
        try {
          if (line.startsWith('TONK_AGENT_SERVE ')) {
            value = JSON.parse(line.slice('TONK_AGENT_SERVE '.length));
            clearTimeout(timer);
            if (/^127\.0\.0\.1:\d+$/.test(value.address ?? '')) resolve(value.address);
            else {
              child.kill();
              reject(new TonkToolError(value.error ?? 'The agent could not open its views.'));
            }
          } else if (line.startsWith('TONK_AGENT_CALL ')) {
            value = JSON.parse(line.slice('TONK_AGENT_CALL '.length));
            const waiting = pending.get(value.id);
            if (!waiting) continue;
            pending.delete(value.id);
            if (typeof value.error === 'string') waiting.reject(new TonkToolError(value.error));
            else waiting.resolve(value.result);
          }
        } catch {}
      }
      if (buffer.length > 4_000_000) child.kill();
    });
  });
  const origin = 'http://' + address;
  return {
    call(call, args = {}) {
      if (exited) return Promise.reject(new TonkToolError('The agent is restarting. Retry.'));
      const id = ++next;
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
          pending.delete(id);
          reject(
            new TonkToolError(
              call === 'tonk_evaluate' ? uncertain : 'The worker query could not be completed.',
            ),
          );
        }, 180_000);
        pending.set(id, {
          resolve: (value) => {
            clearTimeout(timer);
            resolve(value);
          },
          reject: (error) => {
            clearTimeout(timer);
            reject(
              error instanceof TonkToolError
                ? error
                : new TonkToolError(
                    call === 'tonk_evaluate'
                      ? uncertain
                      : 'The worker query could not be completed.',
                  ),
            );
          },
        });
        child.stdin.write(JSON.stringify({ id, call, arguments: args }) + '\n');
      });
    },
    async request(path, options = {}) {
      if (exited || !path.startsWith('/') || path.startsWith('//'))
        throw Error('Worker unavailable');
      const url = new URL(path, origin);
      if (url.origin !== origin) throw Error('Invalid worker path');
      const headers = new Headers(options.headers);
      headers.set('authorization', 'Bearer ' + token);
      return fetch(url, { ...options, headers, redirect: 'error' });
    },
    async close() {
      if (!exited) {
        child.stdin.end();
        const timer = setTimeout(() => child.kill(), 10_000);
        await stopped;
        clearTimeout(timer);
      }
    },
  };
}
