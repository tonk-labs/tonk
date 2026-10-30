// Carries ports between a kernel space's worker and the page around it.
//
// The space's worker puts this at the top of each page the kernel answers a
// page load with (see `kernel_boot.js`), so it runs before anything of the
// page's own. It says to the frame above that the site is here, hands down to
// the worker the port the frame above opens to the person's profile, and tells
// the worker when the page is about to go, so it saves what it holds first. On
// the profile's origin, it passes up what the profile's worker tells the page
// that asked it something, such as to run a passkey ceremony.
//
// The page's own code runs in this document too. The port is not for it:
// this script hears the port before anything else can, and keeps it from
// everything after. It takes its own script out of the page as it runs, so
// the page sees only what the kernel answered with.
(function () {
    "use strict";
    var sw = navigator.serviceWorker;
    var script = document.currentScript;
    if (script) script.remove();

    // Opened on its own, not framed by the app: there is no one to ask for a
    // port, and the app is where a space is shown.
    if (parent === window) return;

    // A frame inside the space's own page: the page around it carries the
    // port.
    try {
        if (parent.location.origin === location.origin) return;
    } catch (error) {}

    var tell = parent.postMessage.bind(parent);
    tell({ __tonkOrigin: "shell" }, "*");

    function ask() {
        tell({ __tonkOrigin: "need-port" }, "*");
    }
    function flush() {
        if (sw.controller) sw.controller.postMessage({ type: "flush" });
    }

    // The app's page runs the passkey ceremonies, and tells the profile's
    // worker what came of one by way of this frame: what a custody ceremony
    // derived, or that it was refused.
    var profile = location.hostname.split(".")[0].indexOf("profile") === 0;
    var TO_WORKER = ["custody", "ceremony-refused"];
    window.addEventListener(
        "message",
        function (e) {
            if (e.source !== parent || !e.data) return;
            if (e.data.__tonkOrigin === "port") {
                e.stopImmediatePropagation();
                var port = e.ports && e.ports[0];
                if (port && sw.controller) sw.controller.postMessage({ type: "port" }, [port]);
            } else if (profile && e.data.__tonkOrigin === "to-worker") {
                e.stopImmediatePropagation();
                var message = e.data.message;
                if (!message || TO_WORKER.indexOf(message.type) < 0 || !sw.controller) return;
                sw.controller.postMessage(message, Array.prototype.slice.call(e.ports || []));
            }
        },
        true,
    );
    // A worker the browser hears nothing from for a while it stops, and
    // the page's open answers with it, such as its subscriptions: an open
    // answer does not keep it up, an event does. The page around this one
    // keeps its own worker up, not this origin's.
    var KEEPALIVE_MS = 10000;
    setInterval(function () {
        if (sw.controller) sw.controller.postMessage({ type: "keepalive" });
    }, KEEPALIVE_MS);
    window.addEventListener("pagehide", flush);
    document.addEventListener("visibilitychange", function () {
        if (document.visibilityState === "hidden") flush();
    });
    // The worker asks for a port as it starts, and again after every restart;
    // it says what it is busy with while the page waits on it. Only the
    // profile's worker says anything else: a space's page holds author code.
    sw.addEventListener("message", function (e) {
        if (e.data && e.data.type === "need-port") ask();
        else if (e.data && e.data.type === "status") {
            tell({ __tonkOrigin: "status", stage: e.data.stage }, "*");
        } else if (profile) {
            tell({ __tonkOrigin: "worker-message", message: e.data }, "*");
        }
    });
    sw.startMessages();
    ask();
    // A page is ready as it loads, unless it says when it is itself.
    document.addEventListener("DOMContentLoaded", function () {
        if (!window.tonkSaysReady) tell({ __tonkOrigin: "status", stage: "ready" }, "*");
        offerUpdate();
    });

    // A site runs the kernel it pins until the person moves it. Where this
    // deployment ships another, say so, and move the site when asked: the
    // worker takes the new kernel up and records it, and the page goes back
    // through the site's shell, which registers the worker for it (a site's
    // own pages may register none) and returns to this address. Shown apart
    // from the page's own elements, which the page's code cannot reach.
    function offerUpdate() {
        Promise.all([pinned(), fetch("/_host/kernel.json", { cache: "no-cache" })])
            .then(function (both) {
                var running = both[0];
                var answer = both[1];
                if (!running || !answer.ok) return;
                return answer.json().then(function (shipped) {
                    var latest = shipped && shipped[profile ? "persona" : "kernel"];
                    if (typeof latest === "string" && latest !== running) showUpdate();
                });
            })
            .catch(function () {});
    }

    // The kernel the site pins: as its shell keeps it, or else as the worker
    // was registered for.
    function pinned() {
        var registered = sw.controller && new URL(sw.controller.scriptURL).searchParams.get("kernel");
        return caches
            .open("tonk-boot")
            .then(function (cache) { return cache.match("/__boot"); })
            .then(function (held) { return held ? held.json() : null; })
            .then(function (record) {
                return record && record.v === 2 && typeof record.kernel === "string" ? record.kernel : registered;
            })
            .catch(function () { return registered; });
    }

    function showUpdate() {
        var host = document.createElement("tonk-update");
        var root = host.attachShadow({ mode: "closed" });
        root.innerHTML =
            "<style>:host{all:initial;position:fixed;right:16px;bottom:16px;z-index:2147483647}" +
            "div{color-scheme:light dark;display:flex;gap:12px;align-items:center;padding:10px 12px;" +
            "font:13px/1.4 system-ui,sans-serif;background:Canvas;color:CanvasText;" +
            "border:1px solid GrayText;border-radius:8px;box-shadow:0 2px 12px rgb(0 0 0/.2)}" +
            "button{font:inherit;padding:4px 10px;border-radius:6px;border:1px solid GrayText;" +
            "background:ButtonFace;color:ButtonText;cursor:pointer}</style>" +
            '<div role="status"><span>A new version of Tonk is available.</span>' +
            '<button type="button">update</button></div>';
        var says = root.querySelector("span");
        var button = root.querySelector("button");
        var failed = function (error) {
            says.textContent = "The update did not finish: " + error;
            button.disabled = false;
        };
        button.addEventListener("click", function () {
            if (!sw.controller) return;
            button.disabled = true;
            says.textContent = "Updating\u2026";
            var channel = new MessageChannel();
            channel.port1.onmessage = function (e) {
                if (e.data && typeof e.data.step === "string") {
                    says.textContent = "Updating: " + e.data.step + "\u2026";
                    return;
                }
                if (!e.data || typeof e.data.kernel !== "string") return failed(e.data && e.data.error);
                var shell = profile ? "/profile.html" : "/space.html";
                var address = location.pathname + location.search + location.hash;
                location.replace(shell + "#boot=" + encodeURIComponent(address));
            };
            sw.controller.postMessage({ type: "update" }, [channel.port2]);
        });
        document.documentElement.appendChild(host);
    }
})();
