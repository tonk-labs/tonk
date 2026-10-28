// Per-call agent authority over the ordinary Rust worker.
import { spawn } from 'node:child_process';
import { TonkToolError } from './errors.mjs';

export function agentLinkBackend({
  binary,
  dataDirectory,
  remote,
  linkOrigin = new URL(remote).origin,
  run = invoke,
  checkpoint,
  maxPending = 32,
}) {
  // One adapter owns this directory. Serialize subprocesses so workers never
  // concurrently open the same replica. Different installations need own roots.
  const lifetime = new AbortController();
  let pending = Promise.resolve();
  let queued = 0,
    closed = false,
    drainQueued = true,
    failed = false;
  return {
    capabilities: ['tonk_query', 'tonk_evaluate'],
    agentLinkSelection: true,
    async close({ cancel = false, drain = true } = {}) {
      closed = true;
      drainQueued = drain;
      if (cancel) lifetime.abort();
      await pending;
    },
    // Hosted agent tools (`agent_create`, `agent_identity`, `agent_sponsor`).
    // They share this queue and checkpoint: agents live under the same root.
    async agent(input) {
      if (closed || failed || queued >= maxPending)
        throw new TonkToolError('Agent worker unavailable or busy.');
      if (
        ![
          'agent_create',
          'agent_identity',
          'agent_sponsor',
          'agent_create_space',
          'agent_call',
        ].includes(input?.tool)
      )
        throw new TonkToolError('Tool unavailable.');
      queued++;
      const task = pending.then(async () => {
        if (failed) throw new TonkToolError('Agent storage requires recovery.');
        if (closed && !drainQueued) throw new TonkToolError('Agent worker is shutting down.');
        let result;
        try {
          result = await run({ binary, dataDirectory, remote, signal: lifetime.signal, input });
        } finally {
          if (checkpoint && input.tool !== 'agent_identity') {
            try {
              await checkpoint.save();
            } catch {
              failed = true;
              throw new TonkToolError('Durable storage failed.');
            }
          }
        }
        if (result.error) throw new TonkToolError(result.error);
        return result.result;
      });
      pending = task
        .catch(() => {})
        .finally(() => {
          queued--;
        });
      return task;
    },
    async call(tool, args, signal) {
      if (closed || failed || queued >= maxPending)
        throw new TonkToolError('Agent worker unavailable or busy.');
      if (
        !['tonk_query', 'tonk_evaluate'].includes(tool) ||
        !args ||
        Object.keys(args).length !== 2 ||
        typeof args.agent_link !== 'string' ||
        !args.agent_link ||
        args.agent_link.length > 160000 ||
        typeof args.document !== 'string' ||
        !args.document.trim() ||
        Buffer.byteLength(args.document) > 32000
      ) {
        throw new TonkToolError(
          'Provide an agent_link and one inline notation document, up to 32 KB.',
        );
      }
      signal = signal ? AbortSignal.any([signal, lifetime.signal]) : lifetime.signal;
      queued++;
      const task = pending.then(async () => {
        if (failed) throw new TonkToolError('Agent storage requires recovery.');
        if (closed && !drainQueued) throw new TonkToolError('Agent worker is shutting down.');
        signal?.throwIfAborted();
        let link;
        try {
          const url = new URL(args.agent_link);
          if (url.pathname.startsWith('/@/')) {
            if (url.origin !== linkOrigin || url.username || url.password || !url.hash)
              throw Error('Invalid shortcut');
            const fragment = url.hash;
            url.hash = '';
            const response = await fetch(url, {
              redirect: 'manual',
              signal: signal
                ? AbortSignal.any([signal, AbortSignal.timeout(5000)])
                : AbortSignal.timeout(5000),
            });
            await response.body?.cancel();
            const location = response.headers.get('location');
            if (
              ![301, 302, 303, 307, 308].includes(response.status) ||
              !location ||
              location.length > 160000
            )
              throw Error('Invalid redirect');
            const resolved = new URL(location, url);
            if (
              resolved.origin !== linkOrigin ||
              resolved.username ||
              resolved.password ||
              resolved.hash ||
              resolved.pathname.startsWith('/@/')
            )
              throw Error('Invalid redirect');
            resolved.hash = fragment;
            link = resolved.href;
          } else link = url.href;
        } catch {
          // Operator diagnostics only: never log the link, which carries the
          // capability in its fragment. stderr keeps stdio transports intact.
          console.error('Agent link expansion failed.');
          throw new TonkToolError('Could not resolve the agent link.');
        }
        signal?.throwIfAborted();
        let result;
        try {
          result = await run({
            binary,
            dataDirectory,
            remote,
            // Once dispatched, finish a write and checkpoint even if its HTTP
            // client disappears. Killing it mid-publication creates avoidable
            // unknown outcomes. Explicit host cancellation and the native
            // process deadline still bound execution; queued calls remain cancellable.
            signal: tool === 'tonk_evaluate' ? lifetime.signal : signal,
            input: { tool, agent_link: link, document: args.document },
          });
        } finally {
          if (checkpoint) {
            try {
              await checkpoint.save();
            } catch {
              failed = true;
              throw new TonkToolError(
                'Durable storage failed. Evaluation outcome may be unknown; do not repeat the write.',
              );
            }
          }
        }
        if (result.error) throw new TonkToolError(result.error);
        return result.result;
      });
      pending = task
        .catch(() => {})
        .finally(() => {
          queued--;
        });
      return task;
    },
  };
}

// Signals whose names are safe to report in subprocess diagnostics.
const SAFE_SIGNALS = new Set([
  'SIGTERM',
  'SIGKILL',
  'SIGABRT',
  'SIGSEGV',
  'SIGBUS',
  'SIGILL',
  'SIGPIPE',
  'SIGINT',
  'SIGHUP',
  'SIGQUIT',
  'SIGTRAP',
  'SIGFPE',
]);

export function invoke({ binary, dataDirectory, remote, input, signal, timeoutMs = 120000 }) {
  return new Promise((resolve, reject) => {
    const timeout = AbortSignal.timeout(timeoutMs);
    const abort = signal ? AbortSignal.any([signal, timeout]) : timeout;
    const child = spawn(binary, [dataDirectory, remote], {
      stdio: ['pipe', 'pipe', 'pipe'],
      env: { PATH: process.env.PATH, DO_NOT_TRACK: '1' },
      signal: abort,
    });
    let output = '';
    let killTimer;
    const terminate = () => {
      child.kill();
      killTimer ??= setTimeout(() => child.kill('SIGKILL'), 3000);
      killTimer.unref();
    };
    abort.addEventListener('abort', terminate, { once: true });
    let failure;
    const fail = (category, code, exitSignal) => {
      // Never include child stderr, stdin, URLs, paths or exception messages.
      const status = Number.isInteger(code) && code >= 0 && code <= 255 ? code : null;
      const safeSignal = SAFE_SIGNALS.has(exitSignal) ? exitSignal : null;
      console.error(
        'Agent worker subprocess failed:',
        JSON.stringify({ category, status, signal: safeSignal }),
      );
      reject(
        new TonkToolError(
          input.tool === 'tonk_evaluate' || input.call === 'tonk_evaluate'
            ? 'Evaluation could not be confirmed. Do not repeat it; query the space first.'
            : 'The worker query could not be completed.',
        ),
      );
    };
    child.stderr.resume();
    // Settle only after close: even cancellation must not allow the next call
    // to open this worker's storage while its process is still exiting.
    child.on('error', () => {
      failure ??= abort.aborted ? (timeout.aborted ? 'timeout' : 'cancelled') : 'spawn_error';
    });
    child.stdin.on('error', () => {
      failure ??= 'stdin_error';
      terminate();
    });
    child.stdout.setEncoding('utf8');
    child.stdout.on('data', (chunk) => {
      if (failure) return;
      output += chunk;
      if (Buffer.byteLength(output) > 2100000) {
        terminate();
        failure = 'output_limit';
      }
    });
    child.on('close', (code, exitSignal) => {
      clearTimeout(killTimer);
      abort.removeEventListener('abort', terminate);
      if (failure || code !== 0) {
        const category =
          failure ?? (abort.aborted ? (timeout.aborted ? 'timeout' : 'cancelled') : 'exit');
        return fail(category, code, exitSignal);
      }
      try {
        const last = output.trimEnd().split('\n').at(-1);
        const marker = 'TONK_AGENT_LINK_RESULT ';
        if (!last.startsWith(marker)) return fail('missing_result', code, exitSignal);
        resolve(JSON.parse(last.slice(marker.length)));
      } catch {
        fail('invalid_result', code, exitSignal);
      }
    });
    child.stdin.end(JSON.stringify(input));
  });
}
