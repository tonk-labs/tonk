import { TonkToolError } from './errors.mjs';

// Portable across Node, browsers and the desktop WebKit host. The host supplies
// an authorized request function; tool arguments never choose an origin or branch.
export const primitiveTools = ['tonk_query', 'tonk_evaluate'];
const encoder = new TextEncoder();
const uncertain = 'Evaluation could not be confirmed. Do not repeat it; query the space first.';

async function readWorkerJSON(response) {
  if (!response.body) throw Error('Missing worker response.');
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  const parts = [];
  let bytes = 0;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      bytes += value.byteLength;
      if (bytes > 2_000_000) throw Error('Worker response exceeds the tool limit.');
      parts.push(decoder.decode(value, { stream: true }));
    }
    return JSON.parse(parts.join('') + decoder.decode());
  } finally {
    // Stop oversized/error streams without masking the original result or error.
    await reader.cancel().catch(() => {});
    reader.releaseLock();
  }
}

export function workerPrimitive({ subject, request, allowsWrites = true }) {
  if (!/^did:key:[A-Za-z0-9]+$/.test(subject)) throw new TonkToolError('Invalid selected space.');
  const base = '/api/repository/' + encodeURIComponent(subject) + '/branch/main';
  return {
    subject,
    capabilities: allowsWrites ? [...primitiveTools] : ['tonk_query'],
    async call(name, args, signal) {
      if (!primitiveTools.includes(name)) throw new TonkToolError('Tool unavailable.');
      const writing = name === 'tonk_evaluate';
      if (writing && !allowsWrites)
        throw new TonkToolError('This runtime connection is read-only.');
      if (
        !args ||
        Array.isArray(args) ||
        Object.keys(args).length !== 1 ||
        typeof args.document !== 'string' ||
        !args.document.trim() ||
        encoder.encode(args.document).length > 32000
      )
        throw new TonkToolError('Provide one inline notation document, up to 32 KB.');
      // Cancellation before dispatch is known not to have written. Once dispatched,
      // a lost reply is uncertain. Never retry here or substitute another operation.
      signal?.throwIfAborted();
      try {
        const response = await request(base + '/evaluate?transact=' + writing, {
          method: 'POST',
          headers: { 'content-type': 'text/plain' },
          body: args.document,
          signal,
        });
        if (!response.ok && response.status === 400) {
          const error = (await readWorkerJSON(response).catch(() => null))?.error;
          if (error?.kind === 'analyze' && typeof error.message === 'string')
            throw new TonkToolError('Notation rejected: ' + error.message.slice(0, 2000));
        }
        if (!response.ok) {
          await response.body?.cancel().catch(() => {});
          throw new TonkToolError(writing ? uncertain : 'The worker rejected this query.');
        }
        if (!(response.headers.get('content-type') || '').includes('json')) {
          await response.body?.cancel().catch(() => {});
          throw Error('Invalid worker response.');
        }
        const value = await readWorkerJSON(response);
        if (!value || typeof value !== 'object' || Array.isArray(value))
          throw Error('Invalid worker response.');
        // Preserve the worker envelope, including revisions, matches and effects.
        return value;
      } catch (error) {
        if (error instanceof TonkToolError) throw error;
        throw new TonkToolError(writing ? uncertain : 'The worker query could not be completed.');
      }
    },
  };
}
