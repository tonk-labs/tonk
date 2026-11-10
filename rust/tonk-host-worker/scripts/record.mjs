// node --experimental-wasm-jspi record.mjs <embedder dir> <component> <out>
//
// Boot the space's kernel `<component>` in Node, which compiles WebAssembly at
// run time, with the hosted embedder (`tonk-kernel-host` with `volatile`,
// through wasm-bindgen's `--target web` output in `<embedder dir>`), recording
// every module the polyfill makes; answer one request, so the kernel's HTTP
// path is linked too; and write each module to `<out>/<key>.wasm`.
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const [embedder, component, out] = process.argv.slice(2);
const glue = await import(pathToFileURL(join(embedder, "tonk_kernel_host.js")).href);
await glue.default({ module_or_path: readFileSync(join(embedder, "tonk_kernel_host_bg.wasm")) });

// Any space: nothing is delegated to the session, so the kernel holds none.
const space = "did:key:z6MkvRXNYcE7MMduynWTgeKbDaT1iijDSC8pZqXZc8rHPrf2";
glue.recordModules();
const kernel = await glue.HostedKernel.boot(
  readFileSync(component),
  space,
  "https",
  "space-writer.invalid",
  new Uint8Array(32).fill(7),
  undefined,
);
const answer = await kernel.request("GET", "/api/identify", [], new Uint8Array());
if (answer.status !== 200) throw new Error(`the kernel answered ${answer.status}`);
const recorded = glue.recordedModules();
mkdirSync(out, { recursive: true });
for (const [key, bytes] of Object.entries(recorded)) writeFileSync(join(out, `${key}.wasm`), bytes);
console.log(`recorded ${Object.keys(recorded).length} modules`);
