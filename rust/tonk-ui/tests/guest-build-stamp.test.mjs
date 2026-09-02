import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const RUST = join(HERE, "..", "..");
const portal = readFileSync(join(RUST, "tonk-portal", "src", "bridge.rs"), "utf8");
const host = readFileSync(join(RUST, "tonk-host", "src", "bridge.rs"), "utf8");
const upload = readFileSync(join(RUST, "tonk-display", "src", "upload.rs"), "utf8");
const lsp = readFileSync(
  join(RUST, "tonk-code", "src-js", "lsp", "transport.ts"),
  "utf8",
);

test("sealed guest worker writes inherit immutable host build provenance", () => {
  assert.match(upload, /\/blob[\s\S]*set_method\("POST"\)/);
  assert.match(lsp, /fetch\(url,[\s\S]*method:\s*"POST"/);

  assert.match(
    host,
    /context_field\("build"\)[\s\S]*JsValue::from_str\("tonkBuild"\)/,
    "nested sealed guests must inherit the immutable document build instead of probing the live deployment",
  );
  assert.match(
    portal,
    /Reflect::set\(&context,\s*&"build"\.into\(\)/,
    "the portal ready envelope must propagate build provenance through every nesting level",
  );
  assert.match(
    portal,
    /normalize_relay_path\(&raw_path\)[\s\S]*guest_relay_target\(&method,\s*&path\.pathname,\s*state\)[\s\S]*build_relayed_request\(&method,\s*target\.is_worker_api\(\),\s*data\)[\s\S]*fetch_path\(&path\.fetch_path/,
    "the trusted host relay must authorize and fetch the same normalized path before it stamps anything",
  );
  assert.match(
    portal,
    /fn build_relayed_request\(\s*method:\s*&str,\s*stamp_worker_context:\s*bool,\s*data:/,
    "request construction must consume the authorization decision instead of reclassifying a raw path",
  );
  assert.match(
    portal,
    /is_trusted_relay_header\(&name\)[\s\S]*if stamp_worker_context\s*\{[\s\S]*tonk_host::bridge::context_headers\(\)[\s\S]*\.set\(name,\s*&value\)/,
    "the relay must discard guest context and stamp trusted host provenance only for authorized worker requests",
  );
});
