// A network the trace can change mid-run, in front of the stack.
//
//   open      pass every request through
//   latency   delay each request by `ms` before passing it through
//   offline   refuse every request: the connection closes at once
//   stall     accept every request and never answer, the worst network:
//             nothing fails, so nothing falls back
//
// Switch with GET /__gate?mode=<mode>[&ms=<n>]. The browser cannot tell
// this from the network it would otherwise see.
//
// Usage: node gate.mjs <listen-port> <upstream-origin>

import http from "node:http";
import net from "node:net";

const [listen, upstream] = [Number(process.argv[2]), new URL(process.argv[3])];
let mode = "open";
let delay = 0;
const held = new Set();

function forward(req, res) {
  const proxied = http.request(
    {
      host: upstream.hostname,
      port: upstream.port,
      method: req.method,
      path: req.url,
      headers: req.headers,
    },
    (upstreamRes) => {
      res.writeHead(upstreamRes.statusCode, upstreamRes.headers);
      upstreamRes.pipe(res);
    },
  );
  proxied.on("error", () => res.destroy());
  req.pipe(proxied);
}

const server = http.createServer((req, res) => {
  if (req.url.startsWith("/__gate")) {
    const params = new URL(req.url, "http://gate").searchParams;
    mode = params.get("mode") || "open";
    delay = Number(params.get("ms") || 0);
    if (mode !== "stall") {
      for (const r of held) r.destroy();
      held.clear();
    }
    res.end(`${mode} ${delay}\n`);
    console.log(`gate: ${mode} ${delay}`);
    return;
  }
  console.log(`${Date.now()} ${mode} ${req.method} ${req.url.slice(0, 160)}`);
  if (mode === "offline") return req.socket.destroy();
  if (mode === "stall") {
    held.add(res);
    res.on("close", () => held.delete(res));
    return;
  }
  if (mode === "latency") return setTimeout(() => forward(req, res), delay);
  forward(req, res);
});

server.on("upgrade", (req, socket, head) => {
  if (mode !== "open" && mode !== "latency") return socket.destroy();
  const target = net.connect(upstream.port, upstream.hostname, () => {
    target.write(
      `${req.method} ${req.url} HTTP/1.1\r\n` +
        Object.entries(req.headers)
          .map(([k, v]) => `${k}: ${v}`)
          .join("\r\n") +
        "\r\n\r\n",
    );
    target.write(head);
    socket.pipe(target).pipe(socket);
  });
  target.on("error", () => socket.destroy());
  socket.on("error", () => target.destroy());
});

server.listen(listen, "127.0.0.1", () => console.log(`GATE_READY ${listen}`));
