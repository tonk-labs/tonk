// Real CLI + ordinary /network form: no injected carrier or probe endpoints.
// cargo build -p tonk-cli --features rtc
// (cd rust/tonk-ui && trunk build)
// node rust/tonk-ui/tests/e2e/cli-status-app.mjs
// Optional: BROWSER=firefox|webkit, CHROME=/path, DIST=/path,
// PLAYWRIGHT_MODULE=/path/to/playwright/index.mjs, TONK_BIN=/path/to/tonk,
// RTC_PRIVATE=1 (per-fixture private DTLS certificate).
// RTC_EMPTY=1 (empty installation: no space, selection, or account).
// RTC_OFFLINE=1 (HTTP blocked and app origin stopped after install; UDP remains).
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { mkdtemp, mkdir, readFile, stat, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";
import path from "node:path";

const engines = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../../..");
const dist = path.resolve(process.env.DIST ?? path.join(repo, "rust/tonk-ui/dist"));
const binary = process.env.TONK_BIN ?? path.join(repo, "target/debug/tonk");
const engine = process.env.BROWSER ?? "chromium";
const temporary = await mkdtemp(path.join(tmpdir(), "tonk-rtc-product-"));
const env = { ...process.env,
    TONK_PROFILE_DIRECTORY: path.join(temporary, "profile"),
    TONK_SPACES_STATE: path.join(temporary, "state"),
    TONK_TELEMETRY_STATE: path.join(temporary, "telemetry"),
    TONK_UPDATE_STATE: path.join(temporary, "updates"),
    TONK_TELEMETRY: "0", TONK_NO_UPDATE_CHECK: "1",
    // Isolated device-root fixture; not evidence of account-authorized sync.
    TONK_UNSAFE_ALLOW_DEVICE_ROOT: "1",
};
delete env.TONK_SPACE;
delete env.TONK_RTC_PRIVATE_IDENTITY;
if (process.env.RTC_EMPTY === "1") delete env.TONK_UNSAFE_ALLOW_DEVICE_ROOT;
if (process.env.RTC_PRIVATE === "1") env.TONK_RTC_PRIVATE_IDENTITY = "1";
await mkdir(env.TONK_PROFILE_DIRECTORY);

function command(args, environment = env) {
    return new Promise((resolve, reject) => {
        const child = spawn(binary, args, { cwd: temporary, env: environment, stdio: ["ignore", "pipe", "pipe"] });
        let output = "";
        child.stdout.on("data", (chunk) => { output += chunk; });
        child.stderr.on("data", (chunk) => { output += chunk; });
        child.once("error", reject);
        child.once("exit", (code) => code === 0 ? resolve(output) : reject(new Error(`tonk ${args.join(" ")}: ${output}`)));
    });
}

function listen(port = 0, environment = env) {
    const child = spawn(binary, ["rtc", "serve", "--port", String(port)], {
        cwd: temporary, env: environment, stdio: ["ignore", "pipe", "pipe"],
    });
    let output = "";
    const ready = new Promise((resolve, reject) => {
        const timer = setTimeout(() => reject(new Error(`CLI startup timed out: ${output}`)), 30000);
        const read = (chunk) => {
            output += chunk;
            const peer = /^PEER (.+)$/m.exec(output)?.[1];
            const bound = /listening on port (\d+)/.exec(output)?.[1];
            if (peer && bound) { clearTimeout(timer); resolve({ peer, port: Number(bound) }); }
        };
        child.stdout.on("data", read);
        child.stderr.on("data", read);
        child.once("error", (error) => { clearTimeout(timer); reject(error); });
        child.once("exit", (code) => { clearTimeout(timer); reject(new Error(`CLI exited ${code}: ${output}`)); });
    });
    const stop = () => new Promise((resolve) => {
        if (child.exitCode !== null || child.signalCode !== null) { resolve(); return; }
        const timer = setTimeout(() => child.kill("SIGTERM"), 5000);
        child.once("exit", () => { clearTimeout(timer); resolve(); });
        child.kill("SIGINT");
    });
    return { ready, stop };
}

const types = { ".html": "text/html", ".js": "text/javascript", ".mjs": "text/javascript",
    ".wasm": "application/wasm", ".json": "application/json", ".css": "text/css", ".svg": "image/svg+xml",
    ".png": "image/png", ".der": "application/octet-stream", ".yaml": "text/yaml", ".woff2": "font/woff2" };
let assetRequests = 0;
const server = createServer(async (request, response) => {
    assetRequests += 1;
    const url = new URL(request.url, "http://127.0.0.1");
    const relative = ["/", "/network"].includes(url.pathname) ? "/index.html" : url.pathname;
    const file = path.resolve(dist, `.${relative}`);
    if (!file.startsWith(`${dist}${path.sep}`)) { response.writeHead(403).end(); return; }
    try {
        if (!(await stat(file)).isFile()) throw new Error("not a file");
        response.writeHead(200, { "content-type": types[path.extname(file)] ?? "application/octet-stream",
            "cache-control": "no-store", "service-worker-allowed": "/" }).end(await readFile(file));
    } catch { response.writeHead(404).end(); }
});

let browser, context, listener, otherListener;
try {
    if (process.env.RTC_EMPTY !== "1") await command(["space", "new", "rtc-fixture"]);
    listener = listen();
    const first = await listener.ready;
    let other;
    if (process.env.RTC_EMPTY !== "1") {
        const alternate = { ...env, TONK_PROFILE_DIRECTORY: path.join(temporary, "other-profile"),
            TONK_SPACES_STATE: path.join(temporary, "other-state") };
        await mkdir(alternate.TONK_PROFILE_DIRECTORY);
        await command(["space", "new", "other-cli-fixture"], alternate);
        otherListener = listen(0, alternate);
        other = await otherListener.ready;
        assert.notEqual(other.peer.split("?")[0], first.peer.split("?")[0], "separate profiles must have separate peer identities");
        assert.notEqual(other.port, first.port);
    }
    await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
    // A normal persisted installation, not a private-browsing context. Besides
    // making later offline-restart coverage possible, this exercises WebKit's
    // normal OPFS policy rather than its ephemeral-context restrictions.
    context = await engines[engine].launchPersistentContext(path.join(temporary, "browser"),
        process.env.CHROME ? { executablePath: process.env.CHROME } : {});
    browser = context.browser();
    let page = await context.newPage();
    page.on("pageerror", (error) => console.error(`page: ${error.message}`));
    if (process.env.DEBUG) page.on("console", (message) => {
        if (!message.text().includes("/.well-known/trunk/ws")) console.log(`page: ${message.text()}`);
    });
    await page.goto(`http://127.0.0.1:${server.address().port}/network`);
    await page.waitForFunction(() => navigator.serviceWorker.controller !== null, null, { timeout: 120000 });
    async function form() {
        for (let tries = 0; tries < 300; tries += 1) {
            for (const frame of page.frames()) {
                const input = frame.locator(".nask-peer:visible").first();
                if (await input.isVisible()) return { frame, input };
            }
            await new Promise((resolve) => setTimeout(resolve, 100));
        }
        const visible = await Promise.all(page.frames().map(async (frame) => ({
            url: frame.url(),
            body: (await frame.locator("body").innerText({ timeout: 1000 }).catch(() => "unavailable")).slice(0, 2000),
        })));
        const health = await page.evaluate(async () => {
            const response = await fetch("/api/health", { signal: AbortSignal.timeout(2000) });
            const health = await response.json();
            return { build: health.build, worker: health.worker, error: health.error,
                log: health.log?.filter(entry => ["warn", "error"].includes(entry.level)).slice(-8) };
        }).catch(error => ({ error: error.message }));
        throw new Error(`no network connect form rendered at ${page.url()}\n${JSON.stringify({ frames: visible, health })}`);
    }
    async function connect(uri, status) {
        const { frame, input } = await form();
        await input.fill(uri);
        await input.press("Enter");
        try {
            if (status === "peer:reachable") {
                // A prior peer may still be rendered while the command starts.
                // Wait for this identity, not just any old successful status.
                await frame.locator(".nrow__subject").filter({ hasText: uri.split("?")[0] })
                    .first().waitFor({ state: "visible", timeout: 40000 });
            }
            await frame.locator(`[data-status="${status}"]`).waitFor({ state: "visible", timeout: 40000 });
        } catch (error) {
            throw new Error(`${error.message}\nNetwork page: ${await frame.locator("body").innerText()}`);
        }
        return frame;
    }
    const connected = await connect(first.peer, "peer:reachable");
    assert.ok((await connected.locator(".nrow__subject").first().textContent()).includes(first.peer.split("?")[0]));
    if (process.env.RTC_EMPTY === "1") {
        assert.equal(await connected.locator('[data-peer-offer]').count(), 0);
    } else {
        await connected.locator('[data-peer-offer]').filter({ hasText: "rtc-fixture" }).waitFor();
        const offer = connected.locator('[data-peer-offer]').filter({ hasText: "rtc-fixture" });
        await offer.getByRole("button", { name: "sync through this peer" }).click();
        await offer.getByRole("status").filter({ hasText: "access needed" }).waitFor();
    }
    if (process.env.DEBUG) console.log("connected to the real CLI and read its inventory");
    if (other) {
        const second = await connect(other.peer, "peer:reachable");
        assert.ok((await second.locator(".nrow__subject").first().textContent()).includes(other.peer.split("?")[0]));
        await second.locator('[data-peer-offer]').filter({ hasText: "other-cli-fixture" }).waitFor();
        const selected = await connect(first.peer, "peer:reachable");
        await selected.locator('[data-peer-offer]').filter({ hasText: "rtc-fixture" }).waitFor();
        assert.equal(await selected.locator('[data-peer-offer]').filter({ hasText: "other-cli-fixture" }).count(), 0);
    }
    if (process.env.RTC_OFFLINE === "1") {
        // First-install activation pins only the worker runtime. The complete
        // offline resource graph is adopted later, after foreground startup.
        // Match the immutable document build, not merely an active controller.
        const build = await page.evaluate(() => globalThis.tonkBuild);
        assert.match(build, /^[0-9a-f]{16}$/);
        const installDeadline = Date.now() + 180000;
        // Poll from Node: waitForFunction treats an async predicate's Promise
        // as truthy before the CacheStorage read has actually finished.
        while (!(await page.evaluate(async (build) => {
            const response = await caches.match(`/.tonk-generation-${build}`, {
                cacheName: `TONK_GENERATION_${build}`,
            });
            if (!response) return false;
            const marker = await response.json();
            return marker.build === build && marker.state === "adopted";
        }, build))) {
            if (Date.now() > installDeadline) throw new Error("complete offline generation was not installed within three minutes");
            await new Promise(resolve => setTimeout(resolve, 1000));
        }
        const cached = await page.evaluate(async (build) => Promise.all(
            ["/rtc-carrier.mjs", "/rtc.mjs", "/rendezvous.der", "/worker_bg.wasm"].map(async (url) => {
                const cacheName = url.endsWith(".wasm") ? `TONK_WORKER_${build}` : `TONK_SHELL_${build}`;
                return { url, present: !!(await caches.match(new URL(url, location.origin).href, { cacheName })) };
            }),
        ), build);
        if (!cached.every(({ present }) => present)) {
            const inventory = await page.evaluate(async () => Promise.all((await caches.keys()).map(async name => ({
                name, urls: (await (await caches.open(name)).keys()).slice(0, 4).map(request => request.url),
            }))));
            throw new Error(`missing adopted RTC assets for ${build}: ${JSON.stringify({ cached, inventory })}`);
        }
        // No origin fallback and no browser HTTP network. setOffline() also
        // affects loopback ICE in Chromium and offline navigation in WebKit,
        // which is not the "internet absent, loopback available" contract.
        // This does not simulate navigator.onLine=false, stop the OS network,
        // or prove delegated repository authorization survives offline.
        const appOrigin = new URL(page.url()).origin;
        await context.route("**/*", route => {
            const url = new URL(route.request().url());
            // WebKit also routes the sealed guest's local blob: module
            // imports. Those are transferred cached bytes, not internet.
            const externalHttp = ["http:", "https:"].includes(url.protocol) && url.origin !== appOrigin;
            return externalHttp ? route.abort("internetdisconnected") : route.continue();
        });
        server.close();
        server.closeAllConnections();
        const requestsBeforeReload = assetRequests;
        await page.reload();
        await page.waitForFunction(() => navigator.serviceWorker.controller !== null);
        assert.equal(assetRequests, requestsBeforeReload, "offline reload reached the asset server");
        assert.equal(await page.evaluate(() => globalThis.tonkBuild), build, "reload did not boot the cached app generation");
        await connect(first.peer, "peer:reachable");
    }
    const invalid = await connect("not a peer URI", "peer:invalid");
    assert.equal(await invalid.locator('[data-peer-offer]').count(), 0, "failed attempts retain no stale inventory");
    // Correct carrier, wrong authenticated endpoint: DTLS reachability must
    // not turn a request for a different iroh identity into a successful probe.
    const wrongKey = "did:key:z6MkrZ1r5XBFZjBU34qyD8fueMbMRkKw17BZaq2ivKFjnz2z";
    assert.notEqual(first.peer.split("?")[0], wrongKey);
    await connect(wrongKey + first.peer.slice(first.peer.indexOf("?")), "peer:unreachable");
    await connect(first.peer, "peer:reachable");
    await listener.stop();
    const { frame } = await form();
    try {
        await frame.locator('[data-status="peer:unreachable"]').waitFor({ state: "visible", timeout: 25000 });
    } catch (error) {
        const health = await page.evaluate(async () => {
            const response = await fetch("/api/health", { signal: AbortSignal.timeout(3000) });
            const health = await response.json();
            return { build: health.build, error: health.error,
                log: health.log?.filter(entry => ["warn", "error"].includes(entry.level)).slice(-5) };
        }).catch(error => ({ error: error.message }));
        throw new Error(`${error.message}\nAfter CLI stop: ${await frame.locator("body").innerText()}\n${JSON.stringify(health)}`);
    }
    listener = listen(first.port);
    const restarted = await listener.ready;
    assert.equal(restarted.peer, first.peer, "restart changed the saved peer identity or route");
    await connect(restarted.peer, "peer:reachable");
    const replacement = await context.newPage();
    await replacement.goto(page.url());
    await replacement.waitForFunction(() => navigator.serviceWorker.controller !== null);
    await page.close();
    page = replacement;
    const moved = await form();
    await moved.frame.locator('[data-status="peer:unreachable"]').waitFor({ state: "visible", timeout: 25000 });
    await connect(restarted.peer, "peer:reachable");
    const version = browser?.version() ?? await page.evaluate(() => navigator.userAgent);
    console.log(`PASS ${engine} ${version} (private certificate: ${process.env.RTC_PRIVATE === "1"}, empty registry: ${process.env.RTC_EMPTY === "1"}, two CLI profiles: ${!!other}, offline reload: ${process.env.RTC_OFFLINE === "1"}): real CLI /network connect, inventory, invalid input, wrong endpoint key, retry, disconnect, restart, carrier-tab close and retry`);
} finally {
    await context?.close();
    await listener?.stop();
    await otherListener?.stop();
    server.close();
    // Only this test's mkdtemp-owned fixture, never an installation directory.
    await rm(temporary, { recursive: true, force: true });
}
