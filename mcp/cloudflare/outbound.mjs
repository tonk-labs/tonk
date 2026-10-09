import { checkpointLimit as limit } from '../checkpoint-limits.mjs';
// This handler is reached only by the container proxy, never by public routing.
export async function outbound(request, env, context, forward = fetch) {
  const url = new URL(request.url);
  if (url.origin === 'https://tonk.foundation') {
    // Do not follow a redirect in the Worker: each next destination must pass
    // through the container's deny-by-default network policy again.
    return forward(new Request(request, { redirect: 'manual' }));
  }
  if (url.origin !== 'http://checkpoint.internal' || !['/state','/credentials'].includes(url.pathname) || url.search) {
    return new Response(null, { status: 403 });
  }
  const key = `checkpoint/${context.containerId}/${url.pathname==='/state'?'state':'credentials'}-v1.json`;
  if (request.method === 'GET') {
    const object = await env.CHECKPOINTS.get(key);
    return object ? new Response(object.body, { headers: { etag: object.httpEtag, 'cache-control': 'no-store' } }) :
      new Response(null, { status: 404 });
  }
  if (request.method !== 'PUT') return new Response(null, { status: 405 });
  const match = request.headers.get('if-match'), absent = request.headers.get('if-none-match');
  if ((!match && absent !== '*') || (match && absent)) return new Response(null, { status: 428 });
  let size = 0;
  const bounded = request.body?.pipeThrough(new TransformStream({
    transform(chunk, controller) {
      size += chunk.byteLength;
      if (size > (url.pathname==='/credentials'?2*1024*1024:limit)) throw new Error('Checkpoint exceeds limit.');
      controller.enqueue(chunk);
    },
  }));
  if (!bounded) return new Response(null, { status: 400 });
  const bytes = await new Response(bounded).arrayBuffer();
  const object = await env.CHECKPOINTS.put(key, bytes, { onlyIf: request.headers });
  return object ? new Response(null, { status: 200, headers: { etag: object.httpEtag } }) :
    new Response(null, { status: 412 });
}
