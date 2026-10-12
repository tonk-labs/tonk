# Releases a space pins

A space runs the worker it chose. It names a release in a cell, the host
loads exactly that, and nothing the host ships later changes what an existing
space runs. The space moves to another release when its owner says so.

This builds on `plan/space-workers.md`: a space on its own origin, with its
own worker, reached by the host over a message port.

## The line between host and application

The application is everything that changes with a build: the worker's wasm,
the `wasm-bindgen` glue generated for that wasm, the worker script that
uses the glue, the page runtime, and the library of definitions. A space
pins all of it as one release.

The host is what a space can rely on without pinning it:

| Surface | What it is |
|---|---|
| Release files | A release's files by hash, same-origin, immutable, kept for good. |
| Shell page | The document a new origin is answered with. It reads the space's pin and registers that release's worker script. |
| Port protocol | The messages between a space and the host around it. |
| Service HTTP | The access service and storage endpoints a worker calls to sync. |
| The browser | IndexedDB, fetch, WebCrypto, which the pinned worker uses directly. |

The host may add to these. It never removes from them or changes what one
means, so a worker pinned long ago keeps working after any host upgrade.

The glue belongs to the application because it cannot belong to the host. It
is generated per build, and a wasm's import list (303 functions in a current
build) is not an interface anyone can keep stable.

## Host versions

A release says which host it needs.

- `v: 1`: the host registers the release's own worker script. That script
  carries its glue, loads its wasm, and uses the browser directly. This is
  what ships first.
- `v: 2`, later: the host runs a bootloader of its own that loads a wasm
  module by hash and gives it only what it imports, in the shape of Sapling's
  runtime world, so that one runtime serves both. The bootloader then holds
  the page's streams and the host port, and can replace a module without
  dropping a subscription.

A host serves every version it has ever served. A space on `v: 1` is not
moved to `v: 2` by anything but its own upgrade.

## A release is one script

A release is entered through one file, `worker.js`: the worker script and the
glue generated for its wasm, bundled together at build time. The hash of that
file is the release's edition.

The script names the rest of its release itself, relative to its own address
and by hash: its wasm, the page runtime, the library. It checks what it
fetches against those hashes, as today's worker script checks its wasm. So
one hash pins the whole release, and loading it takes no step of its own:
the browser fetches the script when it is registered, and the script fetches
its wasm.

## The boot cell

A cell in the space, `boot/loader`, replicated with it:

```yaml
v: 1
src: https://tonk.network/releases/stable/worker.js
edition: <sha256 of worker.js>
```

- `v` is the host version above.
- `src` is the address the release was taken from: a line of releases, such
  as the stable one. It is where to ask whether there is a newer one.
- `edition` is the pin.

A worker cannot read its own boot cell before it exists, so the pin is also
kept where the shell page can read it without Dialog: a boot record in the
origin's cache storage, which the running worker rewrites whenever the cell
changes. On a device that holds nothing yet, the pin arrives with the
delegation from the host: the delegation, the space, its remote, and the
boot cell's content.

## Where releases live

One append-only store for dev, staging and stable, and a second for pull
request previews that is emptied when the pull request closes.

```
releases/
  <edition>/worker.js
  <edition>/worker_bg.wasm
  <edition>/guest/…
  <edition>/library/…
  stable/worker.js     redirects to the current <edition>/worker.js
  staging/worker.js
  dev/worker.js
```

An edition is a content hash, so the lines cannot collide, and a release
promoted from one line to the next is not uploaded twice. A space pinned on
one deployment can turn up on another (signing in through another deployment
already exists), and with one store any deployment can serve it. A release
is under 20 MB.

A line is one address that redirects to the edition current on it. The
edition's `worker.js` is served with its hash as its `ETag`.

## Serving a release

A browser fetches a worker script from the network, from the origin it will
control. It cannot come from another origin or from the space's own storage.
So every origin of a deployment serves the store at a fixed path:

```
GET /_release/<edition>/<file>
```

The access worker answers from the store, marks the response immutable, and
sends `Service-Worker-Allowed: /` with `worker.js` so that it may control the
whole origin. The shell registers `/_release/<edition>/worker.js`, and
everything the script asks for is beside it.

`src` in the cell may name another deployment's store. The host then fetches
the edition from there, checks `worker.js` against `edition`, and keeps it in
its own store before serving it. Only the stores of our own deployments are
accepted at first.

## Publishing

1. Build. Bundle the worker script with its glue, with the hashes of its
   wasm and other files written into it. Its hash is the edition.
2. Upload `<edition>/` (nothing to do if it is there).
3. Deploy the host.
4. Point the line at the edition. Pointing it back is a rollback.

Local development and the browser tests serve the same paths out of `dist`.
The tests' two generations become two editions.

## Finding and taking an update

- Asking `src` with `If-None-Match: "<edition>"` answers either that nothing
  changed or with the script current on that line, whose `ETag` is the newer
  edition. The host asks for the spaces it shows and tells the owner. A space
  opened on its own asks for itself, at most daily, and records what it found
  where its own views can show it.
- Taking the update is one commit on the space by its owner: the cell's
  `edition` moves, and the library definitions that match are installed with
  it. Members' devices follow when they pull.
- A device that sees a new pin registers that edition's script and switches
  when idle. The session overlay is handed over as it is today, and the
  page's subscription client reopens what it had, so nothing on screen drops.
  Where the new worker does not come up, the device stays on the one it has
  and says so.

## Keeping the promise

- The port messages and the service endpoints a worker uses are written down
  in the repository. That file is the contract.
- A space says which port version it speaks when it connects, and the host
  answers within it.
- A few old editions are kept as fixtures. The browser suite runs with each
  against the current host, and a host change that breaks one fails the build.

## A space that exists already

It has no boot cell. The host treats it as pinned to the edition current
when this ships and writes that cell the next time its owner opens it.

## Open

- Whether the host may refuse an edition with a known hole. It would not move
  the pin, but the space would not run until its owner upgrades.
- Whether dev editions are kept for good. Expiry breaks a dev space that never
  upgraded.
- Whether `src` may name a store that is not ours, and what the host checks
  before it serves one.
