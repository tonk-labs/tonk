import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { selectTools } from './catalog.mjs';
import { TonkToolError } from './core.mjs';

// A private child owns one replica. No desktop process, shell, global CLI
// profile, credential file, or model-selected path participates in execution.
export async function startNativeRuntime({ binary, dataDirectory }) {
  const child = await startChild({ binary, dataDirectory, account: false });
  return { ...child, uiRead: (args, signal) => child.call("ui_read", args, signal) };
}

// Private service control, not an MCP backend. The OAuth service chooses this
// directory and binds browser delivery to the pending authorization ceremony.
export async function startNativeAccount({ binary, dataDirectory, enableSpaces = false }) {
  const child = await startChild({ binary, dataDirectory, account: true, enableSpaces });
  return {
    deviceDid: child.deviceDid,
    close: () => child.close(),
    status: signal => child.call('account_status', {}, signal),
    listSpaces: signal => child.call('account_list_spaces', {}, signal),
    async openSpace(subject, signal) {
      const result = await child.call('account_open_space', { subject }, signal);
      selectTools(result.capabilities);
      if (result.subject !== subject) throw new Error('Selected space identity mismatch.');
      return {
        capabilities: result.capabilities,
        subject: result.subject,
        uiRead: (args, signal) => child.call("ui_read", args, signal),
        close: () => child.close(),
        push: pushSignal => child.call('account_push_space', {}, pushSignal),
        pull: pullSignal => child.call('account_pull_space', {}, pullSignal),
        call(name, args, callSignal) {
          if (!result.capabilities.includes(name)) throw new TonkToolError('Tool unavailable.');
          return child.call(name, args, callSignal);
        },
      };
    },
    async authorize(authorization, expectedAccount, signal) {
      if (expectedAccount !== undefined && (typeof expectedAccount !== 'string' || !expectedAccount)) {
        throw new TypeError('expectedAccount must be a nonempty account DID.');
      }
      return child.call('account_authorize', {
        authorization, ...(expectedAccount !== undefined ? { expectedAccount } : {}),
      }, signal);
    },
  };
}

async function startChild({ binary, dataDirectory, account, enableSpaces = false }) {
  const child = spawn(binary, [account ? '--account-data' : '--development-data', dataDirectory,
    ...(account && enableSpaces ? ['--account-spaces'] : [])], {
    stdio: ['pipe', 'pipe', 'pipe'],
    env: { PATH: process.env.PATH, DO_NOT_TRACK: '1', TONK_NO_UPDATE_CHECK: '1' },
  });
  child.stderr.resume(); // Runtime diagnostics can contain private paths.
  let buffer = '';
  let waiter;
  let closed = false;
  let queue = Promise.resolve();
  const stop = () => {
    if (closed) return;
    closed = true;
    child.kill();
    waiter?.reject(new Error('Native runtime stopped.'));
    waiter = undefined;
  };
  child.on('error', stop);
  child.on('exit', stop);
  child.stdin.on('error', stop);
  child.stdout.setEncoding('utf8');
  child.stdout.on('data', chunk => {
    buffer += chunk;
    if (Buffer.byteLength(buffer) > 262144) { stop(); return; }
    const newline = buffer.indexOf('\n');
    if (newline < 0) return;
    const line = buffer.slice(0, newline);
    buffer = buffer.slice(newline + 1);
    if (!waiter || buffer.length) { stop(); return; }
    const pending = waiter;
    waiter = undefined;
    try { pending.resolve(JSON.parse(line)); } catch { pending.reject(new Error('Invalid runtime reply.')); stop(); }
  });
  function receive(signal) {
    return new Promise((resolve, reject) => {
      if (closed || signal?.aborted) { reject(new Error('Native runtime unavailable.')); return; }
      const timer = setTimeout(stop, 65000);
      const abort = () => stop();
      const cleanup = () => { clearTimeout(timer); signal?.removeEventListener('abort', abort); };
      waiter = {
        resolve: value => { cleanup(); resolve(value); },
        reject: error => { cleanup(); reject(error); },
      };
      signal?.addEventListener('abort', abort, { once: true });
    });
  }
  try {
    const greeting = await receive();
    selectTools(greeting.capabilities);
    if (account && (typeof greeting.deviceDid !== 'string' || !greeting.deviceDid.startsWith('did:key:') || greeting.capabilities.length)) {
      throw new Error('Invalid native account identity.');
    }
    const permitted = account ? ['account_status', 'account_authorize',
      ...(enableSpaces ? ['account_list_spaces', 'account_open_space', 'account_push_space', 'account_pull_space',
        'tonk_query', 'tonk_preview', 'tonk_apply', 'tonk_space_info', 'tonk_install_library', 'ui_read'] : [])] : [...greeting.capabilities, 'ui_read'];
    return {
      deviceDid: greeting.deviceDid,
      capabilities: greeting.capabilities,
      async close() {
        const exited = child.exitCode !== null || child.signalCode !== null
          ? Promise.resolve() : once(child, 'exit');
        stop();
        await exited;
      },
      call(name, args, signal) {
        const result = queue.then(async () => {
          if (!permitted.includes(name)) throw new TonkToolError('Tool unavailable.');
          if (closed || signal?.aborted) throw new Error('Native runtime unavailable.');
          const message = JSON.stringify({ name, arguments: args }) + '\n';
          if (Buffer.byteLength(message) > 200000) throw new TonkToolError('Runtime request exceeds the limit.');
          const pending = receive(signal);
          child.stdin.write(message);
          const response = await pending;
          if (typeof response.error === 'string') throw new TonkToolError(response.error);
          return response.result;
        });
        queue = result.catch(() => {});
        return result;
      },
    };
  } catch (error) { stop(); throw error; }
}
