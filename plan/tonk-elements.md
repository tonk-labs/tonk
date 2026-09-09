# `element` — author-defined custom elements, resolved on demand

## Context

A view template renders through inert fragments, so a `<script>` written
in a `show:` template never executes. Behaviour a template cannot express
(rich editing, canvas, drag) is therefore packaged as a **web component**:
JavaScript carried as branch data and executed into the rendering realm.

That mechanism exists today as the `component` concept plus the
`<tonk-component>` loader. It works, but it is the odd one out in the
vocabulary: `concept` (shape), `view` (presentation), `command` / `event`
/ `rule` (behaviour) are all named, resolvable, and superseded by
re-assertion. A component is none of those. This document proposes
replacing it with `element` — same capability, but keyed by the tag it
defines, resolved on demand by the same shape of query `<tonk-display>`
already runs for a view.

## What exists today

The concept (`rust/tonk-core/assets/library/core.yaml:1437`) carries one
field:

```yaml
concept!: &component
  description: A JS module defining web components for the branch's views
  with:
    module:
      the: xyz.tonk.component/module
      cardinality: one
      as: text
```

The loader (`rust/tonk-display/src/component.rs`) resolves a source — the
`module` attribute, or the text of inert `<script type="tonk/module">`
child holders — and appends a real `<script type="module">` to `<head>`,
de-duplicated by an FNV-1a hash of the source. That half is sound: it is
the one insertion path the HTML spec executes, and the hash key makes the
same module mounted by many rows execute once.

Getting a component onto a page takes two manual steps:

1. **Mount the directory.** The author puts `<tonk-display model=component />`
   in a view that always renders (`core.yaml:1461` is the hidden directory
   facet it resolves). That mounts one `<tonk-component>` per row on the
   branch.
2. **Define the tag inside the module.** Every module opens with
   `customElements.get('x-foo') || customElements.define('x-foo', …)`.

## Three problems

### 1. Re-asserting a component accumulates rather than replaces

An assertion with no `this:` is lowered to `id:<body-digest>`
(`rust/tonk-analyzer/src/analyzer.rs:246`). The body includes `module`, so
editing the source yields a different digest, a different entity, and a
**second row** — the original is still on the branch. The directory facet
mounts both, both modules execute, and the `customElements.get(name) ||`
guard means whichever executes first wins. Execution order is directory
row order.

The `&anchor` does publish a name (`db.name/referent` on `id:<anchor>`,
`rust/tonk-evaluator/src/evaluate.rs:1346`) and re-asserting repoints it —
but the directory query never consults the name registry, so the
repointing has no effect on what loads.

So the documented behaviour, "an edited component takes effect on the next
page load", is optimistic: it may never take effect. This is the one part
of this document that is a bug fix rather than a design change.

*Open question:* the row order the directory query returns decides which
definition wins. A test should pin this down before the migration, since it
determines whether existing branches silently flip behaviour when they
move to `element`.

### 2. Three loading strategies for one idea

"A custom element a view may use" is served three different ways in the
same realm:

| Source | Mechanism | Loading |
|---|---|---|
| `<tonk-*>` built-ins | hardcoded Rust list (`rust/tonk-guest/src/bin/guest.rs:38`) | eager, compiled in |
| `<wa-*>` (Web Awesome) | autoloader over `:not(:defined)` | **on demand** |
| author components | `<tonk-display model=component />` | eager, whole branch |

The middle row is the behaviour we want, and it is already running in this
page. `rust/tonk-ui/assets/webawesome/chunks/chunk.RSUSAXIB.js` is a
`MutationObserver` on `document.documentElement` that collects
`:not(:defined)` tags, filters by prefix, de-dupes, and imports one module
per tag. The proposal below is that mechanism with the branch as the
module source instead of a file path.

The current strategy also means every component on a branch executes on
every page whether or not anything renders it.

### 3. No identity, so no on-demand resolution is possible

A `view`'s identity **is the model it renders**, which is why `view_query`
(`rust/tonk-template/src/resolve.rs:37`) pins `this` and projects the
`show` dictionary — one query, no join. A component has no equivalent
anchor: the tag name it defines exists only inside the JS string, where
nothing can query it. That is the structural reason bulk loading is the
only option today. Fix the identity and on-demand resolution falls out.

There is also no CLI surface at all — `rust/tonk-cli/src/bin/tonk.rs` has
`Concept` and `View` subcommands and nothing for components, so the only
authoring path is raw notation.

## Proposal

### The concept: identity is the tag

```yaml
concept!: &element
  description: A custom element definition carried as branch data
  with:
    module:
      description: |
        JS module source. Its default export is the element class; the
        loader performs the `customElements.define`.
      the: xyz.tonk.element/module
      cardinality: one
      as: text
```

An instance pins `this` to a URI derived from the tag:

```yaml
element!:
  this: element:tally-widget
  module: |
    export default class extends HTMLElement {
      connectedCallback() {
        this.addEventListener('click', () => this.dispatchEvent(
          new CustomEvent('bump', { bubbles: true, detail: { amount: 1 } })));
      }
    }
```

Three consequences, all of them things `view` already gets for free:

- **Resolution is one pinned query.** Given the tag `tally-widget`, the
  loader queries `this: element:tally-widget` projecting `module` — the
  same shape as `view_query`, no name lookup, no join, one round trip.
- **Re-assertion supersedes.** `module` is cardinality one on a stable
  entity, so an edit replaces rather than accumulates. Problem 1 is gone
  by construction.
- **It is addressable.** An optional `&tally-widget` anchor publishes the
  name, so `tonk show tally-widget` works like any other entity.

`element:` joins `id:`, `tonk:` and `did:key:` as a URI scheme. The
alternative — keeping the anchor as identity and resolving through
`name_query` — needs no new scheme but costs a second round trip and puts
tag names in the same namespace as concept names. Pinning `this` is the
closer analogue of how `view` works, so it is what this document proposes.

### The module contract: the runtime owns `define`

The module default-exports a class. The loader calls
`customElements.define(tag, cls)`.

This removes the guard boilerplate and the tag repeated three times, but
the real reason is structural: the tag name has to be visible to the
resolver, and the runtime has to own the registry for anything later
(versioning, live swap) to be possible. A module that instead calls
`customElements.define` itself still works — it just cannot be resolved on
demand, because nothing outside the string knows what it defines.

### The loader: one autoloader, installed once

A realm-level autoloader installed beside the existing `with` observer in
`tonk-host`:

1. `MutationObserver` on the render root, `{ subtree: true, childList: true }`.
2. On each added subtree, collect `querySelectorAll(':not(:defined)')`.
3. Keep tags containing a hyphen that are not in the built-in `tonk-` set
   and not `wa-` (Web Awesome's own loader owns those).
4. De-dupe by tag, against both an in-flight set and a negative cache, so
   a tag with no `element` row is queried once per realm, not once per
   occurrence.
5. Query, then `customElements.define`. The browser upgrades every already
   rendered instance automatically — no re-render needed.

Nothing is mounted, nothing is declared, and a component nothing renders
never loads. `<tonk-component>`'s existing hash-keyed `<head>` injection
stays as the execution primitive underneath.

### The alignment: an element as a view implementation

`<tonk-display>` already has a renderer contract. It creates a
`<tonk-view>` (`rust/tonk-display/src/element.rs:2600`) and calls
`draw(frame)` on it with a `Conclusion` — the per-instance closure
installed at `rust/tonk-display/src/view.rs:136`.

That element name is hardcoded. Make it resolvable and an author element
implementing `draw(frame)` becomes a drop-in peer of the template
renderer: a view facet's value is then *either* markup *or* an element,
both fed the same conclusion, both resolved from the branch on demand.
This is the part that makes `element` coherent with `view` rather than
merely adjacent to it, and it is a one-line change plus resolution — the
seam already exists.

### Static checking

`rust/tonk-template/src/scan.rs` already walks tag names (line 123) and
discards them. Emitting a `Found::Element` variant lets the analyzer warn
when a template uses a hyphenated tag that is neither a built-in nor an
`element` on the branch — the same class of check it already performs for
unresolved `{field}` references, and the thing that turns "my component
silently did nothing" into a lowering diagnostic.

### CLI parity

`tonk element add <tag> --module-file …` and `tonk element list`,
mirroring `tonk view add` / `tonk view`.

## What this costs

**On demand trades a silent failure for a visible flash.** Today an
unloaded element is inert forever; on demand it is inert for one round
trip, during which its raw children paint. A cloak
(`:not(:defined) { visibility: hidden }` scoped to the render root, or the
`data-state` convention the other elements use) is part of the work, not
an optimisation to defer.

**Auto-loading widens the trust boundary, and that should be a decision.**
Today the explicit shell mount is a weak opt-in gate: branch JS runs
because someone wrote the mount. With resolution on demand, any template
that writes `<x-foo>` pulls arbitrary branch JavaScript into the realm
shared by every view on the branch — including when rendering a foreign
entity through a foreign view. The current answer ("the branch is the
trust boundary, exactly as it is for views and portals",
`component.rs:36`) is defensible for templates, which cannot execute. It
is a larger claim for JS that loads without anyone naming it. Options
worth weighing: an allow-list attribute on the render root; restricting
resolution to elements named by a view on the same branch; or accepting
it explicitly and writing that down.

**Live swap is not included.** `customElements.define` cannot redefine a
name, so an edited element still needs a page load — matching today's
behaviour, minus the accumulation bug. Making elements live-update like
views do means defining a stable shell class that delegates to the current
export and re-running `connectedCallback` on live instances, which needs a
defined delegate lifecycle. Scoped registries (`new CustomElementRegistry()`,
`attachShadow({ customElements })`) are the other route; browser support
needs checking before either is planned. Phase 2 at the earliest.

## Phasing

1. **`element` concept + pinned identity + `tonk element` CLI.** Fixes the
   accumulation bug on its own. `component` keeps working unchanged.
2. **The autoloader.** Removes the mount step. This is where the trust
   decision above has to be settled.
3. **Analyzer check** via `Found::Element`.
4. **Pluggable renderer** — `draw()`-implementing elements as view facets.
5. **Live swap**, if it earns its complexity.

`component` stays as a deprecated alias through at least phase 2 so
existing branches keep rendering; the directory-mount path is orthogonal
to the autoloader and can coexist.
