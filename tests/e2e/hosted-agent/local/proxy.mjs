// One origin for the local stack, as the provider origin is in production:
// /mcp to the Host Worker under `wrangler dev`, the access service's own
// paths to it directly, and the app to `trunk serve`. Redirects pass through
// untouched, which trunk's own proxy does not do (it follows them, so the
// `/authorize` hand-off to the approval page never reaches the browser).
//
//   node proxy.mjs <access service URL> [port] [trunk port] [host worker port]

import http from 'node:http';
import net from 'node:net';

const access = new URL(process.argv[2]);
const [port = 8081, app = 8080, host = 8790] = process.argv.slice(3).map(Number);
const SERVICE =
  /^\/(authorize|token|register|\.well-known\/(oauth-|tonk|did\.json)|@|ucan|customer|object|connection)/;

function target(url) {
  if (/^\/mcp(\/|\?|$)/.test(url)) return { host: '127.0.0.1', port: host, keepHost: true };
  if (SERVICE.test(url)) return { host: access.hostname, port: Number(access.port) };
  return { host: '127.0.0.1', port: app };
}

// The Host Worker builds its resource URLs from the Host it was asked on.
const headers = (request, to) => ({
  ...request.headers,
  host: to.keepHost ? request.headers.host : `${to.host}:${to.port}`,
});

const server = http.createServer((request, response) => {
  const to = target(request.url);
  const upstream = http.request(
    {
      host: to.host,
      port: to.port,
      path: request.url,
      method: request.method,
      headers: headers(request, to),
    },
    (answer) => {
      response.writeHead(answer.statusCode, answer.headers);
      answer.pipe(response);
    },
  );
  upstream.on('error', (error) => response.writeHead(502).end(String(error)));
  request.pipe(upstream);
});

server.on('upgrade', (request, socket, head) => {
  const to = target(request.url);
  const upstream = net.connect(to.port, to.host, () => {
    const lines = Object.entries(headers(request, to)).map(([key, value]) => `${key}: ${value}`);
    upstream.write(`${request.method} ${request.url} HTTP/1.1\r\n${lines.join('\r\n')}\r\n\r\n`);
    upstream.write(head);
    upstream.pipe(socket).pipe(upstream);
  });
  for (const end of [upstream, socket])
    end.on('error', () => (upstream.destroy(), socket.destroy()));
});

server.listen(port, '127.0.0.1');
