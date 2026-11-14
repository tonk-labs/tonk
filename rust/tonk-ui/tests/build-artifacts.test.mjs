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
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
const HERE = dirname(fileURLToPath(import.meta.url));
const UI = join(HERE, "..");
const STAMP = join(UI, "scripts", "stamp-service-worker.sh");

function fixtureDist() {
  const dist = mkdtempSync(join(tmpdir(), "tonk-build-artifacts-"));
  copyFileSync(join(UI, "index.html"), join(dist, "index.html"));
  copyFileSync(
    join(UI, "assets", "service_worker.js"),
    join(dist, "service_worker.js"),
  );
  writeFileSync(join(dist, "ui-a1b2c3.js"), "export const ui = 1;\n");
  writeFileSync(join(dist, "ui-a1b2c3.js.map"), "source map fixture");
  mkdirSync(join(dist, "agent"));
  copyFileSync(join(UI, "assets", "agent", "index.html"), join(dist, "agent", "index.html"));
  mkdirSync(join(dist, "guest"));
  writeFileSync(
    join(dist, "guest", "manifest.json"),
    '{"js":"guest-a1b2c3.js","wasm":"guest_bg-a1b2c3.wasm"}\n',
  );
  writeFileSync(join(dist, "guest", "guest-a1b2c3.js"), "guest glue\n");
  writeFileSync(join(dist, "guest", "guest_bg-a1b2c3.wasm"), "guest wasm\n");
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
      "/agent/": sha256(join(dist, "agent", "index.html")),
      "/agent/index.html": sha256(join(dist, "agent", "index.html")),
      "/guest/guest-a1b2c3.js": sha256(
        join(dist, "guest", "guest-a1b2c3.js"),
      ),
      "/guest/guest_bg-a1b2c3.wasm": sha256(
        join(dist, "guest", "guest_bg-a1b2c3.wasm"),
      ),
      "/guest/manifest.json": sha256(join(dist, "guest", "manifest.json")),
      "/ui-a1b2c3.js": sha256(join(dist, "ui-a1b2c3.js")),
    });
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
  } finally {
    rmSync(dist, { recursive: true, force: true });
  }
});

test("the page build tracks top-level document resources and ignores guest code", () => {
  const stamp = (mutate) => {
    const dist = fixtureDist();
    try {
      // What a site's own origin runs: its worker, the kernel it hosts,
      // and its shell.
      for (const name of ["kernel_boot.js", "site.html"]) {
        copyFileSync(join(UI, "assets", name), join(dist, name));
      }
      mkdirSync(join(dist, "_host", "kernel"), { recursive: true });
      writeFileSync(join(dist, "_host", "kernel", "bafkr4ikernel.wasm"), "kernel A\n");
      mutate(dist);
      const result = spawnSync("sh", [STAMP, dist], { encoding: "utf8" });
      assert.equal(result.status, 0, result.stderr || result.stdout);
      const version = JSON.parse(readFileSync(join(dist, "version.json"), "utf8"));
      const document = readFileSync(join(dist, "index.html"), "utf8");
      assert.match(version.page, /^[0-9a-f]{16}$/);
      assert.match(
        document,
        new RegExp(`<meta name="tonk-page-build" content="${version.page}" />`),
        "the document names the page build it was emitted as",
      );
      return version;
    } finally {
      rmSync(dist, { recursive: true, force: true });
    }
  };
  const base = stamp(() => {});
  const guest = stamp((dist) => {
    writeFileSync(join(dist, "guest", "guest_bg-a1b2c3.wasm"), "guest wasm B\n");
  });
  const kernel = stamp((dist) => {
    writeFileSync(join(dist, "_host", "kernel", "bafkr4ikernel.wasm"), "kernel B\n");
  });
  const page = stamp((dist) => {
    writeFileSync(join(dist, "ui-a1b2c3.js"), "export const ui = 2;\n");
  });

  assert.notEqual(guest.build, base.build);
  assert.equal(guest.page, base.page, "guest code does not change the page");
  assert.notEqual(kernel.build, base.build);
  assert.equal(kernel.page, base.page, "the kernel a site runs does not change the page");
  const shell = stamp((dist) => {
    writeFileSync(join(dist, "site.html"), "<!doctype html><title>another shell</title>\n");
  });
  assert.notEqual(shell.build, base.build);
  assert.equal(shell.page, base.page, "what a site's origin runs does not change the page");
  assert.notEqual(page.page, base.page, "top-level code changes the page");
});

test("trunk's unordered module preloads do not change the build identity", () => {
  const preloads = (order) =>
    order
      .map((name) => `<link rel="modulepreload" href="/snippets/${name}.js" crossorigin="anonymous">`)
      .join("");
  const stamp = (order) => {
    const dist = fixtureDist();
    try {
      const index = readFileSync(join(dist, "index.html"), "utf8");
      writeFileSync(
        join(dist, "index.html"),
        index.replace("</head>", `    ${preloads(order)}</head>`),
      );
      const result = spawnSync("sh", [STAMP, dist], { encoding: "utf8" });
      assert.equal(result.status, 0, result.stderr || result.stdout);
      return JSON.parse(readFileSync(join(dist, "version.json"), "utf8"));
    } finally {
      rmSync(dist, { recursive: true, force: true });
    }
  };
  const first = stamp(["a", "b", "c"]);
  const shuffled = stamp(["c", "a", "b"]);
  const changed = stamp(["a", "b", "d"]);
  assert.equal(shuffled.page, first.page, "reordered preloads keep the page build");
  assert.equal(shuffled.build, first.build, "reordered preloads keep the build");
  assert.notEqual(changed.page, first.page, "a different preload set changes the page");
});

test("the Cloudflare browser tree excludes local documentation tools", () => {
  const flake = readFileSync(join(UI, "..", "..", "flake.nix"), "utf8");
  const packageStart = flake.indexOf("tonk-cloudflare-artifacts =");
  const packageEnd = flake.indexOf("tonk-ui-test-server =", packageStart);
  const derivation = flake.slice(packageStart, packageEnd);
  assert.doesNotMatch(derivation, /tonk-guide|\.\/build\/tonk-ui\/guide/);
  assert.doesNotMatch(derivation, /tonk-storybook|\.\/build\/tonk-ui\/storybook/);
  assert.doesNotMatch(
    derivation,
    /stamp-service-worker\.sh/,
    "the already-stamped tonk-ui output must not be expanded and restamped",
  );
});

test("the app's worker carries the build id the version names", () => {
  const dist = fixtureDist();
  try {
    const result = spawnSync("sh", [STAMP, dist], { encoding: "utf8" });
    assert.equal(result.status, 0, result.stderr || result.stdout);

    const version = JSON.parse(readFileSync(join(dist, "version.json"), "utf8"));
    const worker = readFileSync(join(dist, "service_worker.js"), "utf8");
    assert.match(version.build, /^[0-9a-f]{16}$/);
    assert.match(version.serviceWorker, /^[0-9a-f]{16}$/);
    assert.match(version.assetManifest, /^[0-9a-f]{64}$/);
    assert.match(
      worker,
      new RegExp(`^const BUILD_ID = "${version.build}";$`, "m"),
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

    writeFileSync(join(dist, "ui-a1b2c3.js"), "export const ui = 2;\n");

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
