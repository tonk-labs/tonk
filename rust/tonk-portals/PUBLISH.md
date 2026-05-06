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

A `.carry/` directory appears, holding the local replica.

## Asserting an HTML page

Carry assertions are EAV triples — `(the, of, is)`. For an HTML page,
the shape is:

| Field | Value |
| --- | --- |
| `the` | `text/html` (the MIME type — what the worker serves the body as) |
| `of`  | `<name>` (the bookmark name — carry derives the entity DID from `blake3(name)`) |
| `is`  | the HTML body, as a YAML block scalar |

### 1. Write a YAML assertion

Create a file (for example `/tmp/<name>.yaml`):

```yaml
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
- For multiple pages, add more `- the: … of: … is: …` blocks to the
  same list.

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

## Subresource caveat

The service worker reroutes every fetch a guest iframe makes back
under the same `/api/repository/<repo>/branch/<branch>/…` namespace —
including cross-origin URLs (this is a known bug in
`tonk-worker/src/worker.rs::route_for`). So images from
`https://example.com/foo.png` will 404. Two ways around it:

- **Inline assets**: paste an `<svg>` directly into the HTML, embed
  PNG bytes via `data:` URI, etc. No subresource fetch, no SW
  interception.
- **Assert the asset as another entity**: e.g. assert
  `(the=image/png, of=banner, is=<bytes>)`, then reference it via
  `/api/repository/<repo>/branch/main/host/<host>/banner.png`. The
  trailing `.<ext>` overrides the MIME type; the worker maps the
  extension to a `the:` attribute and serves the asserted bytes.

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
