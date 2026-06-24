# Generalized root view + extracted chrome

**Date:** 2026-06-24
**Status:** Design approved, pending spec review

## Problem

The Tonk Hub UI forces every space into one shell. `core.yaml` seeds `tonk:binder`
as the space model; its directory view `tonk:shell` welds together the topbar and a
tab-switching `<tonk-sheet-binder>`. Entering a repo means landing in a binder, and
anything you build is trapped inside a sheet. You cannot put a dashboard of mixed
cards on one page, give a vibe-coded app the whole view, or lay out a wiki — the
binder is mandatory, not chosen.

After sealing the space render inside an opaque-origin iframe, the outer tonk-ui
Leptos shell became mostly dead weight: the guest's only real need from the host is
`fetch → service worker`. A parallel effort moves routing into concepts (`route` /
`site`) and dissolves that shell. This design rides on top of that direction.

## Goal

Make the binder one template among several, not the shell. A space renders a single
editable root composition. When empty it shows a launchpad ("paste a link to an
agent" / "choose a template"); when a template is applied it renders that
composition — dashboard of cards, full-bleed app, wiki, or the sheets model. The
topbar is the only remaining global chrome. Then account for how much of the
binder/sheet machinery can be deleted, relocated, or simplified.

## Scope

**In scope (this plan):**
- Generalize the space so `tonk/space` renders one root view instead of the
  hardcoded `tonk:binder` shell.
- Make the launchpad the default empty-state entry.
- Extract the topbar out of `tonk:shell` into the route view as protected,
  declarative global chrome.
- Convert today's mandatory binder/sheets into the **first opt-in template**,
  proving the template mechanism end to end by re-expressing current behavior.
- A simplification map: what binder/sheet machinery is deleted, relocated, kept.

**Out of scope (follow-up specs):**
- The dashboard, wiki, and single-app templates. This plan only enables them and
  ships sheets-as-template.
- Any new in-app compose/edit UX. Editing uses the existing notation/code path.
- The conceptual-routing substrate itself (route/site concepts, SW routing table,
  `navigate!`, shell dissolution). That lands separately; this plan assumes it and
  flags seams where a piece is not yet present.

## Assumed substrate (not built here)

Conceptual routing is landing as parallel work (the June 26 notes; PRs #526, #529):

- `route!` binds a path pattern to a concept/view. The service worker builds a
  routing table (axum matchit; conflicting routes are quarantined and discoverable).
- `site` carries per-`clientId` context in an overlay — `path`, `anchor`, `replica`,
  `route`, `concept`. `navigate!` updates it; the guest renders
  `<tonk-display entity=site:… model=site>`.
- The `route/space` **view** maps the matched path to markup, wrapping the space in
  `<tonk-host><tonk-repository><tonk-branch>`.

This plan targets that world. Where a piece is not yet landed, the implementation
plan notes the seam rather than reimplementing it.

## Design

### Layer model

Three layers with hard boundaries:

```
route/space view  (protected, declarative — routing-level chrome)
  <tonk-host>
    <tonk-repository name={subject}>
      <tonk-branch name=main>
        [ TOPBAR ]                       <- extracted here
        <tonk-display model=tonk/space>  <- the ONE editable root composition
      </tonk-branch>
    </tonk-repository>
  </tonk-host>
```

- **Route view** — protected chrome. Seeded routing data, not the user's editable
  composition, so the user cannot remove it by editing their page.
- **Topbar** — declarative. Brand, "< all repos" breadcrumb, editable repo name,
  sync chip (`state:here`), identity chip (`state:self`), share. Scoped to the local
  replica via `site.replica`.
- **Root composition** — `<tonk-display model=tonk/space>`. The single editable view
  the user composes freely.

Putting the topbar in the route view (rather than a bespoke host-owned Leptos region)
keeps it declarative and protected while advancing shell-dissolution instead of
adding to the dead-weight host.

### Generalized root + launchpad

`tonk/space` stops resolving to the privileged `tonk:binder` shell. It resolves to a
generic **root** whose view has two states:

- **empty** (`data-state=empty`) → the **launchpad**: "paste a link to an agent" /
  "choose a template." Reuses the existing empty-view machinery (directory view stays
  mounted on empty and replays an empty frame; fallback chrome gated on
  `data-state=empty`). This is the natural home for the helpful-error / inline-fill
  direction — a partially-matched concept with missing fields you can fill in place.
- **populated** → delegates to the chosen composition.

The root/template concepts must not be named `workspace` — that collides with
tonk-layout.

### Template binding — selection pointer

The repo carries a single shared **active-layout pointer** on `main`. It is per-repo,
not per-replica — which is precisely why the old per-replica active-tab state leaked a
binder per participant. The root view delegates to the pointed composition via a
nested `<tonk-display>`:

- pointer **unset** → launchpad.
- pointer **set** → render the pointed composition.

An `apply-template` command seeds the composition onto `main` (if not already present)
and sets the pointer — "evaluated onto main like core.yaml." Switching templates later
is a repoint, avoiding content-addressed view-row retraction (editing a `(model, name)`
view row duplicates rather than replaces, and by-name retraction is unsupported; repoint
sidesteps this entirely).

### Sheets as the first template

The binder/sheet concepts, views, and the `<tonk-sheet-binder>` element relocate out of
the mandatory shell into a self-contained **sheets template package** (notation +
elements). Applying it sets the active-layout pointer to the sheets composition. This
re-expresses today's tab behavior as opt-in and proves the template mechanism end to
end.

## Simplification map

The win is decoupling, not mass deletion.

- **Delete**
  - `tonk:shell` as the mandatory space view welding topbar + binder together.
  - the hardcoded `tonk/space → tonk:binder` resolution.
  - the bespoke `space.rs` route fallback (subsumed by route/site).
- **Relocate**
  - topbar markup → the `route/space` view.
  - `tonk:binder`, `tonk:sheet`, `create-sheet` / `activate-sheet` / `close-sheet`,
    and `<tonk-sheet-binder>` → the sheets template package. They survive; they are no
    longer core shell.
- **Fix**
  - the directory-per-replica binder bug → scope the binder to `site.replica`. Largely
    moot once the binder is not the default.
- **Keep**
  - `<tonk-display>` / view / portal stack, the sealed iframe, the `state:here` /
    `state:self` overlays.

## Validation

- Seed changes validated via `analyze_local` (seed tests are wasm-gated).
- Native `clippy --all -D warnings` as the lint gate (wasm-gated handlers leave native
  helpers dead; verify natively, not just the wasm build; gate wasm-only helpers with
  `cfg(wasm32)`).
- wasm tests require Safari or Chrome automation on darwin.
- Manual acceptance:
  - fresh repo → launchpad.
  - apply the sheets template → tabs work as today.
  - topbar persists across template changes and is not removable by editing the page.
  - clear the active-layout pointer → back to the launchpad.
  - the binder no longer renders one panel per participant.

## Open seams to confirm during planning

- Exact name and shape of the generic root concept and the active-layout pointer
  (avoiding `workspace`).
- How far the conceptual-routing substrate has landed at implementation time, and which
  seams need a temporary shim (e.g. if `route/space` or `site.replica` are not yet
  available).
