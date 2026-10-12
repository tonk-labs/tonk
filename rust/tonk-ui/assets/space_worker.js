// The address a site origin's worker was registered at before its worker
// became the bootloader (`boot.js`), which loads a release.
//
// A browser that visited a site then still has a worker registered here, and
// asks this address for its update. This is that update: a worker that takes
// the site's pages from the one before, withdraws the registration, and so
// leaves the pages to load again with no worker. The shell they load then
// registers the bootloader.
self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", event => {
    event.waitUntil(
        // Taken first: a page's shell loads again when its worker changes,
        // and by then the registration is gone.
        self.clients.claim().then(() => self.registration.unregister()),
    );
});
