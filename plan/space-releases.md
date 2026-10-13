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
| Shell page | The document a new origin is answered with. It reads the space's pin and registers the bootloader for it. |
| Bootloader | The worker script. It loads the pinned release and hands it the worker's events. |
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

- `v: 1`: the host's bootloader imports the release's `worker.js`, which
  carries its glue, and hands it its wasm. The release uses the browser
  directly. This is what ships first.
- `v: 2`, later: the host runs a bootloader of its own that loads a wasm
  module by hash and gives it only what it imports, in the shape of Sapling's
  runtime world, so that one runtime serves both. The bootloader then holds
  the page's streams and the host port, and can replace a module without
  dropping a subscription.

A host serves every version it has ever served. A space on `v: 1` is not
moved to `v: 2` by anything but its own upgrade.

## The bootloader and the release

Two pieces of JavaScript run in a space's worker, and they have different
owners.

**The bootloader is the host's.** It is the script the shell registers, one
fixed file that is the same for every space. It adds the worker's `install`,
`activate`, `fetch` and `message` listeners, loads the release the space
pins, checks the wasm against its hash, and hands each event to the release.

**`worker.js` is the release's.** It is the glue generated for the release's
wasm, with whatever JavaScript that build needs around it, bundled into one
classic script. It adds no listener and takes hold of nothing in the worker.
It describes its release and exposes methods, and the bootloader calls them:

```js
self.release = {
  manifest: {
    v: 1,
    version: "0.7.0",
    port: 2,
    wasm: { path: "./worker_bg.wasm", sha256: "…" },
    files: { "guest/guest.js": "…", "library/core.yaml": "…" },
  },
  // Bring the release up. Called once for each start of the worker.
  async start({ wasm, host, handover }) {
    return {
      // A request from a page of this origin. Answer through
      // `host.respond`, or return false and it goes to the network.
      async fetch(request, { id, client }) {},
      // A message on a port, from a page or from the host around the space.
      async receive(port, data, ports) {},
      // A port closed, or a response's reader went away.
      async closed(id) {},
      // The release is about to be replaced. Return what the next one needs.
      async stop() {},
    };
  },
};
```

`manifest` holds whatever is to be known about the release: the host version
it needs, its own version, the port protocol it speaks, and the hash of
every other file.

### The host

`host` is the bootloader speaking for the deployment: the one object a
release is given, and the whole of what it may ask of the worker it runs
in. It is what stays the same when the deployment is upgraded.

```js
host = {
  v: 1,
  pin, src,                       // what this worker was started for
  // Ports. A release holds a port's id, never the port.
  ports: { parent, clients() },   // the host around the space; pages here
  send(port, data, ports),        // `ports` are ids too
  // Responses. A release holds a request's id, never the stream.
  respond(id, { status, headers }),
  write(id, chunk),
  end(id),
  // Keep the worker alive until this settles.
  keep(promise),
  // Say the space's pin has changed; the bootloader arranges the switch.
  repin(pin),
};
```

A release is handed ids, not the things themselves. The bootloader holds
every `MessagePort` and every open response, and a release names them by id:
it sends on a port id, and it answers a request by writing to the request's
id, which the bootloader pipes into the response it gave the page. So what is
open belongs to the bootloader and outlives the release that was using it.
`handover` carries the ids a release had in use, and its successor carries
on with them: for a subscription, by asking its query again and writing a
fresh snapshot to the same id.

How much survives a change of pin depends on whether the bootloader does:

- In `v: 1` a new release is JavaScript, which only a new worker can load,
  so the bootloader is replaced along with the release. Its ports can cross
  to its successor, since a worker may post to the next worker of its
  registration and ports transfer. Its open responses cannot: a browser does
  not activate the next worker while this one still has a response open, so
  they are ended and the page's client reopens them.
- In `v: 2` a release is wasm, which a running worker can load, so the
  bootloader stays and both ports and responses stay open across the switch.

The ids are the same contract in both. Nothing a release does changes when
the bootloader learns to stay.

The bootloader keeps for itself everything about being a service worker: the
listeners, `install` and `activate`, claiming pages, replacing itself, and
keeping the worker alive while a call is answered. It fetches and checks the
release's files on install. It answers what is the host's own, the shell
page and `/_release/`, and passes every other request to `fetch`. It holds
the ports and the open responses. What a release returns from `stop` it
hands to the next one as `handover`.

A release in `v: 1` still uses the browser directly for storage, the network
and keys. It does not use the worker's own globals (`clients`,
`registration`, `skipWaiting`, the events); those are the bootloader's, and
`host` is how a release reaches them.

The methods above and `host` are the whole of the `v: 1` contract. Either
side may gain a method. Neither loses one or changes what one means.

A service worker cannot load JavaScript once it is running, but a classic one
may name what it imports as it starts. The shell registers the bootloader
with the pin in its address, `/boot.js?pin=<pin>`, and the bootloader reads
it from its own address and imports that release:

```js
const pin = new URL(self.location).searchParams.get("pin");
importScripts(mirror(pin));
```

A change of pin is a change of the bootloader's address, which the browser
treats as a new worker. A script loaded this way cannot ask what address it
came from, and does not need to: the bootloader knows the pin it imported.

## What a release is named by

A release is named by a content identifier of its `worker.js`: a CIDv1 with
the raw codec and the SHA-256 of the file's bytes, in base32, such as
`bafkreiglelbablybxkvvmqskikqor5sae6caidgrcwihc7bjdqinwq55j4`. A store keeps
a release in a directory of that name.

`worker.js` carries the SHA-256 of its wasm and of each other file of the
release, so that one identifier fixes all of them. SHA-256, because the
bootloader checks the wasm before any wasm runs, and it is the hash a browser
computes natively. A CID, because it says which hash it is.

## The boot cell

A memory cell in the space, `boot/loader`, replicated with it. Its content is
a DAG-CBOR map:

```
{
  "v": 1,
  "pin": "https://tonk.network/releases/bafkrei…55j4/worker.js",
  "src": "https://tonk.network/releases/stable/worker.js"
}
```

- `v`, an unsigned integer: the host version above.
- `src`, a text string: the line of releases the space took its release
  from. It redirects to whichever release is current on it.
- `pin`, a text string: what `src` redirected to when the space last took a
  release. It is the address of one release's `worker.js`, it never changes,
  and it is what the space runs.

A pin's address ends in `<release>/worker.js`, the directory being the
identifier above. That is how what is fetched from a pin is checked: the
file's hash has to be the one its address names.

The keys are in DAG-CBOR's order (by length, then bytes), so the content of a
given pin is the same bytes on every device:

```
a3 61 76 01
   63 "pin"  78 63 "https://tonk.network/releases/bafkrei…55j4/worker.js"
   63 "src"  78 2e "https://tonk.network/releases/stable/worker.js"
```

A worker cannot read its own boot cell before it exists, so the pin is also
kept where the shell page can read it without Dialog: a boot record in the
origin's cache storage, which the running worker rewrites whenever the cell
changes. On a device that holds nothing yet, the pin arrives with the
delegation from the host: the delegation, the space, its remote, and the
boot cell's content.

The cell is not part of a branch, so moving the pin and installing the
library definitions that go with a release are two writes. The definitions
go first: a release's definitions only add to the ones before, so the worker
still pinned runs with them until the cell moves.

## Where releases live

One store for dev, staging and stable, and a second for pull
request previews that is emptied when the pull request closes.

```
releases/
  <release>/worker.js
  <release>/worker_bg.wasm
  <release>/guest/…
  <release>/library/…
  stable/worker.js     redirects to the current <release>/worker.js
  staging/worker.js
  dev/worker.js
```

A release is named by its content, so the lines cannot collide, and a release
promoted from one line to the next is not uploaded twice. A space pinned on
one deployment can turn up on another (signing in through another deployment
already exists), and with one store any deployment can serve it. A release
is under 20 MB. Releases that were only on the dev line expire: see
[Decided](#decided).

A line is one address that redirects to the release current on it.

## Serving a release

Every origin of a deployment serves releases at a fixed path of its own:

```
GET /_release/<release>/<file>?from=<pin>
```

This is the `mirror(pin)` the bootloader imports. The access worker answers
from its store. Where it does not hold the release, it fetches it from the
pin, checks `worker.js` against the identifier in the pin's address, and
keeps its own copy before answering. So a release is checked before a
browser runs it, whatever store it came from, and one a deployment has
served once it can serve again, however old it is. Responses are immutable.

The underscore keeps the path out of the way of a space's own routes.

## Publishing

1. Build. Bundle the glue and the release's own JavaScript into `worker.js`,
   with the hashes of its wasm and other files written into it. Its hash is
   the release.
2. Upload `<release>/` (nothing to do if it is there).
3. Deploy the host.
4. Point the line at the release. Pointing it back is a rollback.

Local development and the browser tests serve the same paths out of `dist`.
The tests' two generations become two releases.

## Finding and taking an update

A line of releases is one address, such as
`https://tonk.network/releases/stable/worker.js`, that redirects to the
`worker.js` of the release current on it.

To learn whether there is a newer one, ask `src` and see where it leads:

```js
const response = await fetch(src, { method: "HEAD" });
const current = response.url; // where the redirect ended
```

Where `current` is not the space's `pin`, the line has moved, and `current`
is the pin the space would take.

The host asks once for each distinct `src` among the spaces it shows, when
the Hub opens and at most daily, and tells the owner of each space behind. A
space opened on its own asks for itself and records what it found where its
own views can show it.

Taking the update is the owner's act: the definitions of the new release are
committed, then the cell is published with the new `pin`. Members' devices
follow when they pull.

A device that sees a new pin registers the bootloader for it and switches
when idle. The session overlay is handed over as it is today, and the page's
subscription client reopens what it had, so nothing on screen drops. Where
the new worker does not come up, the device stays on the one it has and says
so.

## Keeping the promise

- The port messages and the service endpoints a worker uses are written down
  in the repository. That file is the contract.
- A space says which port version it speaks when it connects, and the host
  answers within it.
- A few old releases are kept as fixtures. The browser suite runs with each
  against the current host, and a host change that breaks one fails the build.

## A space that exists already

It has no boot cell. The host treats it as pinned to the release current
when this ships and writes that cell the next time its owner opens it.

## Decided

- **The host does not refuse a release.** Where one is known to have a hole,
  the host tells the owner and offers the upgrade. Refusing to run it stays
  possible as a last resort and is not built first.
- **Dev releases expire.** Releases on the stable and staging lines are kept
  for good. One that was only ever on the dev line is dropped some time
  after the line moved past it (90 days, to start with). A dev space still
  pinned to a dropped release runs wherever a device has it cached; where
  none does, the host offers its owner the release current on the line.
- **A pin may name any store.** The host fetches over HTTPS, refuses a file
  over a size limit, checks `worker.js` against the identifier in the pin's
  address and every other file against the hashes `worker.js` carries, and
  keeps its own copy before serving it.

## What is built

On `feat/space-boot`:

- **The bootloader** (`assets/boot.js`). The script a site's shell
  registers. It holds the worker's events, loads one release with
  `importScripts`, and passes each event's call to the handlers that release
  gives it. Its address may carry `?pin=`; with none it runs the release the
  deployment was built with, which the publisher stamps into it.
- **The release** (`assets/release.js`, bundled). The publisher bundles it
  with the wasm's bindings into `/_release/<release>/worker.js`, with the
  wasm beside it, and names it by the CID of that file. It adds no listener;
  it sets `self.release`, whose `start` returns its handlers.
- **Pinning.** A space's shell reads a boot record (cache `tonk-boot`, key
  `/__boot`) and registers the bootloader for the pin in it, withdrawing a
  worker registered for another release. The bootloader takes a pin from any
  store and loads `/_release/<release>/worker.js?from=<pin>` on its own
  origin.
- **Keeping releases.** With `TONK_RELEASE_STORE` set, the publisher keeps
  the newest few releases there and serves them all from the dist. The dev
  server keeps five in `rust/tonk-ui/.releases`.
- **The address before.** A worker still registered at `/space_worker.js`
  is retired by the script now served there.

A browser test publishes two generations and shows a space pinned to the
first staying on it when the deployment moves to the second, while the
profile follows the deployment.

## Not built

- **The boot cell.** Nothing writes the boot record but a test. The cell in
  the space, the worker copying it into the record, and the pin arriving with
  a delegation are next.
- **Whether a new space pins.** A space with no pin follows its deployment,
  which is what local development needs. Pinning at creation is to be a
  setting of the deployment.
- **A store on a deployment.** `/_release/` is served from the build's own
  files. Keeping releases in a bucket, fetching one from its `src` and
  checking it, and the lines of releases are not built, nor is the
  publishing workflow.
- **Finding and taking an update.**
- **The contract as written above.** A handler is given a call carrying the
  request or message, with `respondWith` and `waitUntil`, not ids. The
  release still loads its own wasm, serves the shell, hands over to its
  successor, and uses the worker's globals (`clients`, `registration`,
  `skipWaiting`). Each of those moves to the bootloader in turn.
- **The profile's and the app's workers.** The profile's runs a release
  through the bootloader and always follows its deployment. The app's own
  worker (`service_worker.js`) is unchanged.

