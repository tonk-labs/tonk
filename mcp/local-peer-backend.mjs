// A persistent local peer over the ordinary Rust worker. The peer keeps its
// own identity in its data directory; share links are redeemed into it once
// with tonk_join, and later calls name a space instead of carrying a link.
import { TonkToolError } from './errors.mjs';
import { invoke, resolveShortcut } from './agent-link-backend.mjs';

const tools = ['tonk_list_spaces', 'tonk_join', 'tonk_query', 'tonk_evaluate'];

export function localPeerBackend({
  binary,
  dataDirectory,
  remote,
  linkOrigin = new URL(remote).origin,
  run = invoke,
  maxPending = 32,
}) {
  // One adapter process owns its calls; the worker additionally holds an
  // exclusive lock on the directory, so other hosts sharing this peer on the
  // same machine take turns rather than racing.
  const lifetime = new AbortController();
  let pending = Promise.resolve();
  let queued = 0,
    closed = false;
  return {
    capabilities: tools,
    spaceSelection: true,
    async close({ cancel = false } = {}) {
      closed = true;
      if (cancel) lifetime.abort();
      await pending;
    },
    async call(tool, args, signal) {
      if (closed || queued >= maxPending)
        throw new TonkToolError('Local peer unavailable or busy.');
      if (!tools.includes(tool) || !args || typeof args !== 'object')
        throw new TonkToolError('Tool unavailable.');
      const input = { tool, arguments: shape(tool, args) };
      signal = signal ? AbortSignal.any([signal, lifetime.signal]) : lifetime.signal;
      queued++;
      const task = pending.then(async () => {
        signal.throwIfAborted();
        if (tool === 'tonk_join') {
          try {
            input.arguments.link = await resolveShortcut(input.arguments.link, linkOrigin, signal);
          } catch {
            throw new TonkToolError('Could not resolve the share link.');
          }
        }
        signal.throwIfAborted();
        const result = await run({
          binary,
          dataDirectory,
          remote,
          signal,
          input,
          marker: 'TONK_PEER_RESULT ',
        });
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

// Validate here so malformed calls never start a worker, and pass exactly the
// fields each tool takes.
function shape(tool, args) {
  const keys = Object.keys(args).sort().join(',');
  if (tool === 'tonk_list_spaces') {
    if (keys) throw new TonkToolError('tonk_list_spaces takes no arguments.');
    return {};
  }
  if (tool === 'tonk_join') {
    if (
      keys !== 'link' ||
      typeof args.link !== 'string' ||
      !args.link.trim() ||
      args.link.length > 160000
    )
      throw new TonkToolError('Provide one complete Tonk share link as link.');
    return { link: args.link.trim() };
  }
  if (
    keys !== 'document,space' ||
    typeof args.space !== 'string' ||
    !/^did:key:[A-Za-z0-9]+$/.test(args.space) ||
    typeof args.document !== 'string' ||
    !args.document.trim() ||
    Buffer.byteLength(args.document) > 32000
  )
    throw new TonkToolError(
      'Provide the space subject from tonk_list_spaces and one inline notation document, up to 32 KB.',
    );
  return { space: args.space, document: args.document };
}
