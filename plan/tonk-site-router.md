# `<tonk-site>` — a routing portal

> **Status:** shipped, then amended by `plan/tonk-routing-attributes.md` —
> `<tonk-site>` now carries its own `with="branch@repo"` / `allow="…"`
> attributes (both required) instead of reading `<tonk-repository>` /
> `<tonk-branch>` ancestors, which no longer exist.

Converged design from the FAB/de-Leptos redesign discussion. Goal: collapse
routing, the FAB, and the de-Leptos shell into one recursive element, and shrink
the SW to a pure data API.

## The element

```html
<tonk-repository name={subject}>      <!-- containment context: which repo -->
  <tonk-branch name=main>             <!-- which branch -->
    <tonk-site path="/test" />        <!-- portal + router, scoped by ancestors -->
  </tonk-branch>
</tonk-repository>
```

`<tonk-site>` is `<tonk-portal>` plus path/router logic:

- It inherits `subject` / `branch` from its `<tonk-repository>` / `<tonk-branch>`
  ancestors (the existing routing-context annotators) — identity comes from the
  DOM, not its own attributes. Containment is unchanged: the SW scopes the
  guest's data requests to that repo/branch.
- It owns `path`. On connect (and on `path` change) it resolves the route for
  `(branch, path)` and renders the matched view inside its sealed iframe.
- `path` change re-keys the resolution WITHOUT re-creating the iframe (relay the
  new path over the existing MessagePort; do not re-assign `srcdoc`). Deferred:
  first cut may re-srcdoc; the no-reload relay is an optimization.

## Resolution = the existing `match_route`, branch-agnostic

`tonk-worker/src/router/session.rs:match_route` already does it: query the
branch's `route!` table, build a `tonk_router::Router`, `recognize(path)`,
return `(route, concept, params)`. It takes any `BranchSession`, so it works on
the profile meta branch and on any space branch unchanged. The matcher
(`tonk-router`, wasm) runs where the subscription is served — the SW — the same
place subscriptions already run. No new matcher, no duplicate engine.

So `<tonk-site>` does NOT run the matcher itself. It asks the data plane "resolve
`path` on this branch" and renders the matched concept. (Whether that's a one-
shot render fetch or a live subscription keyed on the route resolution is the
live-vs-dead question — DEFERRED; first attempt does the simplest thing that
paints the matched view.)

## Recursion via the rendered view

The matched view for a space route contains a nested
`<tonk-repository name={id}><tonk-branch><tonk-site path={rest}></...>`. Each
`<tonk-site>` is one route resolution scoped by its ancestors. The profile's
`<tonk-site>` matches `/space/{id}/{rest}` to a space-shell concept whose view
mounts the inner `<tonk-site>` for repo `{id}` and sub-path `{rest}`. Same
element, two levels.

The FAB lives in the space-shell route's matched view, so it is present exactly
when a space route matches — "FAB only in /space" falls out for free.

## What the SW sheds

- `register_site`, `stamp_site`, `/api/site`, and `resolve_path`'s
  document-path-parsing role. The SW stops deciding "what does `/` render."
- Containment stays in the SW (per-iframe repo binding + `/api/repository/{repo}`
  URL structure) — that is code/security and does not move.
- `match_route` stays in the SW but is reached as route resolution for a
  `(branch, path)`, not via document-path parsing.

## First increment (this attempt)

Prove the loop on ONE level, delete nothing:

1. `<tonk-site>` element: reads `path` + ancestor repo/branch, resolves the route
   (reusing `match_route` over that branch), renders the matched view in a
   sealed iframe (portal machinery).
2. A SW path to resolve `(repo/branch, path)` → matched concept's view markup,
   served to the element.
3. Drop `<tonk-site path={window.location.pathname}>` on `/` under
   `<tonk-repository profile><tonk-branch meta>`, against a profile `route!`
   table (seed `route! path:/ → tonk:hub` in profile.yaml). Renders the hub.

Leave the existing Leptos `/space` route and the SW's current stamping in place
(unused by this path) until `<tonk-site>` is proven, then delete.

## Open / deferred

- Live vs dead: subscription on route resolution vs one-shot render. Deferred.
- `path`-change-without-reload via the MessagePort relay. Deferred (first cut may
  re-srcdoc).
- Fold vs compose finally settled as FOLD: `<tonk-site>` IS the portal+router,
  scoped by `<tonk-repository>`/`<tonk-branch>` ancestors.
