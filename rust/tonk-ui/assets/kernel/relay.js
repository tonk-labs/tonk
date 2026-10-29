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

    window.addEventListener(
        "message",
        function (e) {
            if (e.source !== parent || !e.data || e.data.__tonkOrigin !== "port") return;
            e.stopImmediatePropagation();
            var port = e.ports && e.ports[0];
            if (port && sw.controller) sw.controller.postMessage({ type: "port" }, [port]);
        },
        true,
    );
    window.addEventListener("pagehide", flush);
    document.addEventListener("visibilitychange", function () {
        if (document.visibilityState === "hidden") flush();
    });
    // The worker asks for a port as it starts, and again after every restart;
    // it says what it is busy with while the page waits on it. Only the
    // profile's worker says anything else: a space's page holds author code.
    var profile = location.hostname.split(".")[0].indexOf("profile") === 0;
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
    });
})();
