import { test } from "node:test";
import assert from "node:assert/strict";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  sourceFingerprint,
  SOURCE_FINGERPRINT_PREFIX,
} from "../../tonk-code/scripts/source-fingerprint.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const UI = join(HERE, "..");
const STAMP = join(UI, "scripts", "stamp-service-worker.sh");
const TONK_CODE = join(UI, "..", "tonk-code");

function fixtureDist() {
  const dist = mkdtempSync(join(tmpdir(), "tonk-build-artifacts-"));
  copyFileSync(join(UI, "index.html"), join(dist, "index.html"));
  copyFileSync(
    join(UI, "assets", "service_worker.js"),
    join(dist, "service_worker.js"),
  );
  writeFileSync(join(dist, "worker.js"), "export const worker = 1;\n");
  writeFileSync(join(dist, "worker_bg.wasm"), "worker-wasm-fixture\n");
  writeFileSync(join(dist, "kill-switch.json"), '{"revoked":[]}\n');
  writeFileSync(join(dist, "ui-a1b2c3.js"), "export const ui = 1;\n");
  mkdirSync(join(dist, "guest"));
  writeFileSync(
    join(dist, "guest", "manifest.json"),
    '{"js":"guest-a1b2c3.js","wasm":"guest_bg-a1b2c3.wasm"}\n',
  );
  writeFileSync(join(dist, "guest", "guest-a1b2c3.js"), "guest glue\n");
  writeFileSync(join(dist, "guest", "guest_bg-a1b2c3.wasm"), "guest wasm\n");
  mkdirSync(join(dist, "tonk-prose"));
  writeFileSync(
    join(dist, "tonk-prose", "tonk-prose-editor.js"),
    "stable lazy editor\n",
  );
  mkdirSync(join(dist, "guide"));
  writeFileSync(join(dist, "guide", "index.html"), "guide document\n");
  return dist;
}

function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

test("the publisher emits the complete immutable UI and guest resource graph", () => {
  const dist = fixtureDist();
  try {
    const result = spawnSync("sh", [STAMP, dist], { encoding: "utf8" });
    assert.equal(result.status, 0, result.stderr || result.stdout);

    const manifestPath = join(dist, "asset-manifest.json");
    assert.equal(
      existsSync(manifestPath),
      true,
      "the installed worker needs an exact build-produced resource graph",
    );
    const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
    const version = JSON.parse(readFileSync(join(dist, "version.json"), "utf8"));
    assert.equal(manifest.version, 1);
    assert.equal(manifest.build, version.build);
    assert.deepEqual(manifest.assets, {
      "/": sha256(join(dist, "index.html")),
      "/guest/guest-a1b2c3.js": sha256(
        join(dist, "guest", "guest-a1b2c3.js"),
      ),
      "/guest/guest_bg-a1b2c3.wasm": sha256(
        join(dist, "guest", "guest_bg-a1b2c3.wasm"),
      ),
      "/guest/manifest.json": sha256(join(dist, "guest", "manifest.json")),
      "/guide/": sha256(join(dist, "guide", "index.html")),
      "/guide/index.html": sha256(join(dist, "guide", "index.html")),
      "/tonk-prose/tonk-prose-editor.js": sha256(
        join(dist, "tonk-prose", "tonk-prose-editor.js"),
      ),
      "/ui-a1b2c3.js": sha256(join(dist, "ui-a1b2c3.js")),
    });
    const worker = readFileSync(join(dist, "service_worker.js"), "utf8");
    const stampedPaths = worker.match(/^const ASSET_PATHS = (.*);$/m);
    assert.ok(stampedPaths, "the worker must carry its stamped immutable paths");
    assert.deepEqual(
      JSON.parse(stampedPaths[1]).sort(),
      Object.keys(manifest.assets).sort(),
      "the worker routing policy must carry the exact immutable graph it installed",
    );
    assert.equal(
      manifest.assets["/service_worker.js"],
      undefined,
      "the browser owns the module service-worker script graph",
    );
    assert.equal(
      manifest.assets["/version.json"],
      undefined,
      "live deployment discovery must remain outside retained generations",
    );
    assert.equal(
      manifest.assets["/kill-switch.json"],
      undefined,
      "live withdrawal control must remain outside retained generations",
    );
  } finally {
    rmSync(dist, { recursive: true, force: true });
  }
});

test("the final Cloudflare browser tree is stamped after guide and Storybook overlays", () => {
  const flake = readFileSync(join(UI, "..", "..", "flake.nix"), "utf8");
  const packageStart = flake.indexOf("tonk-cloudflare-artifacts =");
  const packageEnd = flake.indexOf("tonk-ui-test-server =", packageStart);
  const derivation = flake.slice(packageStart, packageEnd);
  const guideCopy = derivation.indexOf("cp -r ${tonk-guide}/* ./build/tonk-ui/guide/");
  const storybookCopy = derivation.indexOf("cp -r ${tonk-storybook}/* ./build/tonk-ui/storybook/");
  const finalStamp = derivation.lastIndexOf("stamp-service-worker.sh");

  assert.ok(guideCopy >= 0 && storybookCopy >= 0, "expected both browser-tree overlays");
  assert.ok(
    finalStamp > guideCopy && finalStamp > storybookCopy,
    "BUILD_ID and asset-manifest.json must derive from the final deployed browser tree",
  );
});

test("the browser harness serves stamped directory aliases from their own index", () => {
  const flake = readFileSync(join(UI, "..", "..", "flake.nix"), "utf8");
  const packageStart = flake.indexOf("tonk-ui-test-server =");
  const server = flake.slice(packageStart);

  assert.match(
    server,
    /try_files \{path\} \{path\}\/index\.html \/index\.html/,
    "directory URLs such as /guide/ must not fall through to the root SPA document",
  );
});

test("tonk-code source identity changes when its TypeScript configuration changes", () => {
  const root = mkdtempSync(join(tmpdir(), "tonk-code-fingerprint-"));
  try {
    mkdirSync(join(root, "scripts"));
    mkdirSync(join(root, "src-js"));
    writeFileSync(join(root, "package.json"), "{}\n");
    writeFileSync(join(root, "package-lock.json"), "{}\n");
    writeFileSync(join(root, "scripts", "build.mjs"), "// build\n");
    writeFileSync(
      join(root, "scripts", "source-fingerprint.mjs"),
      "// fingerprint\n",
    );
    writeFileSync(join(root, "src-js", "index.ts"), "export {};\n");
    writeFileSync(join(root, "tsconfig.json"), '{"compilerOptions":{}}\n');
    const before = sourceFingerprint(root);
    writeFileSync(
      join(root, "tsconfig.json"),
      '{"compilerOptions":{"strict":true}}\n',
    );
    assert.notEqual(
      sourceFingerprint(root),
      before,
      "compiler semantics are part of the checked-in production bundle identity",
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("the built document and worker carry the same immutable build id", () => {
  const dist = fixtureDist();
  try {
    const result = spawnSync("sh", [STAMP, dist], { encoding: "utf8" });
    assert.equal(result.status, 0, result.stderr || result.stdout);

    const version = JSON.parse(readFileSync(join(dist, "version.json"), "utf8"));
    const worker = readFileSync(join(dist, "service_worker.js"), "utf8");
    const document = readFileSync(join(dist, "index.html"), "utf8");
    assert.match(version.build, /^[0-9a-f]{16}$/);
    assert.match(version.serviceWorker, /^[0-9a-f]{16}$/);
    assert.match(version.workerWasm, /^[0-9a-f]{16}$/);
    assert.match(version.assetManifest, /^[0-9a-f]{64}$/);
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
    assert.match(
      worker,
      new RegExp(`^const ASSET_MANIFEST_HASH = "${version.assetManifest}";$`, "m"),
    );

    const originalBuild = version.build;
    writeFileSync(join(dist, "ui-a1b2c3.js"), "export const ui = 2;\n");
    const assetChanged = spawnSync("sh", [STAMP, dist], { encoding: "utf8" });
    assert.equal(assetChanged.status, 0, assetChanged.stderr || assetChanged.stdout);
    const assetVersion = JSON.parse(readFileSync(join(dist, "version.json"), "utf8"));
    assert.notEqual(
      assetVersion.build,
      originalBuild,
      "the exact browser resource graph is part of the immutable generation",
    );

    const changedWorker = `${worker}\n// changed outer service-worker behavior\n`;
    writeFileSync(join(dist, "service_worker.js"), changedWorker);
    const changed = spawnSync("sh", [STAMP, dist], { encoding: "utf8" });
    assert.equal(changed.status, 0, changed.stderr || changed.stdout);
    const changedVersion = JSON.parse(readFileSync(join(dist, "version.json"), "utf8"));
    assert.notEqual(
      changedVersion.build,
      assetVersion.build,
      "outer service-worker behavior is part of the immutable artifact generation",
    );

    const stable = spawnSync("sh", [STAMP, dist], { encoding: "utf8" });
    assert.equal(stable.status, 0, stable.stderr || stable.stdout);
    assert.equal(
      JSON.parse(readFileSync(join(dist, "version.json"), "utf8")).build,
      changedVersion.build,
      "restamping an unchanged artifact set must be deterministic",
    );
  } finally {
    rmSync(dist, { recursive: true, force: true });
  }
});

test("a catchable publish failure restores the complete previous generation", () => {
  const dist = fixtureDist();
  try {
    const first = spawnSync("sh", [STAMP, dist], { encoding: "utf8" });
    assert.equal(first.status, 0, first.stderr || first.stdout);
    const paths = [
      "service_worker.js",
      "index.html",
      "asset-manifest.json",
      "version.json",
    ];
    const before = new Map(
      paths.map((path) => [path, readFileSync(join(dist, path), "utf8")]),
    );

    writeFileSync(join(dist, "worker.js"), "export const worker = 2;\n");

    const shimDir = join(dist, "shim-bin");
    mkdirSync(shimDir);
    const mvShim = join(shimDir, "mv");
    writeFileSync(
      mvShim,
      `#!/bin/sh
case "$2" in
  *.tmp.*)
    if [ "$3" = "$TONK_FAIL_TARGET" ] && [ ! -e "$TONK_FAIL_MARKER" ]; then
      : > "$TONK_FAIL_MARKER"
      exit 75
    fi
    ;;
esac
exec "$TONK_REAL_MV" "$@"
`,
    );
    chmodSync(mvShim, 0o755);
    const realMv = spawnSync("sh", ["-c", "command -v mv"], {
      encoding: "utf8",
    }).stdout.trim();
    assert.ok(realMv, "the test host must provide mv");

    const failed = spawnSync("sh", [STAMP, dist], {
      encoding: "utf8",
      env: {
        ...process.env,
        PATH: `${shimDir}:${process.env.PATH}`,
        TONK_FAIL_TARGET: join(dist, "version.json"),
        TONK_FAIL_MARKER: join(dist, "failed-once"),
        TONK_REAL_MV: realMv,
      },
    });
    assert.notEqual(failed.status, 0, "the injected publication must fail");
    for (const path of paths) {
      assert.equal(
        readFileSync(join(dist, path), "utf8"),
        before.get(path),
        `${path} must be restored to its complete pre-stamp bytes`,
      );
    }
    assert.equal(
      existsSync(join(dist, ".tonk-stamp.lock")),
      false,
      "a successful rollback releases the publication lock",
    );
  } finally {
    rmSync(dist, { recursive: true, force: true });
  }
});

test("the publisher fails when asset enumeration fails upstream", () => {
  const dist = fixtureDist();
  try {
    const worker = readFileSync(join(dist, "service_worker.js"), "utf8");
    const shimDir = join(dist, "find-shim");
    mkdirSync(shimDir);
    const findShim = join(shimDir, "find");
    writeFileSync(
      findShim,
      "#!/bin/sh\nprintf '%s\\n' \"$1/index.html\"\nexit 75\n",
    );
    chmodSync(findShim, 0o755);

    const result = spawnSync("sh", [STAMP, dist], {
      encoding: "utf8",
      env: { ...process.env, PATH: `${shimDir}:${process.env.PATH}` },
    });

    assert.notEqual(result.status, 0, "a truncated browser graph must not publish");
    assert.equal(readFileSync(join(dist, "service_worker.js"), "utf8"), worker);
    assert.equal(existsSync(join(dist, "asset-manifest.json")), false);
    assert.equal(existsSync(join(dist, "version.json")), false);
  } finally {
    rmSync(dist, { recursive: true, force: true });
  }
});

test("the checked-in tonk-code bundle holds reconnect while an update is pending", () => {
  const bundleUrl = pathToFileURL(join(TONK_CODE, "assets", "tonk-code.js")).href;
  const probe = `
    class FakeElement extends EventTarget {
      constructor() {
        super();
        this.attributes = new Map();
        this.isConnected = true;
      }
      getAttribute(name) { return this.attributes.get(name) ?? null; }
      setAttribute(name, value) { this.attributes.set(name, String(value)); }
      attachShadow() { return { append() {}, appendChild() {} }; }
    }
    globalThis.HTMLElement = FakeElement;
    const registry = new Map();
    globalThis.customElements = {
      get(name) { return registry.get(name); },
      define(name, constructor) { registry.set(name, constructor); },
    };
    globalThis.document = {
      baseURI: "https://tonk.test/",
      documentElement: { style: {} },
      createElement() { return { textContent: "", append() {} }; },
    };
    const serviceWorker = new EventTarget();
    serviceWorker.ready = Promise.resolve({});
    serviceWorker.controller = {};
    Object.defineProperty(globalThis, "navigator", {
      configurable: true,
      value: { serviceWorker, platform: "", userAgent: "", vendor: "" },
    });

    const streamGets = [];
    globalThis.fetch = async (url, init = {}) => {
      const headers = new Headers(init.headers);
      if (headers.get("accept") === "text/event-stream") {
        streamGets.push(String(url));
        await new Promise((resolve) => setTimeout(resolve, 10));
        return new Response('{"control":"update-pending"}', { status: 503 });
      }
      const message = init.body ? JSON.parse(init.body) : null;
      if (message?.method === "initialize") {
        return new Response(JSON.stringify({
          jsonrpc: "2.0",
          id: message.id,
          result: { capabilities: {} },
        }));
      }
      return new Response("");
    };

    await import(${JSON.stringify(`${bundleUrl}?update-pending-probe=1`)});
    const Provider = registry.get("tonk-diagnostics-provider");
    if (!Provider) throw new Error("production bundle did not register its provider");
    const provider = new Provider();
    provider.connect = () => {};
    provider.disconnect = () => {};
    provider.connectedCallback();
    const connect = new Event("tonk-code-connect");
    Object.defineProperty(connect, "detail", {
      value: { source: "tonk-buffer:///artifact-probe" },
    });
    provider.dispatchEvent(connect);

    await new Promise((resolve) => setTimeout(resolve, 50));
    const beforeControllerChange = streamGets.length;
    serviceWorker.dispatchEvent(new Event("controllerchange"));
    await new Promise((resolve) => setTimeout(resolve, 50));
    const afterControllerChange = streamGets.length;
    provider.disconnectedCallback();
    process.stdout.write(JSON.stringify({ beforeControllerChange, afterControllerChange }));
  `;
  const result = spawnSync(process.execPath, ["--input-type=module", "-e", probe], {
    encoding: "utf8",
  });
  assert.equal(result.status, 0, result.stderr || result.stdout);
  assert.deepEqual(JSON.parse(result.stdout), {
    beforeControllerChange: 1,
    afterControllerChange: 2,
  });
});

test("the checked-in tonk-code bundle was generated from the current source", () => {
  const bundle = readFileSync(join(TONK_CODE, "assets", "tonk-code.js"), "utf8");
  assert.equal(
    bundle.split("\n", 1)[0],
    `// ${SOURCE_FINGERPRINT_PREFIX}${sourceFingerprint(TONK_CODE)}`,
    "run `npm run build` in rust/tonk-code and commit the complete output graph",
  );
});

test("the publisher refuses an overlapping stamp without touching artifacts", () => {
  const dist = fixtureDist();
  try {
    const worker = readFileSync(join(dist, "service_worker.js"), "utf8");
    const document = readFileSync(join(dist, "index.html"), "utf8");
    mkdirSync(join(dist, ".tonk-stamp.lock"));

    const result = spawnSync("sh", [STAMP, dist], { encoding: "utf8" });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /another stamp is publishing/);
    assert.equal(readFileSync(join(dist, "service_worker.js"), "utf8"), worker);
    assert.equal(readFileSync(join(dist, "index.html"), "utf8"), document);
    assert.equal(existsSync(join(dist, "version.json")), false);
  } finally {
    rmSync(dist, { recursive: true, force: true });
  }
});
