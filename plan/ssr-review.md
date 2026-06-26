# SSR critical review — findings

## Resolution status: ALL ADDRESSED

Every finding below was fixed or, where inherent to headless SSR,
documented. Summary of the fixes (commits on `feat/ssr-render`):
- Parser tree construction: normalization pass inserts implicit
  `<tbody>` and un-nests tag-omitted `<li>`/`<p>`/cells; unquoted
  `={...}` attribute values are quoted before `tl`. Verified against the
  real browser DOM.
- Serialization: `<style>`/`<script>` content emitted verbatim; entity-
  aware escaping (no double-encode); tag/attr names lowercased.
- Attribute dispatch: boolean presence/absence, absent-field omission,
  non-string property semantics — all matched to the browser via Chrome
  goldens.
- tonk render: `dom.host/*` injection, `_:_` default-view fallback,
  pinned-concept model name resolution, empty-attr nested-route guard,
  visited-set cycle guard.
- Tests: 7 tonk render integration tests; restored wasm coverage for
  the moved resolve/fold tests; new attribute/parse/serialize compat +
  unit tests. Docs: READMEs + `tonk render` in the guides.
- Inherent to SSR (documented, not "fixed"): a `text/html` portal's
  `display` is an author-written HTML *document* (not a template) that
  runs its own JS against the `window.tonk` bridge — the browser loads
  it verbatim with no `{field}` interpolation. SSR inlines the same
  verbatim content into `<iframe srcdoc>` (the faithful match) but omits
  the bridge bootstrap, which can't function headlessly, so the
  portal's own queries don't run. (Finding 12 below originally
  mischaracterized this as inlining "the template"; portals are not
  substituted in the browser either.) The property-vs-attribute case for custom elements that
  reflect a matching property can't be detected without a DOM; SSR
  writes the attribute (correct for standard attributes/elements).

The original findings, for the record:



Review of the `feat/ssr-render` work (tonk-template / tonk-render /
`tonk render`). Empirically verified findings, grouped by severity. The
headline correction: the "byte-identical to the browser" claim holds only for
**well-formed, lowercase, table-free, style-free, entity-free** templates with
literal-string fields. Several common real-world shapes diverge.

## Real bugs (parity-breaking)

### Parser tree divergence from the browser DOM (tonk-render/src/parse.rs)
`tl` is a lenient non-spec parser; the browser's HTML parser applies tree
construction the binding paths assume. Confirmed divergences:

1. **Implicit `<tbody>`** — `<table><tr ...>` parses with `<tr>` directly under
   `<table>` in `tl`; the browser inserts a `<tbody>`, so every binding path
   under a table is off by one level. Breaks table/sheet row templates (a
   common shape). VERIFIED.
2. **Tag-omission auto-close** — `<ul><li>{a}<li>{b}</ul>` nests the second
   `<li>` inside the first in `tl` (children=1); the browser auto-closes so they
   are siblings (children=2). Breaks templates that omit `</li>`/`</p>`/`</td>`.
   VERIFIED.
   - Fix direction: either preprocess via a spec-compliant parser (html5ever —
     heavier), or normalize the `tl` tree for the known tag-omission/foster
     cases, or document templates must be well-formed + explicit `<tbody>`.

### Serialization (tonk-render/src/serialize.rs)
3. **`<style>`/`<script>` content is HTML-escaped** — `.a > .b` serializes as
   `.a &gt; .b`, corrupting CSS/JS. Raw-text elements must emit children
   verbatim. The collector already skips descending into them; the serializer
   must mirror that. VERIFIED.
4. **Double-escaping pre-existing entities** — literal `&amp;` in template text
   round-trips to `&amp;amp;` (and same in attributes). `tl` keeps the raw
   source `&amp;`; the browser decodes to `&` in the DOM then re-encodes once.
   Fix: decode entities at parse time, or don't escape `&` that begins a valid
   entity. VERIFIED.
5. **Valueless boolean attribute** — `<input disabled>` round-trips as
   `<input disabled="">`; the browser serializer emits bare `disabled`. Byte
   divergence (semantically equivalent). Carry an `Option<String>` attr value.
6. **Tag/attr case not normalized** — `tl` preserves `<DIV CLASS>`; the browser
   lowercases. Knock-on: `is_void_tag`/`is_raw_text_element` (lowercase-only
   match) misfire on uppercase. `tree.rs` even *documents* "lowercased tag name"
   but `parse.rs` doesn't lowercase. Fix: lowercase tag + attr names at parse.

### Attribute value dispatch (tonk-render/src/render.rs set_attr)
7. **No property/boolean/typed dispatch** — the browser's
   `apply_attribute_binding`/`single_field_value` does: boolean `false` on a
   forced attr → attribute *absent*; `true` → empty `disabled=""`; non-string
   single-field value → set as a JS *property*, no HTML attribute; absent field
   → attribute omitted (not `name=""`). tonk-render always writes
   `name="<stringified>"`. Diverges for `html:disabled={bool}`, `value={number}`,
   and missing optional fields. Many of these are property-only in the browser
   so wouldn't appear in `outerHTML` at all; SSR emits a spurious attribute.
   VERIFIED by code comparison.

### `tonk render` resolution divergence (tonk-cli/src/render.rs)
8. **`{dom.host/*}` not injected (CRITICAL)** — the browser augments each
   conclusion with `dom.host/<attr>` fields (`with_host_attributes`) so a nested
   `<tonk-display entity={this} model={dom.host/model}>` resolves its model.
   tonk injects nothing, so `{dom.host/model}` renders empty, and the nested
   recursion then builds a route with `model=""` → "no concept matched". The
   canonical directory→detail nesting idiom is unusable in SSR.
9. **No `_:_` default-view fallback (CRITICAL)** — when the model-specific view
   query is empty the browser re-queries with `model="_:_"` (DEFAULT_MODEL) then
   falls to a notation dump; it never errors. tonk hard-errors "no view found".
   This is exactly the directory smoke-test failure.
10. **Model name not resolved via the Name concept** — tonk's `resolve_model`
    only `phase1_query`s; for a non-URI it relies on the `dialog.meta/name`
    filter, which (per resolve.rs's own comment) *misses pinned concepts*. The
    browser name-resolves non-URI models first. A model given as a pinned
    bookmark name (e.g. `workspace`) breaks in tonk.
11. **`ParsedSource` filters parsed then dropped** — `?key=value` constraints
    are never applied; tonk uses `entity_query`/`instances_query` (no filters),
    never `phase2_query`. A filtered route renders the unfiltered set.
12. **Portal renders an inert shell** — `render_portal` inlines the template in
    an iframe with no entity/model/descriptor context, so the portal can't fetch
    its data (the browser passes those so the bridge subscribes). Arguably
    inherent to static SSR, but currently undocumented and misleadingly
    "rendered".
13. **Empty entity → blank chrome silently** (no not-found signal); **multiple
    view rows → first only** (browser mounts all); **depth-only cycle guard**
    (MAX_DEPTH=16) risks false-positives on legitimate deep nesting (notably on
    a tree-inspector branch) vs a `(model,entity,view)` visited-set.

## Test gaps
- tonk-render: parity tests cover only well-formed lowercase templates. No test
  for tables, tag-omission, `<style>`, entities, boolean attrs, typed attr
  values, uppercase, the whole-fragment (path=None) repeat, or the no-`<template>`
  snapshot path's top-level whitespace/comment trimming.
- tonk-cli/src/render.rs: ONLY route-parser unit tests. Zero tests of
  `resolve_model`/`resolve_view`/`resolve_name`/`expand_nested`/`route_from_attrs`/
  `render_portal`, directory mode, empty-entity, filters, or `{dom.host/*}`.
- dialog-reactor: `QueryEffect` has no in-crate unit test (covered only via
  tonk-worker); `command::<C>()` registration + `run()` execute path untested
  in-crate (matching is covered).
- tonk-template: resolve.rs tests converted `#[dialog_common::test]`→`#[test]`,
  dropping wasm coverage (divergence from the repo convention); ~34 planner
  tests still live in tonk-display, not with the moved code.

## Documentation / hygiene gaps
- No README for `tonk-render` or `tonk-template` (every peer crate has one).
- `tonk render` is undocumented in the guides (guide-index, guide-views).
- `tonk-template`'s crate doc claims "pure std-only Rust" but it now depends on
  `tonk-schema` (→ the dialog stack) via resolve/fold. Misleading; also weakens
  the "destined for dialog-db repo, dependency-light" goal.
- `tonk-template/Cargo.toml`: `serde` dep is unused (drop it).
- `tonk-render/Cargo.toml`: `[dev-dependencies]` duplicate the normal deps
  (redundant; remove).

## Honest status
The architecture is sound and the happy path is proven against the real browser.
But "byte-identical" was overstated: it is byte-identical for the tested template
class only. Before this is production-trustworthy for arbitrary authored views,
the parser tree-construction divergences (1-2), raw-text/entity serialization
(3-4), and the two CRITICAL tonk-resolution gaps (8-9) need addressing — those
are the ones that silently produce wrong output on common templates rather than
erroring.
