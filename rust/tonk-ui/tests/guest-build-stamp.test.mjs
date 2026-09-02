import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const HERE = dirname(fileURLToPath(import.meta.url));
const RUST = join(HERE, "..", "..");
const portal = readFileSync(join(RUST, "tonk-portal", "src", "bridge.rs"), "utf8");
const guest = readFileSync(
  join(RUST, "tonk-guest", "src", "bin", "guest.rs"),
  "utf8",
);
const host = readFileSync(join(RUST, "tonk-host", "src", "bridge.rs"), "utf8");
const upload = readFileSync(join(RUST, "tonk-display", "src", "upload.rs"), "utf8");
const lsp = readFileSync(
  join(RUST, "tonk-code", "src-js", "lsp", "transport.ts"),
  "utf8",
);
const workerLsp = readFileSync(
  join(RUST, "tonk-worker", "src", "router", "lsp.rs"),
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
    /normalize_relay_path\(&raw_path\)[\s\S]*guest_relay_target\(&method,\s*&path\.pathname,\s*state\)[\s\S]*build_relayed_request\(\s*&method,\s*target\.is_worker_api\(\),\s*lsp_client\.as_deref\(\),\s*data[\s\S]*let authorized_path = target\.fetch_path\(&path\)[\s\S]*fetch_path\(&authorized_path/,
    "the trusted host relay must authorize and fetch the same normalized path before it stamps anything",
  );
  assert.match(
    portal,
    /fn build_relayed_request\(\s*method:\s*&str,\s*stamp_worker_context:\s*bool,\s*lsp_client:\s*Option<&str>,\s*data:/,
    "request construction must consume the authorization decision instead of reclassifying a raw path",
  );
  assert.match(
    portal,
    /is_trusted_relay_header\(&name\)[\s\S]*if stamp_worker_context\s*\{[\s\S]*tonk_host::bridge::context_headers\(\)[\s\S]*\.set\(name,\s*&value\)/,
    "the relay must discard guest context and stamp trusted host provenance only for authorized worker requests",
  );
});

test("sealed guest language servers are scoped before worker dispatch", () => {
  assert.match(
    portal,
    /pathname == "\/api\/language-server"[\s\S]*\.with[\s\S]*GuestRelayTarget::LanguageServer/,
    "the author-facing alias must derive its scope from trusted portal reach",
  );
  assert.match(
    portal,
    /if let Some\(client\) = lsp_client \{[\s\S]*compose_lsp_client_chain\(client, forwarded\)[\s\S]*\.set\(tonk_worker_api::LSP_CLIENT_HEADER, &client\)/,
    "the relay must strip authored direct authority and namespace a canonical descendant below its host-minted identity",
  );
  assert.match(
    portal,
    /fn is_trusted_relay_header[\s\S]*LSP_CLIENT_HEADER/,
    "guest-provided LSP client headers must enter the relay's strip set",
  );
  assert.doesNotMatch(
    workerLsp,
    /\.route\(\s*"\/api\/language-server"/,
    "the worker must not retain a global LSP endpoint",
  );
  assert.match(
    workerLsp,
    /\/api\/repository\/\{repo\}\/branch\/\{branch\}\/language-server[\s\S]*\/api\/profile\/\{profile\}\/branch\/\{branch\}\/language-server/,
    "worker routing must encode repository/profile and branch authority",
  );
  assert.match(
    workerLsp,
    /scope_inbound\(raw, &key\.scope\)[\s\S]*LspEnvProvider::new\(state, scope\)/,
    "message validation and the live environment must both receive the trusted route scope",
  );
});

test("nested relay provenance uses a capability captured before authored code", () => {
  const capture = portal.indexOf("window.fetch.bind(window)");
  const injectionListener = portal.indexOf(
    'window.addEventListener("message", async function(e)',
  );
  assert.ok(capture >= 0, "runtime bootstrap must capture the trusted relay fetch");
  assert.ok(
    injectionListener > capture,
    "the private capability must be captured synchronously before authored scripts can replace fetch",
  );
  assert.match(
    portal,
    /mod\.start\(trustedRelayFetch\)/,
    "the captured capability must be passed directly into Wasm, never published on window",
  );
  assert.match(
    guest,
    /pub fn start\(trusted_relay_fetch:\s*js_sys::Function\)[\s\S]*set_trusted_relay_fetch\(trusted_relay_fetch\)/,
    "the guest entry point must retain the captured capability before registering nested portals",
  );
  const fetchPath = portal.match(/async fn fetch_path[\s\S]*?\n}\n/)?.[0] ?? "";
  assert.match(fetchPath, /trusted_relay_fetch/);
  assert.doesNotMatch(
    fetchPath,
    /fetch_with_str_and_init/,
    "an authored window.fetch wrapper must never observe or replay descendant principal headers",
  );
});

test("an authored fetch wrapper cannot observe or replay the retained relay capability", async () => {
  const runtime = portal.match(
    /const RUNTIME_BOOTSTRAP_JS: &str = r#"([\s\S]*?)"#;/,
  )?.[1];
  assert.ok(runtime, "expected the shipped sealed-runtime bootstrap");
  const capture = runtime.match(
    /var trustedRelayFetch=window\.fetch\.bind\(window\);/,
  )?.[0];
  const handoff = runtime.match(/mod\.start\(trustedRelayFetch\);/)?.[0];
  assert.ok(capture && handoff, "expected the production capture and Wasm handoff");

  const trustedCalls = [];
  const authoredCalls = [];
  let retained;
  const window = {
    fetch(...args) {
      trustedCalls.push(args);
      return Promise.resolve(new Response("trusted"));
    },
  };
  vm.runInNewContext(`${capture}\nwindow.fetch=authoredFetch;\n${handoff}`, {
    window,
    authoredFetch(...args) {
      authoredCalls.push(args);
      return Promise.resolve(new Response("authored"));
    },
    mod: {
      start(fetch) {
        retained = fetch;
      },
    },
  });

  const init = {
    headers: { "x-tonk-lsp-client": "outer.child" },
  };
  const response = await retained("/api/language-server", init);
  assert.equal(await response.text(), "trusted");
  assert.equal(authoredCalls.length, 0);
  assert.deepEqual(trustedCalls, [["/api/language-server", init]]);
});
