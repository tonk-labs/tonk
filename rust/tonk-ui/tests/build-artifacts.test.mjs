import { test } from "node:test";
import assert from "node:assert/strict";
import {
  copyFileSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const HERE = dirname(fileURLToPath(import.meta.url));
const UI = join(HERE, "..");
const STAMP = join(UI, "scripts", "stamp-service-worker.sh");

test("the built document and worker carry the same immutable build id", () => {
  const dist = mkdtempSync(join(tmpdir(), "tonk-build-artifacts-"));
  try {
    copyFileSync(join(UI, "index.html"), join(dist, "index.html"));
    copyFileSync(
      join(UI, "assets", "service_worker.js"),
      join(dist, "service_worker.js"),
    );
    writeFileSync(join(dist, "worker.js"), "export const worker = 1;\n");
    writeFileSync(join(dist, "worker_bg.wasm"), "worker-wasm-fixture\n");

    const result = spawnSync("sh", [STAMP, dist], { encoding: "utf8" });
    assert.equal(result.status, 0, result.stderr || result.stdout);

    const version = JSON.parse(readFileSync(join(dist, "version.json"), "utf8"));
    const worker = readFileSync(join(dist, "service_worker.js"), "utf8");
    const document = readFileSync(join(dist, "index.html"), "utf8");
    assert.match(version.build, /^[0-9a-f]{16}$/);
    assert.match(
      worker,
      new RegExp(`^const BUILD_ID = "${version.build}";$`, "m"),
    );
    assert.match(
      document,
      new RegExp(
        `<meta\\s+name="tonk-worker-build"\\s+content="${version.build}"\\s*/?>`,
      ),
      "index.html must embed the worker build it was emitted alongside; a live version probe is not document provenance",
    );
  } finally {
    rmSync(dist, { recursive: true, force: true });
  }
});
