# Publishing HTML into a portal

How to put an HTML page on the substrate so it shows up as a tile in
`<tonk-portals>`. The portal picker resolves bookmark names to entity
DIDs server-side, so you only need to remember the name.

## One-time setup

You need the `carry` CLI on your PATH and a `.carry/` repo joined to
the same tonk-ui you're going to view the portal from. From the
running tonk-ui, mint a share link via the share button (or
`POST /api/repository/<repo>/invite`) — it's a URL like:

```
http://localhost:8080/join?access=2Doz…&remote=…#…
```

Pick a working directory and join:

```bash
mkdir -p ~/Workspace/prototypes/carry-tonk-viewer
cd ~/Workspace/prototypes/carry-tonk-viewer
carry join '<invite-url>'
```

A `.carry/` directory appears, holding the local replica. The
canonical location used by tonk-portals docs and skills is
`~/Workspace/prototypes/carry-tonk-viewer/.carry/` — every example
below assumes you're inside that directory or pass `--repo` to point
at it.

## Asserting an HTML page

Carry assertions are EAV triples — `(the, of, is)`. For an HTML page,
the **minimum viable assertion is two triples**: the body, and a
bookmark so the picker can find it by name.

| Field | Value |
| --- | --- |
| `the` | `text/html` (the MIME type — what the worker serves the body as) |
| `of`  | `<name>` (a bookmark name — carry derives the entity DID from `blake3(name)`) |
| `is`  | the HTML body, as a YAML block scalar |

> **Important:** in EAV-triple format, `of: <name>` derives the
> entity DID via blake3 *but does not* assert
> `dialog.meta/name`. The portal picker's `/resolve/<name>` route
> queries `dialog.meta/name`, so without that second triple you'll
> get *"no entity bookmarked as 'X' on branch 'main'"* even though
> the body is in the store. Always include the bookmark triple.

### 1. Write a YAML assertion

Create a file (for example `/tmp/<name>.yaml`):

```yaml
- the: dialog.meta/name
  of: <name>
  is: <name>

- the: text/html
  of: <name>
  is: |
    <!DOCTYPE html>
    <html>
      <head>
        <meta charset="utf-8">
        <title><title></title>
      </head>
      <body>
        <h1>Hello from carry</h1>
        <p>This page lives on the substrate as <code><name></code>.</p>
      </body>
    </html>
```

Notes:
- The `|` after `is:` is YAML's literal block scalar — it preserves
  every line break and leading whitespace inside the indented block.
- Anything under `is: |` must be indented at least one space deeper
  than `is:` itself; the indentation level is stripped before the
  body is stored.
- For multiple pages, add more triple blocks to the same list. Each
  page needs both its `dialog.meta/name` bookmark and its `text/html`
  body.

### 2. Assert and push

From inside the joined `.carry/` directory:

```bash
carry assert /tmp/<name>.yaml
carry push
```

`assert` writes the claims into the local replica; `push` syncs them
upstream so the tonk-ui can read them.

### 3. Open the page in a portal

Open the portals page for the same space:

```
http://localhost:8080/portals/<space>
```

Drag in the grid to create a tile, type `<name>` into the picker
(no `did:` prefix — the worker's
`/api/repository/<space>/branch/main/resolve/<name>` route looks up
the DID), leave the branch as `main`, and hit **Load**. The HTML
renders inside the iframe.

The portals shell also runs a background sync (pull-then-push every
3s on `branch=main`), so a `carry push` from the CLI side propagates
into a running portal automatically without the user reloading. The
iframe content itself doesn't auto-refresh — see *Live updates from
inside the iframe* below.

## Updating an existing page

Re-assert with the same `of: <name>` — carry treats the same name as
the same entity, so the new claim replaces the old `text/html` body
on the next push:

```bash
# edit /tmp/<name>.yaml — change the body under `is: |`
carry assert /tmp/<name>.yaml
carry push
```

Reload the iframe to see the change. The portal picker caches name →
DID resolutions in-memory, but the entity DID is stable across
updates so re-resolution isn't needed.

---

## Reading and writing data from inside the iframe

A page served at `/api/repository/<repo>/branch/<branch>/host/<host>/<entity>`
is **not read-only**. The worker registers a guest binding for the
iframe's service-worker client ID, and from then on every fetch the
iframe makes (with a path that doesn't start with `/api/`) is
rewritten under `/api/repository/<repo>/branch/<branch>/…`. That gives
the iframe a virtual root inside its branch, including the mutating
endpoints.

### Endpoints available to artifacts

Path inside the iframe → effective backend route:

| Iframe call | Rewritten to |
| --- | --- |
| `GET /claim/select?of=<entity>&the=<ns>/<name>` | `…/branch/<branch>/claim/select?…` |
| `POST /claim/assert/<entity>/<ns>/<name>` | `…/branch/<branch>/claim/assert/<entity>/<ns>/<name>` |
| `POST /claim/retract/<entity>/<ns>/<name>` | `…/branch/<branch>/claim/retract/<entity>/<ns>/<name>` |
| `POST /evaluate` (asserted-notation body) | `…/branch/<branch>/evaluate` |
| `POST /query` (`Accept: text/event-stream` for SSE) | `…/branch/<branch>/query` |
| `POST /sync`, `/sync/pull`, `/sync/push` | `…/branch/<branch>/sync…` |

`/claim/select` accepts `the` and/or `of` as query params (at least
one is required). `the` must be in `<namespace>/<name>` form.

### Self-discovery from inside the iframe

Parse `window.location.pathname` to learn the repo, branch, and the
page's own entity DID:

```js
const m = location.pathname.match(
  /^\/api\/repository\/([^/]+)\/branch\/([^/]+)\/host\/[^/]+\/(.+)$/
);
const REPO   = decodeURIComponent(m[1]);
const BRANCH = decodeURIComponent(m[2]);
const SELF   = decodeURIComponent(m[3]);   // did:key:z6Mk…
```

Inside the iframe, write fetches as relative URLs (`/claim/...`).
Don't hardcode `/api/repository/<repo>/...` — the SW does the
rewrite, and writing the absolute URL only works as long as the
iframe's binding agrees with what you typed.

### Sandbox caveats

Tiles render with `sandbox="allow-scripts allow-same-origin"`. That
means the iframe **can run scripts and use `fetch`/IndexedDB/etc.**,
but several browser features are off by default:

- `<form>` submission (incl. pressing Enter inside an input that's
  in a form) is blocked. Use plain buttons + click handlers and
  bind your own `keydown` listener for Enter. Without
  `allow-forms`, the submit event never fires — there is *no*
  console error.
- Popups, top-level navigation, and pointer lock are blocked.
- Cross-origin subresource fetches don't reach the network — see
  *Subresource caveat* below.

If the page genuinely needs forms, the host shell would have to add
`allow-forms` to the sandbox attribute on the iframe (see
`Square.tsx` and `space.rs`). Working around it per-page is usually
easier than fighting the shell.

### Live updates from inside the iframe

The portals shell auto-syncs the repo every 3 seconds, but the
iframe's DOM doesn't redraw on its own. Two patterns:

- **Polling**: re-issue your `/claim/select` on a `setInterval` and
  diff the result. Simple and works.
- **SSE subscription**: `POST /query` with `Accept:
  text/event-stream` and the same body shape as a one-shot `/query`.
  The connection re-broadcasts every time the branch advances. More
  efficient but you have to parse SSE frames and build a serialized
  `ConceptQuery` body.

A manual *refresh* button is a perfectly good middle ground for
prototypes.

### Talking to the portals shell (postMessage)

Artifacts can ask the shell to swap the tile's contents or close the
tile via `window.parent.postMessage`. The shell validates the
origin (same as its own — sandboxed iframes carry `allow-same-origin`
so they post under the shell's real origin) and dispatches on the
message `type`. The contract lives in `src-js/lib/tile-messages.ts`.

Two message types today:

```js
// Load a different artifact into THIS tile, by bookmark name —
// the shell does the /resolve/<name> call so artifacts don't have
// to hardcode DIDs. `branch` is optional (defaults to the tile's
// current branch). You can also send a raw `entity: "did:key:…"`.
parent.postMessage(
  { type: 'tonk:navigate', name: 'todo' },
  window.location.origin,
);

// Close THIS tile (same as clicking the × in the chrome).
parent.postMessage({ type: 'tonk:close' }, window.location.origin);
```

Why this beats `location.href = …`:

- The shell's tile state stays in sync — `square.entity` and
  `square.name` get updated, the chrome's title label updates, the
  picker's "current value" reflects what's actually loaded.
- Bookmark resolution is centralised in the shell's cache; multiple
  tiles asking for the same name share one resolve round-trip.
- Future capabilities (request fullscreen, request resize, etc.)
  can be added as new `tonk:*` types without each artifact reaching
  into framework internals.

### Subresource caveat

The service worker reroutes every fetch a guest iframe makes back
under the same `/api/repository/<repo>/branch/<branch>/…` namespace —
including cross-origin URLs (this is a known bug in
`tonk-worker/src/worker.rs::route_for`). So images from
`https://example.com/foo.png` will 404. Two ways around it:

- **Inline assets**: paste an `<svg>` directly into the HTML, embed
  PNG bytes via `data:` URI, etc. No subresource fetch, no SW
  interception.
- **Assert the asset as another entity**: e.g. assert
  `(the=image/png, of=banner, is=<bytes>)` plus the matching
  `dialog.meta/name` bookmark, then reference it via
  `/api/repository/<repo>/branch/main/host/<host>/banner.png`. The
  trailing `.<ext>` overrides the MIME type; the worker maps the
  extension to a `the:` attribute and serves the asserted bytes.

---

## Designing artifact data so it's discoverable

The substrate makes it dangerously easy to scribble untyped data into
attributes that nobody else can find. Three concrete failure modes
this has caused already:

1. **Bookmark vs concept confusion.** A bookmark name like `todo` is
   *not* a concept name. `carry query todo` looks like the right
   command and fails with "concept not found", which sends an agent
   off chasing concept definitions instead of toward the entity that
   the bookmark resolves to.
2. **Opaque blobs.** A single attribute holding a JSON-stringified
   array (e.g. `app.todo/list = "[{...}]"`) is invisible to schema
   discovery. There's no concept to query, no fields to project, no
   way to find it without already knowing the namespace.
3. **Self-referential storage.** A page entity holding its own data
   under its own DID is a reasonable pattern, but not one a fresh
   agent will guess. They'll look for separate `todo-task` entities
   under a `todo` concept and come up empty.

Pick one of the storage strategies below depending on whether other
agents/humans should be able to read your data without reading your
code.

### Strategy A — Typed claims, one entity per record (recommended)

Best for anything that looks like rows: todos, notes, comments,
tags. Each record is its own entity with proper attributes; the
schema is itself stored as claims so it's discoverable.

```yaml
# Define the attributes once.
- the: dialog.meta/name
  of: todo-text
  is: todo-text
- the: dialog.attribute/the
  of: todo-text
  is: app.todo/text
- the: dialog.attribute/as
  of: todo-text
  is: Text
- the: dialog.attribute/cardinality
  of: todo-text
  is: one

- the: dialog.meta/name
  of: todo-done
  is: todo-done
- the: dialog.attribute/the
  of: todo-done
  is: app.todo/done
- the: dialog.attribute/as
  of: todo-done
  is: Boolean
- the: dialog.attribute/cardinality
  of: todo-done
  is: one
```

Or, more ergonomically, with the `carry` CLI:

```bash
carry assert attribute @todo-text \
  the=app.todo/text as=Text cardinality=one
carry assert attribute @todo-done \
  the=app.todo/done as=Boolean cardinality=one
carry assert concept @todo-task \
  description='A todo item' \
  with.text=todo-text with.done=todo-done
```

Then assert items as entities under the concept:

```bash
carry assert todo-task text='write the docs' done=false
carry assert todo-task text='ship'             done=false
```

From inside the iframe, create new items by minting a fresh entity
DID via `POST /evaluate` (the asserted-notation pipeline allocates
entity DIDs for unbound subjects); to update or delete, you have
the entity DID in hand from the previous read. Listing and
filtering use `GET /claim/select?the=app.todo/text` — every entity
with text shows up.

A new agent landing in this repo can run:

```bash
carry query concept              # lists every defined concept
carry query attribute            # lists every defined attribute
carry query todo-task            # lists every item with all fields
```

…and immediately see the data shape without reading any source code.

### Strategy B — Single typed-string claim (acceptable, easy)

If you really want a single blob (small lists, simple state), at
least define an attribute for it so it shows up in schema queries:

```bash
carry assert attribute @todo-list \
  the=app.todo/list as=Text cardinality=one \
  description='JSON-encoded array of {id,text,done} todo items, owned by the page entity itself'
```

The description is the discoverability hook — anyone running
`carry query attribute` sees that `app.todo/list` exists, what the
shape of the value is, and that it's owned by the page entity.
Cardinality `one` means asserts replace, so the page can skip the
retract-then-assert dance.

### Strategy C — Untyped blob (only for throwaway prototypes)

What the bundled `todo` example does today: assert
`(the=app.todo/list, of=<page-entity>, is=<json-string>)` with no
attribute definition. **Don't ship this**. It works, it stores data,
and absolutely nothing about it is discoverable from the carry side.
Use it only for one-off experiments, and migrate to Strategy A or B
before another agent has to reason about your data.

### Conventions to make any strategy easier to crawl

- Use a stable, app-scoped namespace: `app.<thing>/...` for app
  data, `meta.<thing>/...` for app metadata. Avoid colliding with
  the reserved `dialog.*` namespace.
- For each app, assert one **manifest concept** that points at the
  data it owns. For example:

  ```bash
  carry assert concept @todo-app \
      description='Companion concept for the todo HTML page; data lives at app.todo/*' \
      with.list=todo-list
  ```

  Then `carry query concept` surfaces a one-line trail from the app
  name to the data namespace.

- Reference the page entity from the schema. Asserting
  `(the=meta.app/page, of=<concept>, is=<page-entity-DID>)` lets
  someone go from the concept back to the HTML, closing the loop.

---

## Reading artifact state from the carry side

For other agents (or humans) who want to inspect what an artifact has
written to the repo, without reading the artifact's source. Run from
inside `~/Workspace/prototypes/carry-tonk-viewer`.

### Get current with the upstream

```bash
carry pull
```

The portals shell pushes every 3s, but `carry pull` is needed
locally to see writes the iframe just made.

### Find what bookmarks exist (the entry point)

```bash
carry query bookmark
```

Every named entity surfaces here — including the page entities
themselves. The DID next to a name like `todo` is the entity that
holds that page's content (and any data the page asserted on
itself).

### List defined attributes and concepts

```bash
carry query attribute
carry query concept
```

These are the schema. If the artifact authors followed Strategy A or
B above, the data namespace shows up here with description text
explaining what's stored.

### Project all claims on a known entity

```bash
carry query --of did:key:z6Mk… --format triples
# or via the worker route from anywhere:
curl 'http://localhost:8080/api/repository/<repo>/branch/main/claim/select?of=did:key:z6Mk…'
```

Useful when you have an entity DID (from `carry query bookmark`) and
want to dump every claim on it. You'll see both the page body
(`text/html`) and any data the iframe wrote.

### Project all claims with a known attribute

```bash
curl 'http://localhost:8080/api/repository/<repo>/branch/main/claim/select?the=app.todo/text'
```

Useful when you know the namespace (Strategy A) and want every
record across entities.

### Watch live

```bash
curl -N -H 'Accept: text/event-stream' \
  -X POST -H 'Content-Type: application/json' \
  --data '{"...ConceptQuery body..."}' \
  'http://localhost:8080/api/repository/<repo>/branch/main/query'
```

The same SSE endpoint the iframe can use, from a terminal. Re-emits
on every commit to the branch.

---

## Bookmark naming

Names are arbitrary strings. Carry derives the entity DID
deterministically from `blake3(name.as_bytes())`, so:

- The same name on different branches resolves to the same DID, but
  the body lives per-branch (each branch's `(the, of)` claim is
  independent).
- Two replicas asserting under the same name converge — useful for
  "well-known" pages like `index` or `readme`.
- Names with slashes work fine (`pages/about`, `notes/2026/05/06`),
  though the picker URL-encodes them so paths stay valid.
- A bookmark name only resolves via the picker if you also asserted
  `dialog.meta/name = <name>` on the entity (see the **Important**
  callout at the top of *Asserting an HTML page*).
