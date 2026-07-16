# Spot name in the browser tab title

## Problem

The tab title is the static `<title>Tonk</title>` in `rust/tonk-ui/index.html:17`.
Nothing in the codebase ever sets `document.title`. With several spots open, every
tab reads "Tonk" and is indistinguishable.

The title should name the currently focused spot.

## Decision

On a `/space/{id}` route the tab reads `<Spot name> — Tonk`, live-subscribed to the
spot's name. While the name fact has not landed the tab reads `Untitled — Tonk`.

Declared in `profile.yaml`'s space view, delivered to the page over the existing
guest→host bridge.

## Why the page cannot resolve this itself

The two halves of the problem sit on opposite sides of an iframe boundary:

- `document.title` exists only on the top page. `rust/tonk-ui/src/bin/ui.rs` owns
  that document, but it knows only the URL path — it has the spot DID from
  `/space/{id}`, not the name.
- The name lives on the spot's own repository branch (`tonk/repository`, the
  `name-view` at `core.yaml:835-848`), rendered by chrome inside a sealed guest
  that cannot touch the top document.

The existing seam that crosses this boundary is the guest→host `MessagePort`
protocol in `rust/tonk-portal/src/bridge.rs`, which already carries `navigate`,
`fetch`, `subscribe`, `unsubscribe`. `navigate` is the precedent: a guest asking
the page to do something only the page can do (`bridge.rs:236` posts it;
`handle_navigate` at `bridge.rs:1423` performs it in five lines).

A title is the same shape of request, so it becomes a fifth message type.

## Why the mount goes in `profile.yaml`

A guest's port dispatcher (`make_dispatcher`, `bridge.rs:1167`) runs in its
**parent** document. Depth decides which document a title message lands on:

| Mounted in | Depth | Dispatcher runs in | Title would set |
|---|---|---|---|
| `profile.yaml` space view | 1 | the real top page | the real tab title |
| a spot's own route table | 2 | the profile guest's document | an iframe's title (invisible) |

`profile.yaml`'s `space-view` (`profile.yaml:2075-2079`) is a depth-1 guest, so its
messages reach the top page in one hop. A `<tonk-title>` mounted by the nested
`<tonk-site with="main@{id}">` sits at depth 2 and cannot retitle the tab without a
relay chain.

This is a constraint, not a preference. Same reason `handle_navigate` is only
correct one hop up.

## Why the view still lives in `core.yaml`

The mount's depth and the template's home are different questions.

`<tonk-display>` resolves its model **and its view** from its routing context. The
mount uses `with={id}` — the spot's own branch — so the view row it resolves must
be seeded on that branch, which means `core.yaml`. This is exactly how the hub card
resolves `tonk:repository/label-view` (`core.yaml:918-928`) under a per-space
`with={subject}`.

The element still *instantiates* in the depth-1 profile guest; only the data and
the template text come from the spot's branch. Both constraints are satisfied at
once: view row in `core.yaml`, mount in `profile.yaml`.

## Interpolation happens in the view, not the mount

Children of `<tonk-display>` are **slot fallbacks** (`no-model`, `no-entity`,
`loading`) — not a template. Field interpolation happens only in the resolved
view's `display:` template.

So `<tonk-title text="{name} — Tonk">` written as a direct child of the mount would
render the literal text `{name} — Tonk`. The `{name}` binding has to sit inside a
view row.

## The view kind

`tonk:view/label` (`core.yaml:898-928`) is the pattern to copy. It exists because a
model can carry more than one view: the default `tonk:view` resolution gives
`tonk:repository` its **editable name banner**, so a second, plain-text view needs
its own attribute (`xyz.tonk.view/label`) to avoid colliding, and is selected
explicitly with `view=tonk:view/label`.

A title view is a third view of the same model and needs the same treatment: a
`tonk:view/title` kind carrying its template under `xyz.tonk.view/title`, selected
with `view=tonk:view/title`.

Reusing `tonk:view/label` is not an option — its `tonk:repository` row renders
`{name}` as bare text, and a title needs a `<tonk-title>` element.

## Reading a name across repositories

The name lives on the spot's repo; the space view is profile chrome. This is
already a solved move, not a new capability:

- The top-level site is privileged — `allow="*"` (`ui.rs:51-60`).
- `profile.yaml:176-183` documents the hub card's name being read from "the listed
  space's OWN repository", explicitly *not* the profile-side
  `xyz.tonk.replica/name` record.

So `with="main@{id}"` in the space view is the same cross-repo read the hub and FAB
already perform, and it inherits the same cross-device correctness: a rename on
another device retitles this tab when the fact syncs.

## Reactivity

No subscription lifecycle is written. `tonk-display` already re-renders when the
name fact changes; the rename rule (`core.yaml:820-831`) overwrites the
cardinality-one name in place. The chain is:

```
rename → name fact overwritten → tonk-display re-renders
       → <tonk-title text> attribute changes
       → attribute_changed_callback → window.tonk.setTitle → document.title
```

`Untitled` needs no branch of its own: it reuses `slot="no-model"`, the same
convention the existing name-view uses to render "Untitled" until the name lands
(`core.yaml:838-848`).

## Components

Five changes, each small.

### 1. `tonk-host` — `set_title`

A new `rust/tonk-host/src/title.rs`, sibling to `navigate.rs` — not folded into it,
since a title is not a navigation. Exposes `set_title`, which sets `document.title`
on the current window and guards an empty string to a no-op so a blank render never
wipes the tab.

Note this is *not* installed under `page_effects` in `host.rs:68-94` like the
navigate provider. Nothing is pushed from the worker, so there is no listener to
install — the function is called by the bridge dispatcher, which already runs in
the page.

### 2. `tonk-portal` — the `title` message type

- Guest bootstrap: add `setTitle` beside `navigate` (`bridge.rs:225-237`), posting
  `{v:1, type:"title", text}`. Fire-and-forget, no response, exactly like navigate.
- Host dispatch: add `"title" => handle_title(&data)` to `make_dispatcher`
  (`bridge.rs:1177-1183`), reading `text` and calling `tonk_host::set_title`.

### 3. `tonk-portal` — the `<tonk-title>` element

A headless element whose only job is to push its `text` attribute over the bridge
on `connected_callback` and `attribute_changed_callback("text")`. It renders
nothing.

It lives in `tonk-portal` because that crate already owns the bridge and the
`window.tonk` surface; a dedicated crate for one headless element is not warranted.
Registered in `rust/tonk-guest/src/bin/guest.rs` `start()` alongside the other
element registrations.

### 4. `core.yaml` — the title view kind and its row

Mirrors `tonk:view/label` (`core.yaml:898-928`):

```yaml
concept!: &view/title
  this: tonk:view/title
  description: A tab-title view selected via `view=tonk:view/title`.
  with:
    model:
      description: Concept this view renders
      the: xyz.tonk.view/model
      cardinality: one
      as: entity
    display:
      description: HTML template for the title view
      the: xyz.tonk.view/title
      cardinality: one
      as: text

view/title!:
  this: id:tonk:repository/title-view
  model: tonk:repository
  display: |
    <tonk-title text="{name} — Tonk"></tonk-title>
```

### 5. `profile.yaml` — mount the title

Add to `space-view` (`profile.yaml:2075-2079`), beside the existing FAB mount. The
`view=` pin and the absence slots follow the FAB's own name chip
(`profile.yaml:859`):

```html
<tonk-site with="main@{id}" allow="main@{id}" path={rest}></tonk-site>
<tonk-display with="main@profile:tonk" model="tonk:profile/fab" data-space="{id}"></tonk-display>
<tonk-display with={id} entity={id} model=tonk:repository view=tonk:view/title>
  <tonk-title slot="no-model" text="Untitled — Tonk"></tonk-title>
  <tonk-title slot="no-entity" text="Untitled — Tonk"></tonk-title>
</tonk-display>
```

No `loading` slot: while the name is in flight the tab should keep whatever it
already reads rather than flash "Untitled".

The route pattern stays declared once, in the `route!:` table. No Rust file learns
what `/space/{id}` means.

## Scope

In scope: the `/space/{id}` and `/space/{id}/{*rest}` routes.

Out of scope, deliberately:

- **Other routes.** The hub, `/join`, `/inspector`, `/diagnose` keep the static
  "Tonk". Each is a separate `route!:` entry with its own view; titling the hub is
  one more `<tonk-title>` in that view and can follow if wanted.
- **Sub-route detail in the title.** Naming the open document within a spot would
  need a depth-2 relay (see the depth table). Not built.
- **The profile-side `xyz.tonk.replica/name`.** Not read, matching the hub.
- **Backfilling spots that already exist.** See the limitation below.

## Known limitation: spots created before this change

`core.yaml` is seeded onto a spot's branch **at creation only**
(`seed_library_urls`, `rust/tonk-worker/src/router/repository.rs:1740`). There is no
reseed. So a spot that already exists has no `tonk:view/title` row, its
`view=tonk:view/title` never resolves, and its tab keeps reading "Tonk".

This degrades gracefully rather than breaking, and it is why the mount deliberately
carries **no `slot="no-view"`**: giving one a title would pin every pre-existing
spot to a permanent "Untitled — Tonk", which is worse than the honest "Tonk".

Backfilling is a separate change with an established precedent — the one-shot
`GET /api/migrate/repo-vs-profile` endpoint (`rust/tonk-worker/src/router/migration.rs`)
exists for exactly this "stamp something onto repos that predate it" problem. Not
built here; new spots get titles, old spots keep "Tonk" until someone asks.

## Testing

- **Message parsing** (`title_text`): the natural unit boundary, mirroring the
  existing `navigate_href` test at `navigate.rs:173-200`. It asserts the *parse*,
  not the DOM effect — accepts only a well-formed `{type:"title", text}` with
  non-empty text; rejects other types, empty text, and non-object payloads. Uses
  `#[dialog_common::test]`, matching the file it copies.
- **The `core.yaml` additions** validate via `analyze_local` (seed tests are
  wasm-gated), per the established practice for library notation.
- **The rendered chain** (name fact → view template → attribute → `document.title`)
  is a browser concern and is verified in-browser, not in CI: a fresh spot titles
  its tab; a rename retitles it live; an unnamed spot reads "Untitled — Tonk"; a
  pre-existing spot still reads "Tonk". Per
  `project_wasm_tests_need_safari_automation`, local wasm tests need Safari
  automation or a major-matched chromedriver.

Note `set_title` itself is not meaningfully unit-testable beyond its guard —
asserting it would mean asserting on the test harness's own document title.

## Risks

- **Empty render wiping the title.** A `{name}` resolving blank would post an empty
  string and blank the tab. Guarded twice: `title_text` rejects empty text at the
  parse, and `set_title` no-ops on it.
- **Depth regression.** If the space view is ever moved into a deeper guest, the
  title silently stops working — it would retitle an iframe, with no error. The
  constraint is documented at the element and at the mount.
- **Silent failure on old spots.** The feature simply does nothing on spots created
  before it, which is easy to misread as a bug. Documented above as a limitation
  with a backfill path.
- **Guest bootstrap is a JS string.** `setTitle` is added to hand-written JS inside
  a Rust string literal (`bridge.rs:225-237`), so a typo is a runtime failure the
  compiler cannot catch. Keep it a near-copy of `navigate`, and exercise it in the
  browser check above.
