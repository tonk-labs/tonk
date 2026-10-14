// The deadline detects a lost worker, not a slow account operation. Progress
// carries no authority or account data; only a terminal reply can complete it.
export function waitForCustodyReply(port, timeoutMs) {
  return new Promise((resolve, reject) => {
    let timer;
    const finish = (error, reply) => {
      clearTimeout(timer);
      port.onmessage = null;
      port.onmessageerror = null;
      port.close();
      if (error) reject(error); else resolve(reply);
    };
    const arm = () => {
      clearTimeout(timer);
      timer = setTimeout(() => finish(new Error(
        'the service worker did not answer the custody handoff in time'
      )), timeoutMs);
    };
    port.onmessage = ({data}) => {
      if (data?.pending === true) { arm(); return; }
      if (typeof data?.error === 'string') {
        const error = new Error(data.error);
        if (typeof data.code === 'string') error.code = data.code;
        finish(error);
      } else {
        finish(null, data);
      }
    };
    port.onmessageerror = () => finish(new Error('the custody reply could not be read'));
    arm();
  });
}

// Owned by the worker's in-flight message handler, which is already covered by
// event.waitUntil. Stopping the handler stops these messages; no detached task
// or idle/background sync is needed to keep a user-initiated operation alive.
export function startCustodyProgress(port, intervalMs = 10_000) {
  const progress = () => port.postMessage({pending: true});
  progress();
  const timer = setInterval(progress, intervalMs);
  return () => clearInterval(timer);
}
