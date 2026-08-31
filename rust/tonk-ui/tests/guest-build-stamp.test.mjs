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
    /fn build_relayed_request\(\s*path:\s*&str,\s*data:/,
    "the trusted host relay needs the normalized request path when deciding whether to stamp",
  );
  assert.match(
    portal,
    /headers\.delete\(tonk_worker_api::PAGE_BUILD_HEADER\)[\s\S]*is_worker_api_path\(path\)[\s\S]*headers\s*\.set\(\s*tonk_worker_api::PAGE_BUILD_HEADER/,
    "the relay must replace guest-supplied provenance only on normalized worker /api paths, never provider/control paths",
  );
});
