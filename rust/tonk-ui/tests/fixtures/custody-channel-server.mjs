// Disposable real-service-worker fixture; no account, credentials, or Tonk data.
// node rust/tonk-ui/tests/fixtures/custody-channel-server.mjs
import {createServer} from 'node:http';
import {readFileSync} from 'node:fs';
const source = readFileSync(new URL('../../../tonk-identity/src/custody-channel.mjs', import.meta.url));
const worker = `
import {startCustodyProgress} from '/channel.mjs';
self.addEventListener('install', event => event.waitUntil(self.skipWaiting()));
self.addEventListener('activate', event => event.waitUntil(self.clients.claim()));
self.addEventListener('message', event => event.waitUntil((async () => {
  const port = event.ports[0];
  const stop = event.data.progress ? startCustodyProgress(port) : () => {};
  try {
    await new Promise(resolve => setTimeout(resolve, event.data.delay));
    port.postMessage({ok: 'completed'});
  } finally { stop(); }
})()));
`;
const html = `<!doctype html><title>Custody channel reproduction</title>
<h1>Custody channel reproduction</h1><p id="status">Starting worker</p>
<button id="legacy">Start legacy 35-second operation</button>
<button id="progress">Start fixed 35-second operation</button>
<a href="/blank" target="_blank">Open another tab</a>
<script type="module">
import {waitForCustodyReply} from '/channel.mjs';
await navigator.serviceWorker.register('/worker.mjs', {type:'module'});
await navigator.serviceWorker.ready;
if (!navigator.serviceWorker.controller) await new Promise(resolve =>
  navigator.serviceWorker.addEventListener('controllerchange', resolve, {once:true}));
const status = document.querySelector('#status');
status.textContent = 'Ready';
window.run = (progress, delay = 35000) => {
  const channel = new MessageChannel();
  const started = performance.now();
  window.result = null;
  window.visibility = [{elapsed:0, state:document.visibilityState}];
  document.addEventListener('visibilitychange', () => window.visibility.push({
    elapsed:Math.round(performance.now()-started), state:document.visibilityState
  }));
  status.textContent = 'Working';
  const result = waitForCustodyReply(channel.port1, 30000);
  navigator.serviceWorker.controller.postMessage({progress, delay}, [channel.port2]);
  result.then(reply => ({ok:reply.ok}), error => ({error:error.message})).then(outcome => {
    window.result = {...outcome, elapsed:Math.round(performance.now()-started),
      visibility:document.visibilityState};
    status.textContent = JSON.stringify(window.result);
    fetch('/result', {method:'POST', body:JSON.stringify({progress, ...window.result})});
  });
  return {started:true, progress, delay};
};
document.querySelector('#legacy').onclick = () => window.run(false);
document.querySelector('#progress').onclick = () => window.run(true);
</script>`;
const server = createServer(async (req, res) => {
  if (req.url === '/result' && req.method === 'POST') {
    let result = '';
    for await (const chunk of req) result += chunk;
    console.log(result);
    res.end('ok');
    return;
  }
  const files = {'/': ['text/html', html], '/channel.mjs': ['text/javascript', source],
    '/worker.mjs': ['text/javascript', worker], '/blank': ['text/html', '<title>Background test</title>Other tab']};
  const file = files[req.url];
  if (!file) {res.writeHead(404).end(); return;}
  res.writeHead(200, {'content-type': file[0], 'cache-control': 'no-store'}).end(file[1]);
});
server.listen(Number(process.env.TONK_LINK_FIXTURE_PORT || 0), '127.0.0.1', () => console.log('http://127.0.0.1:' + server.address().port));
