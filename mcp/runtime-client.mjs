import { lstat, readFile } from 'node:fs/promises';

// A private host connection is a capability. Never include it in tool results.
export async function connectRuntime(path) {
  const info = await lstat(path);
  if (!info.isFile() || (info.mode & 0o077) !== 0 ||
      (process.getuid && info.uid !== process.getuid())) {
    throw new Error('Runtime connection must be a private file owned by this user.');
  }
  const config = JSON.parse(await readFile(path, 'utf8'));
  const url = new URL(config.url);
  if (config.version !== 1 || url.protocol !== 'http:' || url.hostname !== '127.0.0.1' ||
      !url.port || url.username || url.password || url.search || url.hash || url.pathname !== '/' ||
      typeof config.token !== 'string' || !/^[A-Za-z0-9-]{64,128}$/.test(config.token)) {
    throw new Error('Invalid local runtime connection.');
  }
  return async (path, body, signal) => {
    if (!['/tools', '/call'].includes(path)) throw new Error('Unknown runtime operation.');
    const response = await fetch(new URL(path, url), {
      method: 'POST', redirect: 'error',
      headers: { 'content-type': 'application/json', authorization: `Bearer ${config.token}` },
      body: JSON.stringify(body),
      signal: signal ? AbortSignal.any([signal, AbortSignal.timeout(65000)]) : AbortSignal.timeout(65000),
    });
    if (!response.ok) throw new Error(`Runtime connection failed (HTTP ${response.status}). Reconnect to Tonk.`);
    let size = 0;
    const chunks = [];
    for await (const chunk of response.body) {
      size += chunk.length;
      if (size > 262144) throw new Error('Runtime response exceeds the limit.');
      chunks.push(chunk);
    }
    return JSON.parse(Buffer.concat(chunks).toString('utf8'));
  };
}
