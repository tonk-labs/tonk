import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
const types = { html: "text/html", js: "text/javascript", wasm: "application/wasm" };
const root = new URL(".", import.meta.url);
createServer(async (req, res) => {
  const path = new URL(req.url, "http://x").pathname;
  const file = path === "/" ? "index.html" : path.slice(1);
  try {
    const body = await readFile(new URL(file, root));
    res.writeHead(200, { "content-type": types[file.split(".").pop()] ?? "application/octet-stream", "cache-control": "no-cache" });
    res.end(body);
  } catch {
    res.writeHead(404); res.end("not found");
  }
}).listen(Number(process.argv[2] ?? 8191), () => console.log("listening"));
