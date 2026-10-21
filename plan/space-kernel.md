# A space runs the kernel it holds

A space is a blank slate with a kernel in it. Creating a space installs one
thing, the kernel, and the kernel serves only what the space declares. A space
nothing has been installed into declares no routes, so every page it is asked
for is a 404. Installing a library is what gives it pages.

This is `v: 2` of `plan/space-releases.md`, with two choices made since:

- **The kernel lives in the space.** Its wasm is a blob of the space, and the
  boot cell pins it by hash. A space carries its own runtime and needs no
  store of releases to keep running; a device that syncs the space has its
  kernel.
- **A route renders a model.** The first thing a space installs routes to is
  a model's view, as `route` does today, not only content.

It builds on `plan/space-workers.md` (a space on its own origin, with its own
worker) and on the kernel spike (`spike/wasi-kernel`): a Tonk kernel built for
`wasm32-wasip3` and run in the browser by a Rust embedder on the Wasm
Component Model Polyfill (`wcmp`), booting in about 100 ms.

## Host and kernel

The host is what every space can rely on without holding it. The kernel is
what a space holds.

| Piece | Owner | What it is |
|---|---|---|
| Shell page | Host | The document a new origin is answered with. Reads the boot record and registers the bootloader. |
| Bootloader (`boot.js`) | Host | The worker script. Owns the worker's events, loads the embedder, boots the kernel the space pins, hands it each request. |
| Embedder | Host | Rust, built for the browser, loaded by the bootloader. Implements the `sapling:kernel` world over the browser: the space's store in IndexedDB, the network, the session. Compiles and instantiates the kernel with `wcmp`. |
| Kernel | Space | A component of the `sapling:kernel` world, built for `wasm32-wasip3`. Opens the space's repository on the store it is given and answers `wasi:http/handler`. |
| Everything else | Space | Libraries, routes, views, `tonk-display`, styles: data and blobs of the space, installed into it. |

The host keeps the embedder and the `sapling:kernel` world stable, as
`plan/space-releases.md` asks of a host. A kernel built against a world the
host serves keeps running after any host upgrade.

## The boot cell

`boot/loader`, as in `plan/space-releases.md`, with `v: 2`:

```
{ "v": 2, "kernel": "bafkr4i…" }
```

`kernel` is the content identifier of the kernel's wasm: a CIDv1, raw codec,
BLAKE3 (what the spike checks). The wasm is a blob of the space under that
hash. The shell's boot record mirrors the cell, as for `v: 1`, so the shell can
read it before anything boots.

## What the kernel does

The kernel is today's space worker (`release.js` and the Rust worker it runs)
with nothing seeded:

- `/api/*`: query, subscribe, transact and evaluate on the space's branches.
- A path a `route` matches: the page that renders the route's model. A path a
  `route/http` matches: that route's response.
- `/asset:{hash}`: the asset, with its recorded media type and size.
- Anything else: 404.
- Sync with the space's remote, through the world's `network` interface.

It does not seed the standard library. Seeding is installing, and installing
is the space's choice.

## Creating a space

1. The profile makes the space (its key and delegation), as it does today.
2. It hands the space's origin the delegation and the bytes of the kernel this
   deployment ships.
3. The space's shell writes the kernel into the space's store as a blob,
   writes the boot cell and the boot record, and registers the bootloader for
   that kernel.
4. The bootloader boots it. The space answers `/` with 404.

After that, the profile installs what it wants the space to start with, such
as the standard library, through the kernel's `/api`: a library's routes and
views are what makes `/` render.

## Milestones

1. **A blank space boots its own kernel.** Creating a space installs the
   kernel; the space's worker boots it from the space's own store and answers
   every page with 404 and `/api/health` with 200. Kernel: the spike's, which
   holds a repository and answers `/api/transact` and `/api/select`.
2. **Tonk's API in the kernel.** The space's routes of today (`/api` query,
   subscribe, transact, evaluate; `/asset:`) from Tonk's own crates, built for
   `wasm32-wasip3`. Needs `tonk-schema`, `dialog-reactor` and the worker's
   space router to build there (first gap: `getrandom` 0.3, pulled by
   `rand` 0.9, has no `wasip3` backend).
3. **Install, then render.** Installing a library through `/api/evaluate`
   writes its routes; the kernel matches `route` and `route/http` and serves
   the page that renders the model. `/` goes from 404 to the library's page.
4. **Sync.** The kernel pushes and pulls through `network`; a second device
   that is handed the space gets the kernel blob with it and boots it.
5. **Upgrade.** A new kernel is a new blob and a moved pin, one commit; the
   bootloader switches without dropping open responses.
