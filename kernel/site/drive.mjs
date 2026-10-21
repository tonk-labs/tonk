// Open `url` in headless Chrome, twice (cold, then a reload), and print what
// the page left in `document.body.dataset.result` each time.
import { spawn } from "node:child_process";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const [url, port = "9333"] = process.argv.slice(2);
const chrome = spawn("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", [
  "--headless=new", `--remote-debugging-port=${port}`, `--user-data-dir=${mkdtempSync(join(tmpdir(), "kspike-"))}`,
  "--no-first-run", "about:blank",
], { stdio: "ignore" });
const sleep = ms => new Promise(r => setTimeout(r, ms));
let targets;
for (let i = 0; i < 50; i++) {
  try { targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json(); break; } catch { await sleep(100); }
}
const page = targets.find(t => t.type === "page");
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise(r => ws.addEventListener("open", r, { once: true }));
let id = 0;
const pending = new Map();
ws.addEventListener("message", ({ data }) => {
  const message = JSON.parse(data);
  if (message.id && pending.has(message.id)) { pending.get(message.id)(message); pending.delete(message.id); }
  if (message.method === "Runtime.consoleAPICalled") console.error("console:", message.params.args.map(a => a.value).join(" "));
});
const send = (method, params = {}) => new Promise(r => { const n = ++id; pending.set(n, r); ws.send(JSON.stringify({ id: n, method, params })); });
await send("Runtime.enable");
async function run(label, action) {
  await action();
  for (let i = 0; i < 600; i++) {
    const { result } = await send("Runtime.evaluate", { expression: "document.body?.dataset.result ?? null", returnByValue: true });
    if (result?.result?.value) { console.log(label, result.result.value); return; }
    await sleep(100);
  }
  console.log(label, "timed out");
}
await run("cold", () => send("Page.navigate", { url }));
await run("reload", () => send("Page.reload"));
await send("ServiceWorker.enable");
for (const round of [1, 2]) {
  await send("ServiceWorker.stopAllWorkers");
  await sleep(500);
  await run(`restart-${round}`, () => send("Page.reload"));
}
ws.close(); chrome.kill();
process.exit(0);
