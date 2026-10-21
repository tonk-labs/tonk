// Profile a space worker's boot: load `url` once so the worker installs,
// have Chrome stop it, then catch the worker as it starts again, record a
// CPU profile until the page has its answer, and print where the time went.
import { spawn } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const [url, port = "9350", out = "boot.cpuprofile"] = process.argv.slice(2);
const chrome = spawn("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", [
  "--headless=new", `--remote-debugging-port=${port}`, `--user-data-dir=${mkdtempSync(join(tmpdir(), "kprof-"))}`,
  "--no-first-run", "about:blank",
], { stdio: "ignore" });
const sleep = ms => new Promise(r => setTimeout(r, ms));
let version;
for (let i = 0; i < 50; i++) {
  try { version = await (await fetch(`http://127.0.0.1:${port}/json/version`)).json(); break; } catch { await sleep(100); }
}
const ws = new WebSocket(version.webSocketDebuggerUrl);
await new Promise(r => ws.addEventListener("open", r, { once: true }));
let id = 0;
const pending = new Map();
const listeners = [];
ws.addEventListener("message", ({ data }) => {
  const message = JSON.parse(data);
  if (message.id && pending.has(message.id)) { pending.get(message.id)(message); pending.delete(message.id); }
  for (const listener of listeners) listener(message);
});
const send = (method, params = {}, sessionId) =>
  new Promise(r => { const n = ++id; pending.set(n, r); ws.send(JSON.stringify({ id: n, method, params, sessionId })); });

const { result: { targetId } } = await send("Target.createTarget", { url: "about:blank" });
const { result: { sessionId: page } } = await send("Target.attachToTarget", { targetId, flatten: true });
await send("Runtime.enable", {}, page);
const answered = async () => {
  for (let i = 0; i < 600; i++) {
    const { result } = await send("Runtime.evaluate", { expression: "document.body?.dataset.result ?? null", returnByValue: true }, page);
    if (result?.result?.value) return result.result.value;
    await sleep(50);
  }
  throw new Error("the page never answered");
};
console.error("step: navigate");
await send("Page.navigate", { url }, page);
await answered();
console.error("step: first load answered");

// Catch the worker the next time it starts, paused, and profile its boot.
let worker = null;
listeners.push(message => {
  if (message.method === "Target.attachedToTarget" && message.params.targetInfo.type === "service_worker") {
    worker = message.params.sessionId;
  }
});
await send("Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: true, flatten: true });
await send("ServiceWorker.enable", {}, page);
await send("ServiceWorker.stopAllWorkers", {}, page);
await sleep(500);
console.error("step: reload");
await send("Page.reload", {}, page);
for (let i = 0; i < 200 && !worker; i++) await sleep(25);
if (!worker) throw new Error("no service worker started");
console.error("step: worker attached");
await send("Profiler.enable", {}, worker);
await send("Profiler.setSamplingInterval", { interval: 100 }, worker);
await send("Profiler.start", {}, worker);
await send("Runtime.runIfWaitingForDebugger", {}, worker);
console.error("step: profiling");
const result = JSON.parse(await answered());
const { result: { profile } } = await send("Profiler.stop", {}, worker);
writeFileSync(out, JSON.stringify(profile));

// Self time per function, from the samples.
const byId = new Map(profile.nodes.map(node => [node.id, node]));
const self = new Map();
for (let i = 0; i < profile.samples.length; i++) {
  const node = byId.get(profile.samples[i]);
  const name = node.callFrame.functionName || `(${node.callFrame.url.split("/").pop() || "anonymous"})`;
  self.set(name, (self.get(name) ?? 0) + (profile.timeDeltas[i] ?? 0) / 1000);
}
const total = [...self.values()].reduce((a, b) => a + b, 0);
console.log(`boot stages: ${JSON.stringify(Object.fromEntries(Object.entries(result.boot?.stages ?? {}).map(([k, v]) => [k, Math.round(v)])))}`);
console.log(`profiled ${Math.round(total)} ms; top self time:`);
for (const [name, ms] of [...self].sort((a, b) => b[1] - a[1]).slice(0, 30)) {
  console.log(`${ms.toFixed(1).padStart(8)} ms  ${name.slice(0, 140)}`);
}
ws.close(); chrome.kill(); process.exit(0);
