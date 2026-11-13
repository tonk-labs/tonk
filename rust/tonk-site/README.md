# tonk-site

The `<tonk-site>` custom element and the bridge between it and the guest it frames.

`<tonk-site with="…" allow="…" path="…">` frames a site on an origin of its own and tells it which path to show. It is the recursive unit of the UI: the top page mounts one for the profile, the profile's chrome mounts a nested one for a space, and each owns one isolation boundary. `with` (the site, `branch@repo`) and `allow` (the reach its guest may ask for: `*`, `self`, or explicit locations) are both required; a site missing or malforming either renders a visible error at connect.

This crate compiles to wasm and registers the element via [`register_site`](src/lib.rs).

## The bridge

A site's shell runs a small bootstrap script ([`bootstrap.js`](src/bootstrap.js)) before anything else. It defines `window.tonk` synchronously, opens a `MessageChannel`, and posts a `hello` to its parent transferring one port. The parent authenticates the handshake by `event.source` identity (matching the message against a registered iframe's live `contentWindow`), never by `event.origin`.

Author code in the guest sees one injected object:

```text
window.tonk = {
  context: { this, model },
  preview(request)   -> Promise<value>,
  delegate(request)  -> Promise<delegation>,
  navigate(href)     -> void,
  setTitle(text)     -> void,
  open(href)         -> void,
  analytics(event)   -> void,
  register(reason, cb) -> void,
  task(payload, cb)  -> void,
  fetch(path, req)   -> Promise<Response>,
  ready: Promise<void>,
}
```

The parent is a port relay: after the handshake it binds the transferred port and posts `ready { context }` back, then answers each inbound envelope on the trusted page. A guest's fetch of a location outside the site's `allow` is denied with a typed error.

The shell then runs [`runtime_bootstrap.js`](src/runtime_bootstrap.js), which brings up the element runtime: from the site's own origin, or, in a sealed guest, by asking the parent for it (`runtime-ready`). Editors and other components are not part of it: a space installs them as library components.

## Modules

- [`site`](src/site.rs): the `<tonk-site>` custom element: attribute parsing, frame lifecycle, and navigation.
- [`bridge`](src/bridge.rs): the page-level `hello` listener and registry, port binding, runtime injection, and the envelope dispatcher.
- [`space_origin`](src/space_origin.rs): the per-site origins sites render on, and the ports brokered between them.
- [`task`](src/task.rs): the typed task requests a guest hands the trusted page.
