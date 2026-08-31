import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const RUST = join(HERE, "..", "..");
const http = readFileSync(join(RUST, "tonk-host", "src", "http.rs"), "utf8");
const host = readFileSync(join(RUST, "tonk-host", "src", "host.rs"), "utf8");
const portal = readFileSync(join(RUST, "tonk-portal", "src", "bridge.rs"), "utf8");

test("every host transport observes the typed stale marker before body handling", () => {
  assert.match(
    http,
    /fn worker_response\([\s\S]*resp\.status\(\)\s*==\s*409[\s\S]*tonk_worker_api::ERROR_KIND_HEADER[\s\S]*tonk_worker_api::STALE_BUILD_ERROR_KIND[\s\S]*announce_update\(\)/,
    "one response adapter must own exact header-based stale signaling",
  );
  assert.match(
    http,
    /async fn response_text\([\s\S]*worker_response\(resp_value\)\?[\s\S]*resp\.text\(\)/,
    "ordinary JSON responses must signal before consuming their typed body",
  );
  for (const name of ["post_site_to", "frame_stream", "post_text"]) {
    const start = http.indexOf(`fn ${name}(`);
    assert.notEqual(start, -1, `expected ${name}`);
    const next = http.indexOf("\n}", start);
    assert.match(
      http.slice(start, next + 2),
      /worker_response\(resp_value\)/,
      `${name} must use the same response adapter`,
    );
  }
  assert.match(
    host,
    /fetch_with_str_and_init\("\/api\/sync\?why=keepalive"[\s\S]*worker_response\(resp_value\)/,
    "keepalive must observe a marked refusal instead of discarding the response",
  );
});

test("nested marked responses relay the update signal to trusted top chrome", () => {
  assert.match(
    http,
    /fn announce_update\([\s\S]*updateAvailable[\s\S]*call0[\s\S]*tonk-update-available/,
    "a sealed host must relay rather than dispatching an iframe-local event",
  );
  assert.match(
    portal,
    /updateAvailable:function\([\s\S]*type:"update-available"/,
    "the guest bridge needs one non-privileged page-effect for stale signaling",
  );
  assert.match(
    portal,
    /"update-available"\s*=>\s*tonk_host::announce_update\(\)/,
    "each portal parent must forward the signal until trusted top chrome receives it",
  );
});
