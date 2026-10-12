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

**The bootloader is the host's.** It is the script the shell registers. It
adds the worker's `install`, `activate`, `fetch` and `message` listeners,
loads the release the space pins, checks the wasm against its hash, and
hands each event to the release. It is small and it is the same for every
space.

**`worker.js` is the release's.** It is the glue generated for the release's
wasm, with whatever JavaScript that build needs around it, bundled into one
module. It names its wasm, by path beside itself and by hash, and exports
one function the bootloader calls:

```js
export const wasm = { path: "./worker_bg.wasm", sha256: "…" };
export function start({ wasm, host }) {
  return { install, activate, fetch, message };
}
```

That export is the whole of the `v: 1` contract between the two: the
bootloader gives the release its wasm and a `host` object, and the release
gives back the handlers events go to. `host` is where the host's stable API
lives, and where it grows.

A service worker cannot load JavaScript after it starts, so the bootloader
names the release in an `import` the server writes into it. The shell
registers `/boot.js?release=<release>`, and the server answers with the
bootloader importing `/_release/<release>/worker.js`. A change of pin is a
change of that address, which the browser treats as a new worker.

## What a release is named by

`<release>` is a content identifier of `worker.js`: a CIDv1 with the raw
codec and the SHA-256 of the file's bytes, written in base32, such as
`bafkreiglelbablybxkvvmqskikqor5sae6caidgrcwihc7bjdqinwq55j4`.

`worker.js` carries the SHA-256 of its wasm and of each other file of the
release, so that one identifier fixes all of them. SHA-256, because the
bootloader checks the wasm before any wasm runs, and it is the hash a browser
computes natively. A CID, because it says which hash it is, and the name
then outlives a change of hash function.

## The boot cell

A memory cell in the space, `boot/loader`, replicated with it. Its content is
a DAG-CBOR map:

```
{
  "v": 1,
  "src": "https://tonk.network/releases/bafkrei…55j4/worker.js",
  "latest": "https://tonk.network/releases/stable/worker.js",
  "release": 42(h'0001551220 cb22c200…db43bd4f')
}
```

- `v`, an unsigned integer: the host version above.
- `src`, a text string: where this release's `worker.js` is, at an address
  that never changes. A host that does not hold the release yet fetches it
  from there, however old it is.
- `release`, a link (CBOR tag 42): the pin, and what the fetched script is
  checked against.
- `latest`, a text string, optional: the line of releases the space watches.
  It is asked whether there is a newer release and never moves the pin. A
  space with none is not told of updates.

The keys are in DAG-CBOR's order (by length, then bytes), so the content of a
given pin is the same bytes on every device. The example above is 213 bytes:

```
a4 61 76 01
   63 73 72 63  78 63 "https://tonk.network/releases/bafkrei…55j4/worker.js"
   66 6c 61 74 65 73 74  78 2e "https://tonk.network/releases/stable/worker.js"
   67 72 65 6c 65 61 73 65  d8 2a 58 25 00 01 55 12 20 <32 bytes of SHA-256>
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

A line is one address that redirects to the release current on it. A
release's `worker.js` is served with its identifier as its `ETag`.

## Serving a release

A browser fetches a worker script from the network, from the origin it will
control. It cannot come from another origin or from the space's own storage.
So every origin of a deployment serves the store at a fixed path:

```
GET /_release/<release>/<file>
```

The access worker answers from the store and marks the response immutable.
The bootloader imports `/_release/<release>/worker.js`, and everything that
module asks for is beside it. Every release ever published stays there.

Where the host does not hold a release, it fetches it from the cell's
`src`, checks `worker.js` against `release`, and keeps it in its own store
before serving it. `src` may name any store: see [Decided](#decided).

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

To learn whether there is a newer one, ask `latest` with the pin as the
condition:

```
GET https://tonk.network/releases/stable/worker.js
If-None-Match: "bafkrei…55j4"
```

- `304 Not Modified`: the pin is the current release.
- `200`: the body is the current `worker.js`. Its SHA-256 gives the new
  release's identifier, and the address the redirect ended at is its `src`.
  Nothing about the newer release is taken on trust: its name is computed
  from what was fetched.

The host asks once for each distinct `latest` among the spaces it shows, when
the Hub opens and at most daily, and tells the owner of each space behind. A
space opened on its own asks for itself and records what it found where its
own views can show it.

Taking the update is the owner's act: the definitions of the new release are
committed, then the cell is published with the new `src` and `release`.
Members' devices follow when they pull.

A device that sees a new pin registers the bootloader for that release and
switches when idle. The session overlay is handed over as it is today, and
the page's subscription client reopens what it had, so nothing on screen
drops. Where the new worker does not come up, the device stays on the one it
has and says so.

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
- **`src` may name any store.** The host fetches over HTTPS, refuses a file
  over a size limit, checks `worker.js` against `release` and every other
  file against the hashes `worker.js` carries, and keeps its own copy before
  serving it.
