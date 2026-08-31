// Structural checks on the boot script in `index.html`.
//
// The boot script is where service-worker registration, update
// discovery, and the revocation gate live. It is deliberately outside
// the app wasm, because the app wasm is one of the things that can be
// the stale or broken half — which also means nothing else type-checks
// or bundles it. `node --check` only proves each block PARSES.
//
// The bug these guard: `isRevoked` was defined in a different
// `<script type="module">` block than the code calling it. Separate
// modules do not share scope, so the call threw `ReferenceError` before
// registration was ever reached — no service worker at all, and every
// `/api/*` request falling through to the static server as a 405. Every
// block parsed fine, so syntax checking saw nothing.
import { test, describe, before } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const HERE = dirname(fileURLToPath(import.meta.url));
const INDEX = join(HERE, "..", "index.html");

/** The `<script type="module">` bodies, in document order. */
function moduleBlocks() {
  const html = readFileSync(INDEX, "utf8");
  return [...html.matchAll(/<script type="module">([\s\S]*?)<\/script>/g)].map(
    (m) => m[1],
  );
}

/** The one module block that owns `needle`. */
function moduleBlockContaining(needle) {
  const matches = moduleBlocks().filter((block) => block.includes(needle));
  assert.equal(
    matches.length,
    1,
    `expected exactly one module block containing ${needle}, found ${matches.length}`,
  );
  return matches[0];
}

/**
 * Bare identifiers a block CALLS as a plain function — `foo(...)`, not
 * `obj.foo(...)` — that it does not itself declare.
 *
 * Method calls and words inside comments are excluded, because a regex
 * that counts those reports mostly noise. This is not general scope
 * analysis; it catches the one shape that actually broke: a helper
 * defined in one module block and called from another.
 */
function undefinedLocalCalls(block) {
  // Strip comments and string literals first — the bug is about code.
  const code = block
    .replace(/\/\*[\s\S]*?\*\//g, " ")
    .replace(/\/\/[^\n]*/g, " ")
    .replace(/"(?:[^"\\]|\\.)*"/g, '""')
    .replace(/'(?:[^'\\]|\\.)*'/g, "''")
    .replace(/`(?:[^`\\]|\\.)*`/g, "``");

  const RESERVED = new Set([
    "if", "for", "while", "switch", "catch", "return", "typeof", "await",
    "function", "new", "delete", "void", "in", "of", "do", "else", "throw",
    // `async (…) =>` reads as a call to `async` under this regex.
    "async",
  ]);

  const called = new Set();
  // A call NOT preceded by `.` or `?.` — i.e. a bare function call.
  for (const m of code.matchAll(/(^|[^.\w$?])([A-Za-z_$][\w$]*)\s*\(/g)) {
    const name = m[2];
    if (!RESERVED.has(name)) called.add(name);
  }

  return [...called].filter((name) => {
    const declared = new RegExp(
      "(?:function\\s+" + name + "\\b" +
        "|(?:const|let|var)\\s+" + name + "\\b" +
        "|class\\s+" + name + "\\b" +
        "|" + name + "\\s*(?::|=)\\s*(?:async\\s*)?(?:function|\\()" +
        ")",
    );
    return !declared.test(code);
  });
}

describe("boot script module scoping", () => {
  // Anything the page itself defines and calls must live in ONE block.
  // Globals and Web APIs are fine across blocks; locals are not.
  const KNOWN_GLOBALS = new Set([
    "fetch", "setTimeout", "setInterval", "clearTimeout", "clearInterval",
    "queueMicrotask", "requestAnimationFrame", "addEventListener",
    "removeEventListener", "dispatchEvent", "Promise", "Error", "TypeError",
    "String", "Number", "Boolean", "Array", "Object", "JSON", "Date", "Math",
    "Map", "Set", "URL", "URLSearchParams", "CustomEvent", "Event", "Response",
    "Request", "Headers", "Blob", "FormData", "TextDecoder", "TextEncoder",
    "AbortController", "IntersectionObserver", "MutationObserver", "matchMedia",
    "getComputedStyle", "structuredClone", "reject", "resolve", "if", "for",
    "while", "switch", "catch", "return", "typeof", "await", "import",
    "requestIdleCallback", "reportError", "btoa", "atob", "parseInt",
    "parseFloat", "isNaN", "encodeURIComponent", "decodeURIComponent",
  ]);

  test("every function the boot script calls is defined in its own block", () => {
    for (const [index, block] of moduleBlocks().entries()) {
      const missing = undefinedLocalCalls(block).filter(
        (name) => !KNOWN_GLOBALS.has(name),
      );
      assert.deepEqual(
        missing,
        [],
        `module block ${index} calls ${missing.join(", ")} but does not ` +
          `define it. Separate <script type="module"> blocks do not share ` +
          `scope, so this throws ReferenceError at runtime even though the ` +
          `block parses.`,
      );
    }
  });

  test("registration and its revocation gate live together", () => {
    // The specific pairing that broke: the gate runs BEFORE registration
    // and decides whether it happens at all, so a scope split here means
    // no service worker is registered on any page load.
    const registering = moduleBlockContaining("serviceWorker.register");
    assert.ok(
      registering.includes("await isRevoked()"),
      "the revocation gate should guard registration",
    );
    assert.ok(
      /(?:function\s+isRevoked|const\s+isRevoked\s*=\s*async)/.test(
        registering,
      ),
      "isRevoked must be defined in the same block that calls it",
    );
  });
});

describe("boot script contract with the worker", () => {
  test("never creates window.tonk on the top page", () => {
    // `window.tonk` is the SEALED GUEST's bridge object, and
    // `page_effect::forward` tests for its presence to decide "am I
    // running inside a guest?". Creating it on the top page made the
    // host believe it was a guest: every `navigate_to` forwarded to a
    // `navigate` method that does not exist there and returned early,
    // so links and menu buttons silently did nothing while the rest of
    // the app looked fine.
    //
    // The boot script must publish its build under its own name.
    for (const [index, block] of moduleBlocks().entries()) {
      const code = block
        .replace(/\/\*[\s\S]*?\*\//g, " ")
        .replace(/\/\/[^\n]*/g, " ");
      assert.ok(
        !/(?:globalThis|window|self)\s*\.\s*tonk\s*(?:=|\.|\[)/.test(code),
        `module block ${index} assigns to window.tonk. That name belongs ` +
          `to the guest bridge; writing it on the top page makes the host ` +
          `look like a guest and silently breaks all navigation.`,
      );
    }
  });

  test("publishes the build id the version handshake sends", () => {
    // The host reads `window.tonk.build` and sends it on every /api/*
    // request; the worker compares it against its own. If the boot
    // script stops publishing it, the handshake silently stops working
    // rather than failing loudly.
    const updateDiscovery = moduleBlockContaining("globalThis.tonkBuild");
    assert.match(
      updateDiscovery,
      /globalThis\.tonkBuild\s*=/,
      "the build id is published under its own name, not on window.tonk",
    );
  });

  test("asks for updates on the triggers an SPA actually gets", () => {
    // The browser only checks on navigation and at register time, which
    // a pushState app with long-lived tabs can go days without.
    const updateDiscovery = moduleBlockContaining("const checkForUpdate");
    assert.match(updateDiscovery, /visibilitychange/);
    assert.match(updateDiscovery, /"online"/);
    assert.match(updateDiscovery, /registration\.update\(\)/);
  });

  test("decides staleness by comparing builds, not by listening for events", () => {
    // `registration` is shared across every tab on the origin, so
    // `updatefound` fires in all of them — including tabs already
    // running the new build, and including the install a sibling tab's
    // reload just triggered. Announcing on that event made two tabs
    // ping-pong: reload A, B prompts; reload B, A prompts; forever.
    //
    // Gating on `registration.waiting` did not fix it either: the
    // worker calls `skipWaiting()`, so a successor goes
    // `installing -> activating` and barely touches that slot.
    //
    // The honest question is a state each tab answers for itself: is
    // the served build different from the one this page loaded?
    const updateDiscovery = moduleBlockContaining("const checkStale");
    const code = updateDiscovery
      .replace(/\/\*[\s\S]*?\*\//g, " ")
      .replace(/\/\/[^\n]*/g, " ");

    assert.match(
      code,
      /build\s*!==\s*ourBuild/,
      "staleness must be a build comparison against this page's own build",
    );
    assert.ok(
      !/registration\.addEventListener\(\s*["']updatefound["']/.test(code),
      "announcing from the `updatefound` event makes sibling tabs prompt " +
        "each other in a loop",
    );
  });

  test("reads the version and kill-switch probes uncached", () => {
    // Both exist to answer correctly when the worker's own update
    // machinery is wedged; a cached answer defeats that entirely.
    const registering = moduleBlocks().find((b) =>
      b.includes("serviceWorker.register"),
    );
    for (const probe of ["/version.json", "/kill-switch.json"]) {
      const call = new RegExp(
        `fetch\\(\\s*"${probe.replace("/", "\\/")}"[^)]*cache:\\s*"no-store"`,
      );
      assert.match(
        registering,
        call,
        `${probe} must be fetched with cache: "no-store"`,
      );
    }
  });

  test("a withdrawn build terminalizes and unregisters only this page's worker", () => {
    const registering = moduleBlockContaining("await isRevoked()");
    const code = registering
      .replace(/\/\*[\s\S]*?\*\//g, " ")
      .replace(/\/\/[^\n]*/g, " ");

    assert.match(
      code,
      /navigator\.serviceWorker\.getRegistration\(\)/,
      "the page kill switch should target only its own registration",
    );
    assert.doesNotMatch(
      code,
      /navigator\.serviceWorker\.getRegistrations\(\)/,
      "withdrawal must not unregister unrelated service-worker scopes",
    );
    assert.match(
      code,
      /presentBootFailure\(WITHDRAWN_BOOT_FAILURE\)/,
      "the withdrawal copy must win before the readiness gate's generic fallback",
    );
  });

  test("activation failures terminalize with safe actionable copy", () => {
    const registering = moduleBlockContaining("serviceWorkerActivation.catch");
    const updatePrompt = moduleBlockContaining(
      "await self.serviceWorkerActivates()",
    );

    assert.match(
      registering,
      /presentBootFailure\(\s*\/module\|type\/i\.test\(detail\)\s*\?\s*OLD_BROWSER_BOOT_FAILURE\s*:\s*GENERIC_BOOT_FAILURE\s*,?\s*\)/,
      "the eager activation owner should terminalize with specific old-browser or generic safe copy",
    );
    assert.match(
      registering,
      /Your local data is safe\./,
      "generic activation failure copy should describe the local-data safety boundary",
    );
    assert.match(
      registering,
      /Safari 16\.4\+/,
      "module-worker failures should retain the supported-browser next action",
    );
    assert.doesNotMatch(
      updatePrompt,
      /Tonk could not start:/,
      "the later update-prompt module must not surface raw exception text",
    );
  });
});
