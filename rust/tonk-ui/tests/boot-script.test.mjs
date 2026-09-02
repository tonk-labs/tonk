import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { describe, test } from "node:test";
import { fileURLToPath } from "node:url";
import { runInNewContext } from "node:vm";

const HERE = dirname(fileURLToPath(import.meta.url));
const INDEX = join(HERE, "..", "index.html");
const HOT_SWAP = join(HERE, "..", "assets", "hot-swap.js");

function moduleBlocks() {
  const html = readFileSync(INDEX, "utf8");
  return [...html.matchAll(/<script type="module">([\s\S]*?)<\/script>/g)]
    .map((match) => match[1]);
}

function moduleBlockContaining(needle) {
  const matches = moduleBlocks().filter((block) => block.includes(needle));
  assert.equal(matches.length, 1, `expected one module containing ${needle}`);
  return matches[0];
}

describe("boot script contract", () => {
  test("publishes immutable document provenance before the Rust loader", () => {
    const html = readFileSync(INDEX, "utf8");
    const rustLoader = html.indexOf('data-trunk\n            rel="rust"');
    const scripts = [...html.matchAll(/<script(?:\s[^>]*)?>([\s\S]*?)<\/script>/g)]
      .filter((match) => match[1].includes('meta[name="tonk-worker-build"]'));
    assert.equal(scripts.length, 1);
    assert.ok(scripts[0].index < rustLoader);

    for (const [build, expected] of [
      ["0123456789abcdef", "0123456789abcdef"],
      ["dev", undefined],
      ["AAAAAAAAAAAAAAAA", undefined],
    ]) {
      const context = { document: { querySelector: () => ({ content: build }) } };
      runInNewContext(scripts[0][1], context);
      assert.equal(context.tonkBuild, expected);
    }
  });

  test("uses one load-time update check without polling or an update prompt", () => {
    const lifecycle = moduleBlockContaining("serviceWorker.register");
    assert.equal([...lifecycle.matchAll(/registration\.update\(\)/g)].length, 1);
    assert.doesNotMatch(lifecycle, /setInterval|visibilitychange[\s\S]*registration\.update/);
    assert.doesNotMatch(lifecycle, /\/version\.json|kill-switch|Not now|announceUpdate/);
    assert.doesNotMatch(lifecycle, /type:\s*["']activate["']/);
    assert.match(lifecycle, /incoming\?\.state === "activated"[\s\S]*type: "claim"/);
  });

  test("consumes the alignment guard before considering another update", () => {
    const lifecycle = moduleBlockContaining("serviceWorker.register");
    const guard = lifecycle.indexOf("if (alignmentReload)");
    const update = lifecycle.indexOf("await registration.update()");
    assert.ok(guard >= 0 && guard < update);
    assert.match(
      lifecycle.slice(guard, update),
      /sessionStorage\.removeItem\(UPGRADE_RELOAD\)[\s\S]*return/,
    );
  });

  test("keeps deferred account safety and remote withdrawal out of production", () => {
    const html = readFileSync(INDEX, "utf8");
    assert.doesNotMatch(html, /tonk-update-safety-v1|account-setup-critical|kill-switch\.json/);
    assert.doesNotMatch(html, /Reload[\s/]+Not now|Not now/);
  });

  test("development hot swap reloads directly without a missing account gate", () => {
    const source = readFileSync(HOT_SWAP, "utf8");
    assert.equal([...source.matchAll(/window\.location\.reload\(\)/g)].length, 2);
    assert.doesNotMatch(source, /tonkReloadWhenAccountSetupDurable/);
  });

  test("activation failures retain safe actionable copy", () => {
    const lifecycle = moduleBlockContaining("serviceWorkerActivation.catch");
    assert.match(lifecycle, /Your local data is safe\./);
    assert.match(lifecycle, /Safari 16\.4\+/);
    assert.doesNotMatch(lifecycle, /Tonk could not start:\s*\$\{/);
  });
});
