// The service worker of a site origin (`{label}.{host}`): the host's
// bootloader.
//
// A site runs the release it pins. This script is the worker the site's
// shell registers, and it is the same for every site: it loads one release
// and passes that release each of the worker's events. The release is a
// script of its own, `/_release/{release}/worker.js` (`release.js`, bundled
// by the publisher), with its wasm beside it. It adds no listener; it sets `self.release`, and its `start` returns
// the handlers a call goes to (see `plan/space-releases.md`).
//
// A service worker cannot load a script once it is running, but a classic
// one may name what it imports as it starts. So the release is named in this
// worker's own address, `/boot.js?pin={pin}`, and a site that pins another
// is a site with another worker. A site that names none runs the
// release this deployment was built with, and so follows the deployment.

// The release built alongside this script, as an address on this origin.
// Written in by `scripts/stamp-service-worker.sh`. It makes a new release a
// change to this script, which is what the browser compares when it looks
// for an update to a site that follows the deployment.
const CURRENT_RELEASE = "dev";

// Where this origin serves the release a pin names. A pin is the address of
// a release's `worker.js`, in the directory its content identifier names;
// the origin a release runs on serves it at `/_release/`, having checked it,
// and is told where it came from when that is elsewhere.
function served(pin) {
    if (!pin) return new URL(CURRENT_RELEASE, self.location.href);
    const named = new URL(pin, self.location.href);
    const release = /\/(b[a-z2-7]{58})\/worker\.js$/.exec(named.pathname)?.[1];
    if (!release) throw new Error(`boot: ${pin} does not name a release`);
    const here = new URL(`/_release/${release}/worker.js`, self.location.href);
    if (named.origin !== here.origin || named.pathname !== here.pathname) {
        here.searchParams.set("from", named.href);
    }
    return here;
}

const PINNED = new URL(self.location.href).searchParams.get("pin");
const RELEASE = served(PINNED);

importScripts(RELEASE.href);

const release = self.release;
if (typeof release?.start !== "function") {
    throw new Error(`boot: ${RELEASE.href} is not a release`);
}
// What the release answers each kind of call with. Asked for as the worker
// starts, so a call that arrives at once has somewhere to go. `host` is what
// a release may ask of the worker it runs in: for now, which release this
// worker was started for.
const handlers = release.start({
    base: RELEASE.href,
    host: { v: 1, pin: PINNED },
});

// What a handler is given in place of the worker's event: what the event
// carried, and the two things a handler may ask of it.
const call = (event, carried) => ({
    ...carried,
    waitUntil: promise => event.waitUntil(promise),
});

self.addEventListener("install", event => handlers.install(call(event, {})));
self.addEventListener("activate", event => handlers.activate(call(event, {})));
self.addEventListener("message", event =>
    handlers.message(call(event, { data: event.data, ports: event.ports, source: event.source })),
);
self.addEventListener("fetch", event =>
    handlers.fetch(
        call(event, {
            request: event.request,
            clientId: event.clientId,
            resultingClientId: event.resultingClientId,
            respondWith: response => event.respondWith(response),
        }),
    ),
);
self.addEventListener("unhandledrejection", event =>
    handlers.unhandledrejection({ reason: event.reason }),
);
