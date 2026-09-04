import init, { activate } from "./worker.js";

// ---- Build identity --------------------------------------------------
//
// These constants are REWRITTEN IN PLACE by `scripts/stamp-service-worker.sh` at
// post-build. The values below are the dev placeholders; a built dist
// always carries real hashes.
//
// `BUILD_ID` covers this outer policy, worker glue/Wasm, and the canonical
// browser resource graph, so the browser's byte-comparison update check fires
// whenever any part changes. It also names the per-version caches, so two
// builds can never share cache state.
//
// `WORKER_WASM_HASH` is the sha256 prefix of `worker_bg.wasm` as built
// ALONGSIDE this exact glue. `ASSET_MANIFEST_HASH` is the full sha256 of the
// publisher-generated UI/guest resource graph. A first install verifies only
// its matching worker Wasm before activation; it verifies and publishes the
// complete offline graph atomically after taking control. Successor workers
// still complete that graph behind the incumbent before replacing it.
const BUILD_ID = "dev";
const WORKER_WASM_HASH = "dev";
const ASSET_MANIFEST_HASH = "dev";
// Replaced with the exact publisher-produced resource graph. The dev sentinel
// preserves Trunk's live-file behavior; a stamped worker serves only members
// it proved complete before publishing an offline generation.
const ASSET_PATHS = ["dev"];
const ASSET_PATH_SET = new Set(ASSET_PATHS);

const log = (...args) => console.log("[Tonk Service Worker]", ...args);

// ---- Introspection -------------------------------------------------
//
// Everything the worker logs is captured into a bounded ring, and
// `GET /api/health` answers from this glue WITHOUT the wasm worker —
// so a worker that fails to initialize is still diagnosable with one
// fetch from any page, instead of spelunking serviceworker-internals.
const LOG_RING_CAPACITY = 400;
const logRing = [];
const ringify = level => {
    const original = console[level].bind(console);
    console[level] = (...args) => {
        const message = args
            .map(arg => {
                if (typeof arg === "string") return arg;
                try {
                    return arg instanceof Error ? (arg.stack || String(arg)) : JSON.stringify(arg);
                } catch {
                    return String(arg);
                }
            })
            .join(" ");
        logRing.push({ t: Date.now(), level, message: message.slice(0, 2000) });
        if (logRing.length > LOG_RING_CAPACITY) logRing.shift();
        original(...args);
    };
};
["log", "warn", "error"].forEach(ringify);
self.addEventListener("unhandledrejection", event => {
    logRing.push({
        t: Date.now(),
        level: "error",
        message: `unhandledrejection: ${event.reason?.stack || event.reason}`.slice(0, 2000),
    });
    if (logRing.length > LOG_RING_CAPACITY) logRing.shift();
});

// Worker-init observability: state, the last failure, and how many
// times initialization has been attempted.
const workerHealth = {
    state: "idle", // idle | initializing | ok | failed
    // Digest of the exact ArrayBuffer handed to wasm-bindgen init. This is
    // runtime evidence that the active glue booted its own verified Wasm, not
    // merely that the expected bytes existed in an install fixture.
    workerWasm: null,
    error: null,
    attempts: 0,
    lastAttemptAt: null,
    startedAt: Date.now(),
};

function healthResponse() {
    return new Response(
        JSON.stringify({
            build: BUILD_ID,
            worker: workerHealth.state,
            workerWasm: workerHealth.workerWasm,
            error: workerHealth.error,
            attempts: workerHealth.attempts,
            lastAttemptAt: workerHealth.lastAttemptAt,
            startedAt: workerHealth.startedAt,
            log: logRing.slice(-200),
        }),
        {
            status: 200,
            headers: {
                "content-type": "application/json",
                // `/api/health` is answered by this JS shortcut rather than
                // Rust's common CORS layer. Opaque-origin guests still need
                // the actual response to pass CORS after their preflight.
                "access-control-allow-origin": "*",
            },
        },
    );
}

// ---- Caches ----------------------------------------------------------
//
// Both caches are named per BUILD, not per schema version. Two workers
// from different builds therefore never read or write the same cache:
// an install populates its OWN cache. An incoming install may read a member
// from an older final Tonk cache only after hashing it against the incoming
// manifest. Once the verified successor is active, it removes obsolete cache
// names that are proved to belong to this lifecycle protocol.
// Separate names still make an install atomic — a half-populated incoming
// cache can't be observed by the still-serving old worker,
// which previously could hand out the new shell beside the old build's
// hashed assets during the swap window.
//
// The Rust side derives its shell name from the build id handed to it at
// activate time (see `cache.rs`), so the name is injected once here rather
// than hand-synced across two languages. Browser storage pressure may evict
// entries at any time; protocol cleanup begins only after the successor is the
// active, adopted generation.
const SHELL_CACHE = `TONK_SHELL_${BUILD_ID}`;

// Where this worker's own wasm lives. Separate from the shell graph because it
// is instantiated as bytes rather than returned as an ordinary response.
const WORKER_CACHE = `TONK_WORKER_${BUILD_ID}`;
// A first install verifies and pins only the worker's own executable runtime
// before activation. The complete offline generation is assembled after the
// page is controlled, then this bootstrap copy is superseded by WORKER_CACHE.
const RUNTIME_CACHE = `TONK_RUNTIME_${BUILD_ID}`;
const GENERATION_CACHE = `TONK_GENERATION_${BUILD_ID}`;
const GENERATION_MARKER_URL = new URL(
    `./.tonk-generation-${BUILD_ID}`,
    self.location.href,
).href;
const SHELL_STAGE_PREFIX = `TONK_SHELL_STAGE_${BUILD_ID}_`;
const WORKER_STAGE_PREFIX = `TONK_WORKER_STAGE_${BUILD_ID}_`;
const WORKER_WASM_URL = new URL("./worker_bg.wasm", self.location.href).href;
const ASSET_MANIFEST_URL = new URL("./asset-manifest.json", self.location.href).href;

const FINAL_SHELL_CACHE_RE = /^TONK_SHELL_([0-9a-f]{16})$/;
const FINAL_WORKER_CACHE_RE = /^TONK_WORKER_([0-9a-f]{16})$/;
const RUNTIME_CACHE_RE = /^TONK_RUNTIME_([0-9a-f]{16})$/;
const GENERATION_CACHE_RE = /^TONK_GENERATION_([0-9a-f]{16})$/;
const SHELL_STAGE_CACHE_RE = /^TONK_SHELL_STAGE_([0-9a-f]{16})_([0-9a-f]{32})$/;
const WORKER_STAGE_CACHE_RE = /^TONK_WORKER_STAGE_([0-9a-f]{16})_([0-9a-f]{32})$/;

function parsedBuild(name, pattern) {
    return pattern.exec(name)?.[1] ?? null;
}

function parseFinalShellGeneration(name) {
    return parsedBuild(name, FINAL_SHELL_CACHE_RE);
}

function parseFinalWorkerGeneration(name) {
    return parsedBuild(name, FINAL_WORKER_CACHE_RE);
}

function parseRuntimeGeneration(name) {
    return parsedBuild(name, RUNTIME_CACHE_RE);
}

function parseGenerationMarkerCache(name) {
    return parsedBuild(name, GENERATION_CACHE_RE);
}

function parseShellStageGeneration(name) {
    return parsedBuild(name, SHELL_STAGE_CACHE_RE);
}

function parseWorkerStageGeneration(name) {
    return parsedBuild(name, WORKER_STAGE_CACHE_RE);
}

let tonkServiceWorkerResolves;

// Static import at top-level is forced on us: dynamic `import()`
// is disallowed inside a ServiceWorkerGlobalScope per the HTML
// spec, and `importScripts()` is incompatible with module SWs.
// So the worker bundle costs ~1 s of `workerStart` time on cold
// navigations until we find a better way to defer it.
// How long a failed initialization parks further attempts. A failed
// init used to be memoized forever (the rejected promise was cached),
// so one transient failure — storage still settling, a race at boot —
// bricked the worker until the browser discarded it. Now a failure is
// recorded for `/api/health`, and the next fetch after the hold-off
// retries from scratch.
const INIT_RETRY_HOLDOFF_MS = 5000;

// ---- Worker wasm: one atomic artifact set ----------------------------
//
// `service_worker.js` and its static import `worker.js` are pinned in
// the browser's SW script resource map at install time: the browser
// re-runs those exact bytes for the registration's lifetime. But
// `worker_bg.wasm` used to be fetched by `init()` at RUNTIME from a
// fixed URL, through the ordinary HTTP cache — which always answers
// with the newest deployed bytes.
//
// So after every deploy, any not-yet-updated worker that cold-started
// ran OLD GLUE AGAINST NEW WASM. Glue and wasm are tightly coupled
// (export indices, shim names), so that is an init failure at best and
// silent miswiring at worst. Safari terminates idle workers within
// seconds, which made the cold boot — and the skew — near-certain
// there: "stuck on a broken old version no matter what".
//
// The fix is to make the worker self-contained. At install we fetch the
// wasm, verify it against the hash stamped alongside THIS glue, and
// store it in a per-build cache. `init()` instantiates from those cached bytes.
// If storage evicts them, recovery accepts a `no-store` fetch only after the
// same digest check and never writes it back into the retained generation.

/// Full sha256 of `bytes`. Worker Wasm compares the stamped 16-character
/// prefix; the build-produced resource manifest and its assets use all 64
/// characters.
async function digestOf(bytes) {
    const digest = await crypto.subtle.digest("SHA-256", bytes);
    return Array.from(new Uint8Array(digest))
        .map(b => b.toString(16).padStart(2, "0"))
        .join("");
}

function noteVerifiedReadProgress(onChunk, received) {
    if (!onChunk) return;
    try {
        Promise.resolve(onChunk(received)).catch(() => {});
    } catch {
        // Install progress is optional observability. A broken transport must
        // never affect the bytes, hash, or outcome of the verified read.
    }
}

async function readResponseClone(response, onChunk) {
    const clone = response.clone();
    if (!clone.body || typeof clone.body.getReader !== "function") {
        return clone.arrayBuffer();
    }

    const reader = clone.body.getReader();
    const chunks = [];
    let received = 0;
    while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        if (!value || value.byteLength === 0) continue;
        const chunk = value instanceof Uint8Array
            ? value
            : new Uint8Array(value);
        chunks.push(chunk);
        received += chunk.byteLength;
        noteVerifiedReadProgress(onChunk, received);
    }

    const bytes = new Uint8Array(received);
    let offset = 0;
    for (const chunk of chunks) {
        bytes.set(chunk, offset);
        offset += chunk.byteLength;
    }
    return bytes.buffer;
}

async function fetchVerified(url, expectedHash, label, onChunk = null) {
    const response = await fetch(url, { cache: "no-store" });
    if (!response.ok) {
        throw new Error(`${label} fetch failed: ${response.status}`);
    }
    // Hash a clone so the original network response retains its URL list and
    // unconsumed body when CacheStorage stores it. Reconstructing a synthetic
    // Response would discard that metadata and can change relative asset
    // resolution even when the verified bytes are identical.
    const bytes = await readResponseClone(response, onChunk);
    if (expectedHash !== "dev") {
        const actual = await digestOf(bytes);
        if (actual.slice(0, expectedHash.length) !== expectedHash) {
            throw new Error(
                `${label} hash mismatch: expected ${expectedHash}, got ${actual}`,
            );
        }
    }
    return { bytes, response };
}

async function fetchVerifiedWorkerWasm(cacheNames = []) {
    const cached = await verifiedGenerationResponse(
        cacheNames,
        WORKER_WASM_URL,
        WORKER_WASM_HASH,
    );
    if (cached) return cached.arrayBuffer();
    let firstChunk = true;
    return (await fetchVerified(
        WORKER_WASM_URL,
        WORKER_WASM_HASH,
        "worker wasm",
        received => {
            const force = firstChunk;
            firstChunk = false;
            return reportInstallProgress(
                "verify-worker",
                received,
                received,
                force,
            );
        },
    )).bytes;
}

function validateAssetManifest(manifest) {
    if (
        manifest?.version !== 1 ||
        manifest?.build !== BUILD_ID ||
        manifest.assets == null ||
        typeof manifest.assets !== "object" ||
        Array.isArray(manifest.assets)
    ) {
        throw new Error("asset manifest does not describe this build");
    }

    const entries = Object.entries(manifest.assets);
    if (!entries.some(([path]) => path === "/")) {
        throw new Error("asset manifest has no root document");
    }
    for (const [path, hash] of entries) {
        if (typeof path !== "string" || !/^\/[^\x00-\u001f]*$/.test(path)) {
            throw new Error(`asset manifest has invalid path: ${path}`);
        }
        const url = new URL(path, self.location.origin);
        if (
            url.origin !== self.location.origin ||
            url.pathname !== path ||
            url.search !== "" ||
            url.hash !== "" ||
            path.startsWith("/api/")
        ) {
            throw new Error(`asset manifest path escapes the static graph: ${path}`);
        }
        if (typeof hash !== "string" || !/^[0-9a-f]{64}$/.test(hash)) {
            throw new Error(`asset manifest has invalid hash for ${path}`);
        }
    }
    return entries;
}

async function fetchVerifiedAssetManifest() {
    const { bytes } = await fetchVerified(
        ASSET_MANIFEST_URL,
        ASSET_MANIFEST_HASH,
        "asset manifest",
    );
    let manifest;
    try {
        manifest = JSON.parse(new TextDecoder().decode(bytes));
    } catch {
        throw new Error("asset manifest is not valid JSON");
    }
    return validateAssetManifest(manifest);
}

/// Recover one evicted member without mutating the retained generation. The
/// live manifest must still be byte-for-byte the manifest stamped into this
/// worker, and the resource must match its manifest digest. A newer deploy,
/// an offline network, or a corrupt response therefore fails closed.
async function fetchVerifiedRetainedAsset(path) {
    const entries = await fetchVerifiedAssetManifest();
    const expectedHash = new Map(entries).get(path);
    if (!expectedHash) {
        throw new Error(`asset manifest has no retained path: ${path}`);
    }
    return (await fetchVerified(
        new URL(path, self.location.origin).href,
        expectedHash,
        `asset ${path}`,
    )).response;
}

let installProgressReportedAt = 0;
const INSTALL_PROGRESS_CHANNEL = "tonk-sw-install-progress-v1";

/// Keep the uncontrolled bootstrap document's stall watchdog informed while
/// this worker verifies and seals a large production graph. Progress is a
/// liveness hint only: failures are deliberately ignored and cannot make an
/// incomplete generation installable.
async function reportInstallProgress(phase, completed, total, force = false) {
    const now = Date.now();
    if (!force && now - installProgressReportedAt < 5_000) return;
    installProgressReportedAt = now;
    try {
        const channel = new BroadcastChannel(INSTALL_PROGRESS_CHANNEL);
        channel.postMessage({
            type: "tonk-install-progress",
            build: BUILD_ID,
            phase,
            completed,
            total,
        });
        channel.close();
    } catch {
        // The client-message path below covers browsers without a worker-side
        // BroadcastChannel implementation.
    }
    // Client discovery is only a compatibility fallback for browsers without
    // worker-side BroadcastChannel. In particular, Chrome can leave
    // `matchAll({ includeUncontrolled: true })` pending while this worker is
    // still installing. Never let that optional lookup hold the verified
    // generation transaction open.
    try {
        void self.clients
            .matchAll({ type: "window", includeUncontrolled: true })
            .then(windows => {
                for (const client of windows) {
                    client.postMessage({
                        type: "tonk-install-progress",
                        build: BUILD_ID,
                        phase,
                        completed,
                        total,
                    });
                }
            })
            .catch(() => {});
    } catch {
        // Progress exists only to distinguish a slow verified install from a
        // silent stall; install correctness never depends on delivery.
    }
}

async function verifiedGenerationResponse(cacheNames, key, expectedHash) {
    for (const cacheName of cacheNames) {
        try {
            const response = await caches.match(key, { cacheName });
            if (!response) continue;
            if (expectedHash === "dev") return response;
            const actual = await digestOf(await response.clone().arrayBuffer());
            if (actual.slice(0, expectedHash.length) === expectedHash) {
                return response;
            }
        } catch (error) {
            log(`Unable to verify reusable response from ${cacheName}:`, error);
        }
    }
    return null;
}

/// Pin the one generated artifact that must match this service-worker glue
/// before the worker can safely control a page. Fetch and verify before opening
/// the cache so a bad deployment cannot leave a new partial runtime behind.
async function installWorkerRuntime() {
    if (await verifiedGenerationResponse(
        [RUNTIME_CACHE],
        WORKER_WASM_URL,
        WORKER_WASM_HASH,
    )) return;

    const names = await caches.keys();
    const reusable = names.filter(name => {
        const build = parseFinalWorkerGeneration(name);
        return build != null && build !== BUILD_ID;
    });
    const bytes = await fetchVerifiedWorkerWasm(reusable);
    const runtimeExisted = names.includes(RUNTIME_CACHE);
    try {
        const runtime = await caches.open(RUNTIME_CACHE);
        await runtime.put(WORKER_WASM_URL, new Response(bytes, {
            headers: { "content-type": "application/wasm" },
        }));
    } catch (error) {
        if (!runtimeExisted) await caches.delete(RUNTIME_CACHE);
        throw error;
    }
}

async function fetchVerifiedAssets(entries, cacheNames = []) {
    const results = new Array(entries.length);
    let next = 0;
    let completed = 0;
    const fetchNext = async () => {
        while (next < entries.length) {
            const index = next++;
            const [path, hash] = entries[index];
            const url = new URL(path, self.location.origin).href;
            const cached = await verifiedGenerationResponse(
                cacheNames,
                assetCacheKey(path),
                hash,
            );
            if (cached) {
                results[index] = { path, response: cached };
                completed += 1;
                await reportInstallProgress("verify", completed, entries.length);
                continue;
            }
            let firstChunk = true;
            const { response } = await fetchVerified(
                url,
                hash,
                `asset ${path}`,
                () => {
                    const force = firstChunk;
                    firstChunk = false;
                    return reportInstallProgress(
                        "verify",
                        completed,
                        entries.length,
                        force,
                    );
                },
            );
            results[index] = { path, response };
            completed += 1;
            await reportInstallProgress("verify", completed, entries.length);
        }
    };
    const concurrency = Math.min(8, entries.length);
    await Promise.all(Array.from({ length: concurrency }, fetchNext));
    return results;
}

function assetCacheKey(path) {
    return path === "/" ? "/" : new URL(path, self.location.origin).href;
}

async function cachedResponseMatches(cacheName, key, expectedHash) {
    const response = await caches.match(key, { cacheName });
    if (!response) return false;
    return (await digestOf(await response.arrayBuffer())) === expectedHash;
}

async function existingGenerationIsComplete(
    entries,
    shellCache = SHELL_CACHE,
    workerCache = WORKER_CACHE,
) {
    // CacheStorage.match is deliberately read-only. Storage pressure can evict
    // a retained cache after the names inventory; opening it here would
    // silently recreate an empty old-generation cache during verification.
    const wasm = await caches.match(WORKER_WASM_URL, { cacheName: workerCache });
    if (!wasm) return false;
    if (WORKER_WASM_HASH !== "dev") {
        const actual = await digestOf(await wasm.arrayBuffer());
        if (actual.slice(0, WORKER_WASM_HASH.length) !== WORKER_WASM_HASH) {
            return false;
        }
    }
    for (const [path, hash] of entries) {
        if (!(await cachedResponseMatches(shellCache, assetCacheKey(path), hash))) {
            return false;
        }
    }
    return true;
}

function freshStageNonce() {
    const bytes = new Uint8Array(16);
    crypto.getRandomValues(bytes);
    return Array.from(bytes, byte => byte.toString(16).padStart(2, "0")).join("");
}

function generationMarker(state, nonce) {
    return {
        version: 1,
        build: BUILD_ID,
        manifest: ASSET_MANIFEST_HASH,
        state,
        nonce,
        shellStage: `${SHELL_STAGE_PREFIX}${nonce}`,
        workerStage: `${WORKER_STAGE_PREFIX}${nonce}`,
    };
}

function validateGenerationMarker(value) {
    if (
        value?.version !== 1 ||
        value?.build !== BUILD_ID ||
        value?.manifest !== ASSET_MANIFEST_HASH ||
        !["building", "publishing", "adopted"].includes(value?.state) ||
        typeof value?.nonce !== "string" ||
        !/^[0-9a-f]{32}$/.test(value.nonce) ||
        value?.shellStage !== `${SHELL_STAGE_PREFIX}${value.nonce}` ||
        value?.workerStage !== `${WORKER_STAGE_PREFIX}${value.nonce}`
    ) {
        throw new Error("generation provenance marker is invalid");
    }
    return value;
}

async function readGenerationMarker() {
    const response = await caches.match(GENERATION_MARKER_URL, {
        cacheName: GENERATION_CACHE,
    });
    if (!response) return null;
    try {
        return validateGenerationMarker(await response.json());
    } catch (error) {
        if (error?.message === "generation provenance marker is invalid") throw error;
        throw new Error("generation provenance marker is invalid");
    }
}

async function writeGenerationMarker(marker) {
    const metadata = await caches.open(GENERATION_CACHE);
    await metadata.put(
        GENERATION_MARKER_URL,
        new Response(JSON.stringify(marker), {
            headers: { "content-type": "application/json" },
        }),
    );
}

async function cleanInterruptedGeneration(marker) {
    const names = [marker.shellStage, marker.workerStage];
    if (marker.state === "publishing") {
        // The marker moves to `publishing` before either final cache is opened.
        // Therefore these stable names cannot have controlled a page yet.
        names.push(SHELL_CACHE, WORKER_CACHE);
    }
    const results = await Promise.allSettled(names.map(name => caches.delete(name)));
    return results.every(result => result.status === "fulfilled");
}

/// Assemble this generation completely before making its isolated caches
/// visible. Existing cache names are treated as retained immutable state: they
/// are verified and reused when complete, or rejected without repair when
/// incomplete. Only caches newly created by this install are cleanup targets.
async function installGeneration() {
    const entries = await fetchVerifiedAssetManifest();
    await reportInstallProgress("verify", 0, entries.length, true);
    const retainedMarker = await readGenerationMarker();
    let names = new Set(await caches.keys());
    let hadShell = names.has(SHELL_CACHE);
    let hadWorker = names.has(WORKER_CACHE);

    if (retainedMarker?.state === "adopted") {
        if (hadShell && hadWorker && await existingGenerationIsComplete(entries)) {
            // A crash after adoption but before staging cleanup can leave these
            // provably-unadopted names behind. The final generation is never
            // opened for mutation on this path.
            await Promise.allSettled([
                caches.delete(retainedMarker.shellStage),
                caches.delete(retainedMarker.workerStage),
            ]);
            return;
        }
        throw new Error("retained generation cache is incomplete");
    }

    if (retainedMarker) {
        // `building` has not touched stable names; `publishing` was durably
        // recorded before it did. Only those two states prove cleanup cannot
        // delete a generation that ever controlled a page.
        if (!(await cleanInterruptedGeneration(retainedMarker))) {
            throw new Error("interrupted generation cleanup failed");
        }
        await caches.delete(GENERATION_CACHE);
        names = new Set(await caches.keys());
        hadShell = names.has(SHELL_CACHE);
        hadWorker = names.has(WORKER_CACHE);
    }

    if (hadShell || hadWorker) {
        // Stable caches without an adopted marker have unknown provenance. Do
        // not infer that they are disposable merely because they are partial.
        throw new Error("retained generation cache has no adoption provenance");
    }

    const reusableShellCaches = [...names].filter(name => {
        const build = parseFinalShellGeneration(name);
        return build != null && build !== BUILD_ID;
    });
    const reusableWorkerCaches = [...names].filter(name => {
        const build = parseFinalWorkerGeneration(name);
        return build != null && build !== BUILD_ID;
    });
    const [assets, wasmBytes] = await Promise.all([
        fetchVerifiedAssets(entries, reusableShellCaches),
        fetchVerifiedWorkerWasm([RUNTIME_CACHE, ...reusableWorkerCaches]),
    ]);

    const nonce = freshStageNonce();
    let marker = generationMarker("building", nonce);
    let markerWritten = false;
    try {
        // This durable intent is written before either unique staging name is
        // opened. A same-build retry can therefore identify exactly which
        // caches an interrupted attempt owned without guessing from a prefix.
        await writeGenerationMarker(marker);
        markerWritten = true;
        const shellStage = await caches.open(marker.shellStage);
        const workerStage = await caches.open(marker.workerStage);
        let staged = 0;
        for (const { path, response } of assets) {
            await shellStage.put(assetCacheKey(path), response.clone());
            staged += 1;
            await reportInstallProgress("stage", staged, assets.length);
        }
        await workerStage.put(
            WORKER_WASM_URL,
            new Response(wasmBytes, {
                headers: { "content-type": "application/wasm" },
            }),
        );
        if (!(await existingGenerationIsComplete(entries, marker.shellStage, marker.workerStage))) {
            throw new Error("staged generation cache is incomplete");
        }

        // From this point until `adopted`, stable names are known incoming
        // publication state and are safe for this same build to replace after
        // abrupt termination. No page can use them until install resolves.
        marker = generationMarker("publishing", nonce);
        await writeGenerationMarker(marker);
        const shell = await caches.open(SHELL_CACHE);
        const worker = await caches.open(WORKER_CACHE);
        let published = 0;
        for (const { path, response } of assets) {
            await shell.put(assetCacheKey(path), response);
            published += 1;
            await reportInstallProgress("publish", published, assets.length);
        }
        await worker.put(
            WORKER_WASM_URL,
            new Response(wasmBytes, {
                headers: { "content-type": "application/wasm" },
            }),
        );
        if (!(await existingGenerationIsComplete(entries))) {
            throw new Error("published generation cache is incomplete");
        }

        // One Cache.put is the adoption commit. Retrying after this point may
        // verify/reuse final caches, but can never mutate or delete them.
        marker = generationMarker("adopted", nonce);
        await writeGenerationMarker(marker);
        await Promise.allSettled([
            caches.delete(marker.shellStage),
            caches.delete(marker.workerStage),
            caches.delete(RUNTIME_CACHE),
        ]);
        await reportInstallProgress("adopted", assets.length, assets.length, true);
    } catch (error) {
        if (markerWritten && marker.state !== "adopted") {
            const cleaned = await cleanInterruptedGeneration(marker);
            if (cleaned) await caches.delete(GENERATION_CACHE);
        }
        throw error;
    }
}

function lifecycleCacheBuild(name) {
    return parseFinalShellGeneration(name) ??
        parseFinalWorkerGeneration(name) ??
        parseRuntimeGeneration(name) ??
        parseGenerationMarkerCache(name) ??
        parseShellStageGeneration(name) ??
        parseWorkerStageGeneration(name);
}

/// Remove only obsolete names owned by the immutable-generation protocol.
/// Activation is the first point at which the browser has made this worker
/// the registration's active worker, so install failure can never prune the
/// incumbent that still serves existing documents.
async function pruneObsoleteGenerationCaches() {
    const marker = await readGenerationMarker();
    if (marker?.state !== "adopted") {
        log("Skipping generation cache cleanup without an adopted current marker");
        return;
    }

    const names = await caches.keys();
    const obsolete = names.filter(name => {
        const build = lifecycleCacheBuild(name);
        return build != null && build !== BUILD_ID;
    });
    const results = await Promise.allSettled(obsolete.map(name => caches.delete(name)));
    results.forEach((result, index) => {
        if (result.status === "rejected") {
            log(`Failed to delete obsolete generation cache ${obsolete[index]}:`, result.reason);
        }
    });
}

let offlineGenerationResolves;

/// Complete offline availability after control without making it a readiness
/// prerequisite. The shared promise prevents duplicate fills within one worker
/// lifetime; the durable marker makes later worker instances return cheaply.
function ensureOfflineGeneration() {
    if (offlineGenerationResolves == null) {
        offlineGenerationResolves = (async () => {
            const marker = await readGenerationMarker();
            if (marker?.state !== "adopted") await installGeneration();
            await pruneObsoleteGenerationCaches();
        })().catch(error => {
            offlineGenerationResolves = null;
            log("Offline generation fill failed; it will retry later:", error);
        });
    }
    return offlineGenerationResolves;
}

function extendOfflineGeneration(event) {
    if (typeof event.waitUntil !== "function") return;
    event.waitUntil(ensureOfflineGeneration());
}

/// The wasm bytes this worker boots from: prefer the complete offline
/// generation, then the first-install runtime pin. If storage pressure evicts
/// both entries, recover only by fetching with `no-store` and re-verifying
/// against this glue's stamp. The verified bytes may boot this instance but
/// must not backfill the retained cache.
async function workerWasmModule() {
    const cached = await caches.match(WORKER_WASM_URL, { cacheName: WORKER_CACHE }) ??
        await caches.match(WORKER_WASM_URL, { cacheName: RUNTIME_CACHE });
    if (cached) return cached.arrayBuffer();
    log("Worker wasm missing from cache — refetching");
    return fetchVerifiedWorkerWasm();
}

async function activateWorker() {
    if (tonkServiceWorkerResolves == null) {
        const now = Date.now();
        if (
            workerHealth.state === "failed" &&
            now - workerHealth.lastAttemptAt < INIT_RETRY_HOLDOFF_MS
        ) {
            throw new Error(`worker initialization failed: ${workerHealth.error}`);
        }
        workerHealth.state = "initializing";
        workerHealth.attempts += 1;
        workerHealth.lastAttemptAt = now;
        tonkServiceWorkerResolves = workerWasmModule()
            .then(async module_or_path => {
                if (WORKER_WASM_HASH === "dev") {
                    workerHealth.workerWasm = "dev";
                    return init({ module_or_path });
                }
                const actual = await digestOf(module_or_path);
                if (actual.slice(0, WORKER_WASM_HASH.length) !== WORKER_WASM_HASH) {
                    throw new Error(
                        `worker wasm hash mismatch: expected ${WORKER_WASM_HASH}, got ${actual}`,
                    );
                }
                workerHealth.workerWasm = actual.slice(0, WORKER_WASM_HASH.length);
                return init({ module_or_path });
            })
            .then(() => activate(BUILD_ID, ASSET_PATHS))
            .then(worker => {
                workerHealth.state = "ok";
                workerHealth.error = null;
                return worker;
            })
            .catch(error => {
                workerHealth.state = "failed";
                workerHealth.workerWasm = null;
                workerHealth.error = String(error?.message || error).slice(0, 2000);
                // Un-memoize so a later fetch retries instead of
                // replaying this rejection for the worker's lifetime.
                tonkServiceWorkerResolves = null;
                log("Worker initialization failed:", error);
                throw error;
            });
        log("Worker initialized");
    }

    return tonkServiceWorkerResolves;
}

self.oninstall = event => {
    event.waitUntil((async () => {
        // A first page must not wait for the whole offline graph. Pin only the
        // worker's glue-bound Wasm, then assemble the graph under later
        // fetch/message lifetimes. An update still fills behind its live
        // incumbent before takeover, preserving offline-safe replacement.
        if (self.registration.active == null) {
            await installWorkerRuntime();
        } else {
            await installGeneration();
        }
        // One-release rollout bridge: pages deployed before this lifecycle
        // protocol never send an `activate` message. A verified successor must
        // therefore activate automatically even when an incumbent exists.
        // Keep this until every supported deployed page generation is known to
        // understand the successor protocol.
        await self.skipWaiting();
    })());
    log("Installed");
};

self.onactivate = event => {
    // Activation replaces this registration's active worker for clients that
    // it already controlled, and those documents receive `controllerchange`.
    // Do not call `clients.claim()` here: an explicit claim is reserved for an
    // otherwise-uncontrolled first-install document.
    //
    // The wasm worker is still poked outside waitUntil: activateWorker()
    // waits for the active-worker lock, and gating ACTIVATION on that lock
    // deadlocks the swap — the outgoing worker cannot die while its
    // in-flight fetches hang, the lock never frees, and this worker pins in
    // `activating` while every page waits on it.
    (async () => {
        try {
            const worker = await activateWorker();
            await worker.onactivate?.();
        } catch (err) {
            log("onactivate dispatch failed:", err);
        }
    })();
    event.waitUntil?.(
        pruneObsoleteGenerationCaches().catch(error => {
            log("Generation cache cleanup failed:", error);
        }),
    );
    log("Activated");
};

// When a *newer* version completes installation, this script (the currently
// active worker) is on the way out. Forward the lifecycle event to the Rust
// side so it can release every long-lived response we're serving — chiefly
// the query and language-server SSE streams. With those streams hung up, the
// outgoing instance can terminate after the successor takes over.
//
// `worker.onupdatefound()` is exported by `tonk_worker::worker`; it publishes
// the shared retirement latch and drops the query and LSP senders. Receivers
// see EOF, then the browser resumes against the new worker.
/// Whether this worker has already released its streams for an installed
/// successor. Per worker instance, which is what we want: a restarted
/// worker re-checks. Declared here, above every use — a `let` read
/// before its declaration is a runtime TDZ error, not a hoist.
let retired = false;
let retirement = null;

/// Release this worker's long-lived streams because a successor installed.
/// Idempotent: safe after the observed candidate reaches `installed` and again
/// when a waiting successor is found during an incumbent restart.
async function retire(reason) {
    if (retired) return true;
    if (retirement) return retirement;
    retirement = (async () => {
        log(`Retiring — ${reason}`);
        try {
            const worker = await activateWorker();
            // Stop the sync loop and release long-lived streams so this
            // instance winds down. Serving can overlap briefly with successor
            // activation while the browser settles in-flight work, which storage
            // is designed to tolerate (CAS commits over content-addressed
            // blocks; transaction settling is race-armed). Nothing here may
            // couple the successor's startup to this worker's death: worker
            // lifecycles belong to the browser.
            await worker.onupdatefound?.();
            // Publish the one-way latch only after release succeeds. A failed
            // worker boot or hook must remain retryable on the next fetch.
            retired = true;
            return true;
        } catch (err) {
            log("Failed to release streams:", err);
            return false;
        }
    })();
    try {
        return await retirement;
    } finally {
        retirement = null;
    }
}

/// A registration is shared by the active, waiting, and installing worker
/// globals. Only the worker object that the browser identifies as the active
/// incumbent may stop the reactor serving the current pages.
function isActiveIncumbent() {
    return self.registration.active === self.serviceWorker;
}

function retireActiveIncumbent(reason) {
    if (!isActiveIncumbent()) return Promise.resolve(true);
    return retire(reason);
}

// Catch-up check, run at every script evaluation.
//
// `updatefound` is an EVENT, and a service worker is killed and
// restarted constantly — so the event fires into a dead worker whenever
// the successor installs while this one is asleep. The listener below
// is then re-registered on restart having already missed its only
// notice, and this worker can go on holding streams after the successor's
// automatic activation. The replacement then owns the registration while
// stale clients remain pinned to the unreleased incumbent.
//
// The registration itself is durable where the event is not, so ask it
// directly on every startup instead of trusting that we were awake.
if (self.registration.waiting) {
    Promise.resolve().then(() =>
        retireActiveIncumbent("a successor was already waiting at startup"),
    );
}

function watchSuccessor(candidate) {
    if (!candidate) return;
    // The event fires on the REGISTRATION, so every worker running this script
    // hears it — including the newly-installing worker, about its own arrival.
    // Only the OUTGOING worker may act on it, and only after the candidate
    // reaches `installed`: `onupdatefound` stops all sync work, and a retiring
    // worker must not re-arm `waitUntil` or keep itself serving stale clients.
    // A new worker that ran it would latch itself off and then go on to serve
    // the page while refusing every sync drain for the rest of its life.
    //
    // `registration.installing` is the incoming worker. If that is us, this is
    // our own birth announcement, not our eviction notice.
    if (!isActiveIncumbent()) {
        log("Update found — this worker is not the active incumbent; staying live");
        return;
    }

    const observe = () => {
        if (["installed", "activating", "activated"].includes(candidate.state)) {
            candidate.removeEventListener?.("statechange", observe);
            return retireActiveIncumbent("a newer worker installed successfully");
        }
        if (candidate.state === "redundant") {
            candidate.removeEventListener?.("statechange", observe);
            log("Successor install failed — incumbent stays live");
        }
    };
    candidate.addEventListener?.("statechange", observe);
    return observe();
}

// An installing candidate is not a successor yet: manifest, asset, quota, or
// network failure can still make it redundant. Irreversible stream/sync
// teardown begins only after its state proves installation succeeded.
watchSuccessor(self.registration.installing);
self.registration.addEventListener?.("updatefound", () => {
    return watchSuccessor(self.registration.installing);
});

// Connectivity transitions. The Rust side reads `navigator.onLine` itself
// (reliable in the SW scope) and reconciles — an offline reading stamps
// `sync:offline`, an online one runs a drain. We only need to POKE it on a
// transition; the decision is the worker's. The SW's own `offline`/`online`
// events fire the poke, and so does a `{type:"connectivity"}` message from the
// active page (belt-and-suspenders — the page's events are the ones guaranteed
// to fire, and it forwards them here).
const onConnectivityChange = async () => {
    try {
        const worker = await activateWorker();
        await worker.onconnectivity?.();
    } catch (err) {
        log("Failed to handle connectivity change:", err);
    }
};
self.addEventListener("offline", onConnectivityChange);
self.addEventListener("online", onConnectivityChange);

// Serve the precached app shell for every route navigation. The
// SPA router resolves the actual path client-side, so there's
// nothing URL-specific to serve — just hand back the shell from
// the cache, which means a navigation never waits on the network
// (a network-first shell blocked slow loads on a full round-trip
// even though the shell was already on disk).
//
// Keyed on `/`, where the origin serves the shell HTML (200).
// `/index.html` is a 307 redirect to `/`, so it can't be cached
// or served as the shell.
//
// A generation cache is SEALED by install. An old controller can keep serving
// after a deployment, so a runtime recovery accepts bytes only when the live
// manifest still matches this worker's stamped manifest and the resource
// matches that manifest. The outgoing document's update-aware
// controller-replacement handoff crosses to the successor; recovery never writes
// current-deployment bytes into an old cache.
function evictionRecoveryResponse() {
    return new Response(
        `<!doctype html><html><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Tonk is checking for an update</title>
<style>
  body { font: 16px/1.5 system-ui, sans-serif; margin: 0; display: grid;
         place-items: center; min-height: 100vh; min-height: 100dvh;
         background: #111; color: #eee; }
  main { max-width: 34rem; padding: 2rem; }
  h1 { font-size: 1.35rem; }
  button { font: inherit; padding: 0.55rem 1.25rem; border-radius: 8px;
           border: 1px solid #555; background: #2a2a2e; color: #eee;
           cursor: pointer; }
</style></head><body><main>
<h1>Checking for a recoverable Tonk version</h1>
<p id="status">A required file is no longer available locally. Tonk is checking for the current version without changing your local data.</p>
<button id="retry" type="button" hidden>Try again</button>
</main><script>
(() => {
  const key = "tonk:sw-eviction-reload";
  const serviceWorkers = navigator.serviceWorker;
  const status = document.getElementById("status");
  const retry = document.getElementById("retry");
  let settled = false;
  let timer = null;
  let updateFound = false;
  let attempt = 0;

  const stop = () => {
    if (timer !== null) clearTimeout(timer);
    serviceWorkers.removeEventListener("controllerchange", adopted);
  };
  const failed = currentAttempt => {
    if (currentAttempt !== attempt || settled) return;
    settled = true;
    stop();
    status.textContent = "Tonk could not switch to a complete current version. Check your connection and try again.";
    retry.hidden = false;
  };
  const adopted = () => {
    if (settled) return;
    settled = true;
    stop();
    try { sessionStorage.setItem(key, "1"); } catch {}
    location.reload();
  };
  const observe = (candidate, currentAttempt) => {
    if (!candidate) return;
    const changed = () => {
      if (candidate.state === "redundant") failed(currentAttempt);
    };
    changed();
    candidate.addEventListener?.("statechange", changed);
  };
  const check = async () => {
    const currentAttempt = ++attempt;
    settled = false;
    updateFound = false;
    retry.hidden = true;
    status.textContent = "A required file is no longer available locally. Tonk is checking for the current version without changing your local data.";
    serviceWorkers.addEventListener("controllerchange", adopted, { once: true });
    timer = setTimeout(() => failed(currentAttempt), 30000);
    try {
      const before = serviceWorkers.controller;
      const registration = await serviceWorkers.getRegistration();
      if (!registration) return failed(currentAttempt);
      registration.addEventListener?.("updatefound", () => {
        if (currentAttempt !== attempt) return;
        updateFound = true;
        observe(registration.installing, currentAttempt);
      }, { once: true });
      await registration.update();
      if (currentAttempt !== attempt) return;
      if (serviceWorkers.controller !== before) return adopted();
      observe(registration.installing || registration.waiting, currentAttempt);
      if (!updateFound && !registration.installing && !registration.waiting) failed(currentAttempt);
    } catch {
      failed(currentAttempt);
    }
  };

  retry.addEventListener("click", check);
  try {
    if (sessionStorage.getItem(key) === "1") {
      sessionStorage.removeItem(key);
      failed(attempt);
      return;
    }
  } catch {}
  void check();
})();
</script></body></html>`,
        {
            status: 503,
            statusText: "Retained Tonk resource unavailable",
            headers: {
                "content-type": "text/html; charset=utf-8",
                "cache-control": "no-store",
                "content-security-policy": "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; worker-src 'self'; base-uri 'none'; form-action 'none'",
            },
        },
    );
}

function missingGenerationAssetResponse(path) {
    if (path === "/") return evictionRecoveryResponse();
    return new Response(
        `A resource required by this retained Tonk version is unavailable (${path}). ` +
            "Reload to check for the current version.",
        {
            status: 503,
            statusText: "Retained Tonk resource unavailable",
            headers: {
                "content-type": "text/plain; charset=utf-8",
                "cache-control": "no-store",
            },
        },
    );
}

async function serveNavigation() {
    const cached = await caches.match("/", { cacheName: SHELL_CACHE });

    if (cached) {
        // The bytes installed for this BUILD_ID are immutable. In particular,
        // do not revalidate a retained offline generation against the live
        // deployment and do not prune assets an old page may still reference.
        return cached;
    }
    try {
        return await fetchVerifiedRetainedAsset("/");
    } catch (error) {
        log("Retained root recovery failed:", error);
        return missingGenerationAssetResponse("/");
    }
}

// Whether a request can be served from the shell cache by the JS shim
// WITHOUT booting the wasm worker. A stamped generation admits only exact
// manifest members; live edge routes and unpublished paths reach Rust/network.
// Only an unstamped development worker honors an authored cache-bypass flag.
// Mirrors
// `cache.rs`'s `is_cacheable`, but lives here so a cached asset is served straight from
// the Cache API — the wasm worker's own bundle can be multiple MB, and
// booting it to serve a static file made every subresource wait on that
// download (fatal on 3G). The Rust worker still owns branch-scoped guest
// rewrites, but it cannot repair or recreate this retained cache.
function isShellCacheable(request, path) {
    if (request.method !== "GET") return false;
    if (request.mode === "navigate") return false;
    if (path.startsWith("/api/")) return false;
    // Same-origin only. Excluding opaque responses isn't enough: a
    // CORS-enabled cross-origin GET yields a perfectly ordinary
    // `basic`-looking success that would be stored in — and later
    // served from — the app's own shell cache, which has no business
    // holding another origin's resources.
    if (new URL(request.url).origin !== self.location.origin) return false;
    if (BUILD_ID !== "dev" && !ASSET_PATH_SET.has(path)) return false;
    const cache = request.cache;
    if (
        BUILD_ID === "dev" &&
        (cache === "no-store" || cache === "reload" || cache === "no-cache")
    ) {
        return false;
    }
    return true;
}

// Serve a static asset from the sealed generation when present (no network,
// no worker boot). On eviction, accept an ephemeral network response only
// after proving that both the live manifest and member still belong to this
// exact generation. Never backfill an old cache from the live deployment.
async function serveAsset(event) {
    // Match ONLY the top-level resource graph installed under the request URL.
    // A guest-iframe subresource is rewritten to a branch-scoped `/api/...`
    // path by the worker, so it can never match here — no cross-serving a
    // guest asset from the top-level cache.
    const path = new URL(event.request.url).pathname;
    const cached = await caches.match(assetCacheKey(path), { cacheName: SHELL_CACHE });
    if (cached) {
        return cached;
    }
    try {
        return await fetchVerifiedRetainedAsset(path);
    } catch (error) {
        log(`Retained asset recovery failed for ${path}:`, error);
        return missingGenerationAssetResponse(path);
    }
}

async function rustFetch(event) {
    return (await activateWorker()).onfetch(event);
}

// A registered guest's subresource may have the same URL as a top-level
// immutable asset. Its client identity, not its pathname, decides whether the
// Rust worker rewrites it into the guest's repository/branch. A missing or
// failed client lookup is delegated conservatively: cross-serving a top-level
// asset into an opaque guest is worse than paying the worker boot cost.
const clientFrameTypes = new Map();
const CLIENT_FRAME_TYPE_LIMIT = 256;

function rememberClientFrameType(clientId, frameType) {
    if (!clientFrameTypes.has(clientId) && clientFrameTypes.size >= CLIENT_FRAME_TYPE_LIMIT) {
        clientFrameTypes.delete(clientFrameTypes.keys().next().value);
    }
    clientFrameTypes.set(clientId, frameType);
}

async function isNestedClientRequest(event) {
    if (!event.clientId) return false;
    const known = clientFrameTypes.get(event.clientId);
    if (known) return known === "nested";
    try {
        const client = await self.clients.get(event.clientId);
        if (!client || !["top-level", "nested"].includes(client.frameType)) {
            return true;
        }
        rememberClientFrameType(event.clientId, client.frameType);
        return client.frameType === "nested";
    } catch (error) {
        log("Client lookup failed; delegating request to Rust:", error);
        return true;
    }
}

async function routeFetch(event, path) {
    if (event.request.mode === "navigate") {
        if (path.startsWith("/api/") || await isNestedClientRequest(event)) {
            return rustFetch(event);
        }
        return serveNavigation();
    }
    if (!isShellCacheable(event.request, path)) {
        return rustFetch(event);
    }
    return await isNestedClientRequest(event) ? rustFetch(event) : serveAsset(event);
}

// Route navigations straight to the cached shell (bypassing the
// Rust worker boot, so TTFB doesn't wait on dialog-repository /
// axum / IndexedDB init); cached static assets are served from the
// Cache API directly (also bypassing the boot); `/api/*` goes through the Rust
// worker, which owns branch-scoped guest rewrites. Missing top-level assets
// fail closed because this generation was complete when it installed.
// ---- Graceful upgrade -----------------------------------------------
//
// A deploy must never brick a page, and worker lifecycles belong to
// the browser: nothing here may couple the successor's startup to the
// outgoing worker's death. On updatefound the outgoing worker stops
// background work and releases streams; it keeps serving until the
// successor activates and replaces their controller (a brief overlap storage
// tolerates by design: CAS commits over content-addressed blocks,
// race-armed transaction settling). Teardown is then the browser's
// default path — it works because every response is fast or bounded
// by the storage settle watchdog, so activation always finds a quiet
// gap and the reaped worker never has a reason to linger.

/// A real error page for a worker that cannot start: the actual error,
/// a retry, and a pointer at /api/health — never an endless spinner.

function failurePage() {
    const detail = String(workerHealth.error || "unknown error");
    const escaped = detail
        .replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
    return new Response(
        `<!doctype html><html><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Tonk failed to start</title>
<style>
  body { font: 16px/1.5 system-ui, sans-serif; margin: 0; display: grid;
         place-items: center; min-height: 100vh; min-height: 100dvh;
         background: #111; color: #eee; }
  main { max-width: 42rem; padding: 2rem; }
  h1 { font-size: 1.3rem; }
  pre { background: #1d1d1f; border: 1px solid #333; border-radius: 8px;
        padding: 1rem; white-space: pre-wrap; word-break: break-word;
        color: #f0b0b0; font-size: 0.85rem; }
  button { font: inherit; padding: 0.5rem 1.25rem; border-radius: 8px;
           border: 1px solid #555; background: #2a2a2e; color: #eee;
           cursor: pointer; }
  p.hint { color: #999; font-size: 0.85rem; }
</style></head><body><main>
<h1>Tonk failed to start</h1>
<p>The storage worker could not initialize. Attempt ${workerHealth.attempts}.</p>
<pre>${escaped}</pre>
<button onclick="location.reload()">Try again</button>
<p class="hint">Diagnostics: <code>/api/health</code> has the full log ring.</p>
</main>
</body></html>`,
        { status: 503, headers: { "content-type": "text/html; charset=utf-8" } },
    );
}

/// Release streams if a successor is waiting and we have not already.
///
/// Called from both the fetch path and an explicit update-aware page nudge. A
/// worker holding an open SSE stream is never killed — the stream keeps it
/// alive — so it never restarts, never re-evaluates this script, and never
/// reaches the startup catch-up above. Either contact can wake it after the
/// successor becomes durable registration state.
function retireIfSuperseded(event) {
    if (retired || !self.registration.waiting || !isActiveIncumbent()) return null;
    const attempt = retireActiveIncumbent("a successor is waiting");
    event.waitUntil?.(attempt);
    return attempt;
}

self.onfetch = event => {
    const path = new URL(event.request.url).pathname;
    if (event.request.mode === "navigate") extendOfflineGeneration(event);
    // A waiting successor means this worker is retiring. Checked on the
    // fetch path because a worker pinned by its own open streams never
    // restarts to run the startup check.
    const retirementAttempt = retireIfSuperseded(event);
    // Answered from this glue, never the wasm worker: health must be
    // readable precisely when the worker cannot answer for itself.
    if (
        path === "/api/health" &&
        (event.request.method === "GET" || event.request.method === "HEAD")
    ) {
        event.respondWith(healthResponse());
        return;
    }
    // A failed worker must fail LOUD: navigations get an error page with
    // the actual reason and a retry; data-plane requests get a clean 503
    // carrying the error. An endless spinner is never acceptable. The
    // init-retry holdoff still applies — a later reload attempts a fresh
    // initialization, and success clears this state.
    if (workerHealth.state === "failed") {
        if (event.request.mode === "navigate" && !path.startsWith("/api/")) {
            // Kick a background re-initialization (the holdoff inside
            // activateWorker paces it) so "Try again" can actually
            // succeed once the cause has healed.
            activateWorker().catch(() => {});
            event.respondWith(failurePage());
            return;
        }
        if (path.startsWith("/api/")) {
            event.respondWith(new Response(
                JSON.stringify({ error: {
                    kind: "worker-failed",
                    message: workerHealth.error || "worker initialization failed",
                } }),
                { status: 503, headers: { "content-type": "application/json" } },
            ));
            return;
        }
    }
    // Shortcut-service routes (`PUT /@`, `GET /@/{hash}`) belong to
    // the edge worker, not this one. Not intercepting at all is the
    // point: the edge answers `GET /@/{hash}` with a relative 301
    // that the browser must follow natively so the short link's
    // `#fragment` (an invite's seed) carries over to the target via
    // RFC 7231 fragment inheritance. Serving the shell here would
    // swallow the redirect for any user who already has this worker
    // installed.
    if (path === "/@" || path.startsWith("/@/")) {
        return;
    }
    // Build metadata is mutable deployment information, not part of a
    // retained generation's application routing.
    if (path === "/version.json") return;
    // `/api/*` navigations remain real data-plane requests. All other
    // navigations use an exact stamped static document when one exists, or the
    // root SPA shell. Non-navigation assets use the immutable cache only on an
    // exact manifest match. Nested clients always reach Rust first so their
    // registered repository/branch binding can rewrite the request.
    if (retirementAttempt && path.startsWith("/api/")) {
        // Do not let the fetch that retries a failed release race ahead and
        // reopen a long-lived Rust stream. Once release succeeds, the shared
        // Rust latch makes the route return its finite handoff response.
        event.respondWith((async () => {
            const released = await retirementAttempt;
            if (!released) {
                return new Response(
                    JSON.stringify({
                        control: "update-pending",
                        reason: "the outgoing worker could not release its streams",
                    }),
                    {
                        status: 503,
                        headers: {
                            "content-type": "application/json",
                            "retry-after": "5",
                        },
                    },
                );
            }
            return routeFetch(event, path);
        })());
        return;
    }
    event.respondWith(routeFetch(event, path));
};

// Iframe-side bridge messages. The iframe sends `{v:1,type:"hello"}`
// at boot (with a transferred MessagePort) and then dispatches
// query/subscribe/evaluate envelopes over the port. The Rust
// worker's `onmessage` stashes the port against the client id and
// routes per-envelope dispatch from there.
//
// Synchronous early-outs handle lifecycle nudges before ordinary iframe
// messages reach Rust. A `{type:"claim"}` message asks the SW to take control
// of every client in scope. The page sends this on cold-start when it lands on
// a SW that was activated in a previous session — `onactivate` doesn't refire,
// so without this nudge the page would stay uncontrolled (and every /api/*
// fetch would land on the static-asset server as a 405). The claim raises
// `controllerchange` on the page side, which the shell's
// `serviceWorkerActivates()` Promise awaits.
self.onmessage = event => {
    if (event.data && event.data.type === "claim") {
        event.waitUntil?.(self.clients.claim());
        extendOfflineGeneration(event);
        return;
    }
    // An update-aware page observed that a fully-installed successor is
    // waiting. This reaches an incumbent that stayed alive long enough to miss
    // `updatefound` and therefore never ran the startup catch-up above. Verify
    // the registration state here before releasing any streams.
    if (event.data && event.data.type === "retire-if-superseded") {
        retireIfSuperseded(event);
        return;
    }
    // Connectivity nudge from the active page on an `online`/`offline`
    // transition. The worker re-reads `navigator.onLine` itself and reconciles;
    // this just wakes it so the overlay updates without waiting for a fetch.
    if (event.data && event.data.type === "connectivity") {
        extendOfflineGeneration(event);
        event.waitUntil?.(
            (async () => {
                try {
                    const worker = await activateWorker();
                    await worker.onconnectivity?.();
                } catch (err) {
                    log("connectivity dispatch failed:", err);
                }
            })(),
        );
        return;
    }
    // A page became visible again — wake the worker so sync resumes the
    // active cadence immediately instead of waiting out a hidden interval.
    if (event.data && event.data.type === "visibility") {
        event.waitUntil?.(
            (async () => {
                try {
                    const worker = await activateWorker();
                    await worker.onvisibility?.();
                } catch (err) {
                    log("visibility dispatch failed:", err);
                }
            })(),
        );
        return;
    }
    event.waitUntil?.(
        (async () => {
            try {
                const worker = await activateWorker();
                await worker.onmessage(event);
            } catch (err) {
                log("onmessage dispatch failed:", err);
            }
        })(),
    );
};

// Some engines report a structured-clone failure only to the receiver.
// Never inspect `event.data` here: it may be unavailable, and custody
// messages can contain transient PRF bytes.
self.onmessageerror = () => {
    console.error("messageerror: service-worker message could not be deserialized");
};

// Background Sync API event handler. A single bare `sync` tag: the worker
// drains the whole queue regardless, so the tag carries no identity. A
// rejected promise here tells the user agent to retry the sync with backoff.
self.onsync = event => {
    log("Background sync event:", event.tag);
    event.waitUntil(
        (async () => {
            let worker = await activateWorker();
            return worker.sync(event.tag);
        })()
    );
};
