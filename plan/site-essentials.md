# A site loads only what it installs

A site (the profile, or a space) runs on its kernel and renders with what the
deployment builds in: the whole standard library is seeded into it, and its
pages load every built-in component whether the site uses it or not. The goal
is the bare minimum: a space seeds nothing it does not need, the profile seeds
only what `profile.yaml` declares, and a page loads only the components the
site has installed, when something on screen needs them.

## What a site gets today

Seeded (the kernel reads the library documents compiled in, `include_str!` in
`router/repository.rs`; nothing is fetched):

- A new space: all of `core.yaml` (46 concepts, 30 views, 11 commands, 9
  rules, 8 events, 4 elements, 6 library components) on `main`, from
  `create_content` when the space's kernel first boots.
- The profile: all of `profile.yaml` (61 concepts, 31 views, 37 commands, 14
  rules, 26 events, 17 elements, 12 routes) on its active branch, from
  `reconcile_profile_library`. It needs nothing from `core.yaml`, which is
  never on the profile.
- notebook, prose, table, meta, issue and tonk: only through `library/install`.

Loaded by every site page (`rust/tonk-kernel/assets/page.html`,
`tonk-portal/src/bootstrap.js`, `runtime_bootstrap.js`):

- the bridge (`bootstrap.js`), the guest wasm and its CSS, Web Awesome (`wa.js`,
  `wa.css`, the icon path), the preview capture, and the `tonk-prose` and
  `tonk-table` shells; `tonk-code` loads when a code element appears;
- what the guest's `start()` registers: tonk-display and the author-element
  registry, tonk-tree, all of tonk-fab (about twenty elements), the inspector
  and notebook, tonk-portal and tonk-site.

The hub needs a fraction of that: the bridge, tonk-display with the registry,
tonk-fab's palette and dialog, and three Web Awesome elements (button,
callout, skeleton). The inspector and tree serve `/inspector` and `/diagnose`;
portal and site serve `/space/*`.

On first load the hub sends about 370 queries to its kernel:

- The author-element registry (`tonk-display/src/registry.rs`), about two
  thirds: per element tag it subscribes to the tag's name and to each of the
  four element dictionaries (method, attribute, getter, setter), and re-reads
  all four on every frame of any of them: about 27 requests per element, for
  10 to 12 elements, though only one declares a getter or setter.
- Each `<tonk-display>`, about a third: its model, name, view and entity
  subscriptions, and the view's bindings, for 15 to 20 displays, most of
  them panels not on screen.

Each kernel is a 21 MB component, the persona's and a space's alike (measured
with twiggy on the core module, 2026-10-29):

| Size | What |
|---|---|
| 4.55 MB | the function names section, kept for debugging |
| 2.55 MB | `tonk-worker`: a space's kernel carries the persona's whole worker (accounts, custody, registration, onboarding, rotation) |
| 1.95 MB | read-only data: every library document (0.55 MB) and the two Welcome images (0.8 MB) |
| 1.94 MB | `core` |
| 1.40 MB, 1.35 MB | `dialog_query`, `dialog_repository` |
| 0.08 MB | `axum` itself, though its handlers' instantiations count as `tonk-worker` |

## Where it goes

1. **A space seeds what it renders and nothing else.** A `space.yaml` holds
   what a fresh space's page needs and what its kernel writes (`tonk:site`,
   `tonk:load`, the route concepts and the five seeded routes, the workspace
   shell, `tonk:blank` and its name, `tonk:repository`, `tonk:member`, the
   seed records, `tonk:space/kernel`), and `core.yaml`'s other contents become
   libraries a space installs. What Rust names by URI stays declared somewhere
   seeded (`tonk:blank`, `tonk:_`, `tonk:space`, `tonk:site`,
   `tonk:repository`, `tonk:member`, `tonk:invite`, `tonk:blob`, the
   `tonk:library/*` components).
2. **The profile seeds `profile.yaml`, which declares its components.** Its
   hub, account and settings stay; the inspector and diagnose routes become a
   library it installs.
3. **The renderer is a library.** `tonk-display` and the registry are what a
   site installs to render with; the page loads the renderer its site names,
   not the deployment's. Built-in elements (tonk-fab, tonk-tree, the
   inspector, notebook, portal, site, prose, table, code, Web Awesome) are
   each a component a library declares, loaded the first time an element of
   theirs appears. Nothing is preloaded.
4. **What is not visible stays inert.** A `<tonk-display loading="lazy">`
   that is not rendered (`hidden`, a closed dialog or menu, a collapsed
   panel) does not resolve its model, subscribe or render its children until
   it is, as React's `Activity` defers a hidden subtree. Opt-in, because the
   profile reads data from displays it never shows (the deletion's scope,
   the linked account); the library marks its dialogs and menus lazy.
5. **The registry reads an element once.** One query for an element's
   definition (its four dictionaries together), one subscription to it, and
   no re-read on frames that carry nothing new.
6. **The kernel needs no web framework.** The worker's routes move off axum to
   a small router over the requests the kernel's host hands it, which also
   drops the `Unthreaded` wrapper axum's `Send` bound forced.
7. **A kernel ships only what it runs.** The names section is stripped from
   what is deployed; a space's kernel is built without the persona's
   routes (accounts, custody, registration, onboarding, rotation); the library
   documents a kernel embeds are the ones it seeds; and the Welcome images
   move out of the kernel.
8. **A site's hostname is answered with its shell.** The server answers a
   site's address with the shell, not the app's page, so a first visit no
   longer downloads the app's bundle to throw it away.

## Order

1. Delete the browser-only paths of `tonk-worker` and the old workers. Done.
2. What a kernel ships (7): the names section is stripped (21 MB to 16.7 MB).
   Next, a space's kernel without the persona's routes (about 1 to 2 MB), and
   the Welcome images out of the kernel (0.8 MB). `wasm-opt -Oz` takes a
   kernel to 13.5 MB but only 6.2 MB to 5.7 MB compressed, for about 70
   seconds a kernel per build.
3. The registry and lazy displays (5, 4). Done: the hub's first load went
   from 373 requests to 218. No display is marked lazy yet. What remains is displays the browser renders
   though they draw nothing, the six registration panels foremost (one
   display that picks the stage would do), and a read of each command a
   view's bindings name.
4. `space.yaml` and the profile's own library (1, 2), with the URIs Rust names
   kept declared.
5. The renderer and built-in elements as installed components (3), and the
   site shell from the server (8).
6. Axum out of the worker (6): simpler, though it is 0.08 MB of a kernel.

## Open

- Where an installed component's code comes from: the deployment's files, as
  the guest bundle is now, or blobs in the space, as a library's media are.
- Whether a space that installs no renderer has a page at all, or the kernel
  answers it with its own plain one.
