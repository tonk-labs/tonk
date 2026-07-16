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

## Why `profile.yaml`, not `core.yaml`

A guest's port dispatcher (`make_dispatcher`, `bridge.rs:1167`) runs in its
**parent** document. Depth decides which document a title message lands on:

| Renders in | Depth | Dispatcher runs in | Title would set |
|---|---|---|---|
| `profile.yaml` space view | 1 | the real top page | the real tab title |
| `core.yaml` (spot's own `main`) | 2 | the profile guest's document | an iframe's title (invisible) |

`profile.yaml`'s `space-view` (`profile.yaml:2075-2079`) is a depth-1 guest, so its
messages reach the top page in one hop. Anything seeded onto a spot's own branch
renders inside the nested `<tonk-site with="main@{id}">` at depth 2 and cannot
retitle the tab without a relay chain.

This is a constraint, not a preference. Same reason `handle_navigate` is only
correct one hop up.

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

Four changes, each small.

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

### 4. `profile.yaml` — declare the title

Add to `space-view` (`profile.yaml:2075-2079`), beside the existing FAB mount:

```html
<tonk-site with="main@{id}" allow="main@{id}" path={rest}></tonk-site>
<tonk-display with="main@profile:tonk" model="tonk:profile/fab" data-space="{id}"></tonk-display>
<tonk-display with="main@{id}" model="tonk:repository" entity="{id}">
  <tonk-title text="{name} — Tonk"></tonk-title>
  <tonk-title slot="no-model" text="Untitled — Tonk"></tonk-title>
</tonk-display>
```

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

## Testing

- `set_title` and the empty-string guard: unit-testable in `tonk-host`.
- `handle_title` message parsing: follows the existing `navigate_href` test at
  `navigate.rs:173-200`, which asserts the parse (accepts only a well-formed
  `title` message; ignores other types and malformed shapes) rather than the DOM
  effect.
- The rendered chain (name → attribute → title) is a wasm/browser concern. Per
  `project_wasm_tests_need_safari_automation`, local wasm tests need Safari
  automation or a major-matched chromedriver; verify in-browser against a live
  spot, including a rename retitling the tab and an unnamed spot reading
  "Untitled — Tonk".

## Risks

- **Empty render wiping the title.** A `{name}` that resolves blank would post an
  empty string. Mitigated by the no-op guard in `set_title`.
- **Depth regression.** If the space view is ever moved into a deeper guest the
  title silently stops working (it would retitle an iframe). The depth constraint
  is documented at the element and in the space view.
