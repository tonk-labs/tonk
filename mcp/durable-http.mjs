// The response is released only after native handles are closed and the state
// checkpoint is durable. Failed persistence poisons this process: no retries
// against an uncertain generation, including requests already in the queue.
export function createDurableHTTP({ handle, quiesce, checkpoint, capacity = 16, onFailure = () => {} }) {
  let tail = Promise.resolve(), queued = 0, failed = false;
  return {
    async fetch(request) {
      if (failed) return unavailable();
      if (queued >= capacity) return new Response(null, { status: 429 });
      queued++;
      const result = tail.then(async () => {
        if (failed) return unavailable();
        let response;
        try { response = await handle(request); }
        catch { response = unavailable(); }
        let stage = 'quiesce';
        try { await quiesce(); stage = 'checkpoint'; await checkpoint.save(); }
        catch (error) {
          failed = true;
          // Never log arbitrary errors: they may contain credentials or paths.
          const reason = error?.message === 'Checkpoint exceeds limit.' ? 'capacity' : 'unavailable';
          try { onFailure({ stage, reason }); } catch {}
          return unavailable();
        }
        return response;
      });
      tail = result.catch(() => { failed = true; });
      try { return await result; } finally { queued--; }
    },
    async close() { failed = true; await tail; await quiesce(); },
  };
}
function unavailable() {
  return Response.json({ error: 'Runtime unavailable. Reconnect and inspect the space before retrying any write.' },
    { status: 503, headers: { 'cache-control': 'no-store' } });
}
