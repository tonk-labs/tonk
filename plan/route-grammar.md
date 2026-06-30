# Data-driven `route!` grammar (route.flow-style combinator engine)

Goal: make a `route!`'s `path` string the single, authoritative grammar for a
URL — including sub-segment structure like `{entity}@{model}!{view}` and
slash-containing model names like `tonk/person` — and hide the implementation
(matchit and any secondary parsing) behind that notation. Routes stay data on a
branch; the SW matches against the route table, captures named params, stamps
them on the tab's `tonk:site`, and the matched route model's view reads them.

This supersedes the catch-all + `display.rs` hand-parser. matchit's Level-1 use
is retired; `leptos_router` is deleted at the end.

## Why a combinator engine (not matchit, not regex, not query params)

The two hard requirements no path router gives us:

1. **Intra-segment params** — `{entity}@{model}!{view}` is three params in one
   URL segment split on `@`/`!`. matchit only splits on `/`.
2. **Slash-containing params** — `tonk/person` as a `{model}` spans two matchit
   segments; `{model}` truncates at the first `/`, only `{*model}` (catch-all,
   must be last, swallows everything) grabs it.

Options weighed:

- **Pure matchit + query params** (`/space/{space}/view?model=tonk/person&…`):
  sidesteps both, but ugly/verbose URLs, and `route!` STILL doesn't express the
  grammar — the query parse lives outside the pattern. Rejected.
- **Regex-per-route compiler**: works but one-directional, stringly-typed, ad-hoc
  — strictly worse than prior art. Rejected.
- **route.flow-style combinator engine**: the only option that solves
  `tonk/person` AND gives a bidirectional, typed grammar. The author already
  designed and validated this model in JS
  (https://github.com/Gozala/route.flow,
  inspired by https://package.elm-lang.org/packages/elm/parser/latest/).
  `@`/`!` are just literal joiners between params (route.flow's calculator
  example: `/calculator/{float}+{float}` — `+` is a literal between two params,
  identical shape to `@`/`!`). A `String` param consumes greedily up to the next
  fixed delimiter, slashes included — that is what makes `tonk/person` work.

## Decisions (locked)

- **3b — string DSL in YAML.** `route!` carries the literal pattern string
  (`/space/{space}/{entity}@{model}!{view}`); a load-time compiler turns it into
  the combinator chain. Fully data-driven end to end.
- **Build the full grammar first.** Port the combinator engine and express ALL
  route shapes (directory + artifact + ad-hoc) on it before landing the cutover —
  not a minimal directory-only matcher first.
- **Bidirectional engine.** Port BOTH halves (parse URL→params and format
  params→URL) up front, so views can generate route URLs from data (breadcrumbs,
  cards, share links) — matching route.flow's full design.
- **Param facts shape: either per-route-model typed fields (1) OR generic bag
  (3).** Sub-decision, resolved during Stage 3 once the engine exists, by
  whichever is cleaner to wire. (1) = each route model concept declares its param
  fields (`tonk:artifact/route` has `entity`+`model`+`view` `site/*` fields) and
  the matcher binds by name; (3) = `site/param {name,value}` rows read by name.
  NOT the fixed-vocabulary option.

## Route shapes to express (the target `route!` table)

The exact `/space` routes today (Level 0 splits `/space/{space}/{rest}`; these
are the Level-1 `{rest}` shapes):

| URL shape (`rest`) | params | renders | usage |
|---|---|---|---|
| `/` (bare) | — | `tonk:space/route` → binder/workspace | the migrated default |
| `/{model}` (incl. `tonk/person`) | `model` | `<tonk-display model={model}>` directory | **frequent: `/inspector`, `/diagnose`, `/tonk/person`** |
| `/{entity}@{model}` | `entity`,`model` | `<tonk-display entity model>` artifact | rare, typed/shared URL |
| `/{entity}@{model}!{view}` | `entity`,`model`,`view` | `+ view` ad-hoc | rare |

Notes carried from the current `display.rs` that the new grammar must preserve
or consciously drop:
- **Name resolution**: a bare `{entity}` (no `:`) is a bookmark name resolved via
  the branch `Name` index (`id:{name}` → `dialog.name/referent`). Decide whether
  this stays (a step in the route model / a resolved fact) or is dropped (URLs
  must carry real URIs).
- **404**: an unresolved name renders `.not-found`. Becomes a `tonk:site`
  reactive state (the `display-reactive-states` design governs no-model-resolved).
- Only in-app link in the whole library is `href="/"`; nav is full-page
  (`location.assign` via the SW `navigate` message). No SPA routing to preserve.

## Stages (each ends with a booting app + green tests)

1. **DONE — combinator engine as crate `rust/tonk-router`** (NOT tonk-schema;
   dependency-free). A Rust adaptation of https://github.com/Gozala/subroute
   (credited in README + lib.rs). `Route` = ordered `Vec<Term>`
   (`Term::Text`/`Term::Param`); bidirectional `parse`(url→`Params`) +
   `format`(`Params`→url), round-trip tested. **Two-axes param design** solves
   `tonk/space`:
   - EXTENT (`Kind`: `Segment`/`Path`/`Rest`) — URL-grammar property, written in
     pattern as `{name}`/`{name:path}`/`{name:rest}`. `tonk/person` → `{model:path}`.
   - TYPE (`Type`/`ValueType`: text/entity/unsigned/…) — owned by the route MODEL
     field, NEVER in the pattern. `parse_pattern` defaults params to `text`; the
     binding layer injects via `Route::with_types(name->Type)`. Types participate
     in matching (`ParseError::InvalidType` lets two structurally-identical routes
     differ by type — `{page}:unsigned` vs `{model}:entity`). Engine ships only
     `text`, stays dep-free; binding layer plugs in the rest via `ValueType`.
   25 tests; native + wasm32 clean; clippy/fmt gated.
2. **DONE (in crate) — pattern-string → route compiler** `Route::parse_pattern`.
   STILL TODO: the routing **table** (`oneOf` over many routes + specificity
   ordering static>param>greedy + furthest-progress error). Today matching is
   one-route-at-a-time; the table is built next (Stage 2b) before wiring the SW.
3. **DONE — param facts on `tonk:site`.** `match_route` (router/session.rs) now
   builds a `tonk_router::Router` from the `tonk:route` table, recognizes the
   Level-1 path, and returns captured params; `stamp_site` writes the fixed `Site`
   stamp PLUS each param as a `xyz.tonk.site/{name}` raw claim (via `RawClaim`,
   now `pub(crate)`). Storage = generic-bag (per-param `xyz.tonk.site/*` facts);
   consumption = typed-per-model (each route model declares typed fields reading
   those attrs). `matchit` dropped from tonk-worker.
   INTERIM: param value type is keyed by name in `site_param_claim` (`entity` →
   `Value::Entity`, `model`/`view` → `Value::String`) to match the route models'
   `as:` field types. TODO (descriptor-driven typing): have `match_route` query
   each route model's field descriptors and thread the `as:` types through
   `tonk_router::Route::with_types`, so the value type comes from the field and
   the name table in `site_param_claim` goes away. (This also enables type-based
   route disambiguation, which the engine already supports.)
4. **DONE — route shapes in core.yaml** as `route!` entries + route models +
   views: `tonk:directory/route` (`/{model:path}` → `<tonk-display model>`),
   `tonk:artifact/route` (`/{entity}@{model:path}` → `entity`+`model`),
   `tonk:adhoc/route` (`/{entity}@{model:path}!{view}` → + `view`), plus the
   existing `tonk:space/route` (`/`). All four seeded into the route table.
   `display.rs` DELETED; `*subject` Leptos route now points at `TonkSpaceSealed`
   (which threads the full path incl. sub-route into the registered site), so
   board/inspector/diagnose/artifact all flow through the sealed concept-router.
   DROPPED for now: bookmark name-resolution + 404 that `display.rs` did — the
   `{entity}` field is `as: entity`, so URLs must carry real URIs (a bare name
   won't resolve). Re-add as a route-model step / `tonk:site` reactive state later
   (see name-resolution note above).
5. **Cut the SW router over** — DONE (matchit gone from tonk-worker; the engine
   matches Level 1). Level 0 (`/space/{space}`) stays the `parse_space`/
   `resolve_path` builtin in tonk-schema.
6. **Retire Leptos** — PARTIAL: `*subject` route + `display.rs` gone. STILL on
   Leptos: the `<Router>` shell itself, `/` (`TonkHub`/`<tonk-hub>`), `/join`
   (`TonkJoin`/`<tonk-join>`). Next: move `/` and `/join` to `route!` on the
   profile branch (drop the `RouteTarget::Profile` special-case in
   `space.rs`/`session.rs`), then delete `leptos_router` + the shims +
   `route_views.rs`.

## Remaining work (after this session)

1. **BROWSER-VERIFIED (2026-06-29, Chrome MCP).** The router stamps correctly for
   every shape — confirmed via overlay query on a real space:
   - `/inspector` → concept `tonk:directory/route`, model `inspector`
   - `/diagnose` → model `diagnose`
   - `/tonk/person` → model `tonk/person` (slash-tolerant ✓)
   - `/id:demo@trip` → concept `tonk:artifact/route`, entity `id:demo`, model `trip`
   - `/id:demo@trip!tonk:view` → concept `tonk:adhoc/route`, +view `tonk:view`
   Two bugs were FOUND AND FIXED in the process (commit after this):
   - **Param facts accumulated instead of superseding** — `RawClaim` used
     `associate` (cardinality-many), so each navigation piled up a new
     `xyz.tonk.site/model` value and a cardinality-one read returned a stale one.
     Fixed: `RawClaim` gained a `unique` flag → `associate_unique` (Replace) for
     site params. Verified: after registering 3 paths, exactly 1 model value
     remains (the latest).
   - (process note) a wasm change without a `service_worker.js` change does NOT
     trigger a SW update — had to `unregister()` + double-reload to load the new
     worker. Relevant for any future SW browser-verify.

   **Per-model sealed-guest status (browser-checked 2026-06-29):**
   - `/board` and any normal `<tonk-display>` model — WORK fully.
   - `/diagnose` (`<tonk-tree>`) — repository-context FIXED (stamp `repo`/`branch`
     site facts + route view wraps in `<tonk-repository>`/`<tonk-branch>`; the tree
     header now renders). REMAINING: `<tonk-tree>` issues its OWN `window.fetch` to
     a repo endpoint instead of the bridge query channel, so under the guest's
     opaque origin it gets `Failed to fetch` — header shows, data doesn't. Needs
     tonk-tree to route data through the bridge like `<tonk-display>` does.
   - `/inspector` (`<tonk-inspector>`) — DE-LEPTOSED + REGISTERED. Extracted to a
     standalone leptos-free crate `rust/tonk-inspector` (plain-DOM notebook +
     HTML-string result rendering over engine-free serde mirror types; evaluates
     by POST to the branch `/evaluate` endpoint via the guest fetch proxy).
     `guest.rs` now registers it. REMAINING blocker: the `<tonk-code>` EDITOR
     bundle is not in the guest yet. tonk-code is a CODE-SPLIT CodeMirror bundle
     (`assets/tonk-code.js` 116K + `chunk-*.js` + per-language
     `tonk-code-lang-dialog-yaml.js` 158K) that loads chunks/language packs via
     RELATIVE dynamic `import("./…")` — dead at the guest's opaque origin. WA was
     fixed by esbuilding ONE self-contained ESM, but tonk-code REQUIRES
     `splitting: true` (single `@codemirror/state` identity, else "Unrecognized
     extension value"; see scripts/build.mjs), so it can't collapse to one file.
     FIX (designed, not built): in `tonk-portal build_inject_payload`, fetch ALL
     tonk-code bundles (main + chunks + dialog-yaml lang pack), transfer them, and
     in the guest bootstrap mint a blob URL per file and REWRITE each relative
     dynamic-import specifier to its blob URL (the same blob-rewrite the glue
     snippets already use, but for dynamic imports + a manifest of files). The LSP
     side already works over the bridge: the diagnostics provider uses
     `httpTransport` (HTTP `/api/language-server`, NOT WebSocket), which the guest
     `window.fetch` proxy routes. EDITOR INJECTION DONE + BROWSER-VERIFIED
     (2026-06-29): the portal fetches the whole code-split tonk-code bundle graph
     (main + chunks + dialog-yaml pack), transfers it, and the guest bootstrap
     mints a blob per file in dependency order, rewriting relative imports to blob
     URLs (the runtime language-pack URL → a `window.__tonkCodeLang` lookup). The
     CodeMirror editor now RENDERS and ACCEPTS INPUT inside the sealed iframe
     (snapshot: a real CM textbox with placeholder; typed `replica ?r:` registers).
     REMAINING (LSP/auto-eval): a pure query only runs via auto-eval, which is
     driven by the editor's `diagnostics` event, which needs the LSP. Two fixes
     applied: (1) the inspector mounts its own `<tonk-diagnostics-provider>` as the
     cell host; (2) tonk-code's provider `#whenWorkerReady` now races
     `navigator.serviceWorker.ready` against a 2s timeout (in the sealed guest
     `.ready` never resolves — no SW at an opaque origin — so it hung forever
     before building the LSP transport). STILL NOT WORKING: no `/api/language-server`
     request fires after typing, and an `Uncaught (in promise)` appears right after
     the inspector mounts. Likely either the `tonk-code-connect` → provider attach
     races the element upgrade order (both defined in one bundle; if `tonk-code`
     upgrades before the provider installs its listener the connect event is lost),
     or the LSP `initialize` rejects. NEXT: instrument inside the guest (the opaque
     iframe hides its DOM/console detail) — log in the provider's `#onConnect`/
     `#ensureClient` and the editor's `#announceConnect`; consider having the
     provider scan for existing `<tonk-code>` descendants on connect, or the
     inspector re-announce the editor after a frame (detach/reattach re-runs
     connectedCallback unconditionally).
   - Full `did:key:` URL form (`/space/did:key:z6Mk…/inspector`) parses fine —
     `resolve_path` reconstructs the same SpaceRef as the short form (verified). A
     "not working" full-DID URL means that space isn't in the current profile, not
     a parse bug.
2. **Descriptor-driven param typing** (Stage 3 TODO above) — replace the name
   table in `site_param_claim` with types read from the route model's field
   descriptors via `Route::with_types`; unlocks type-based disambiguation.
3. **Name-resolution + 404** — re-introduce the dropped `display.rs` behavior as a
   route-model concern (a `Name`-index lookup step + a `tonk:site` not-found
   reactive state) if bare bookmark names in URLs are still wanted.
4. **`/` and `/join` → `route!`** on the profile branch, then delete
   `leptos_router` (Stage 6).

## Current architecture (ground truth, verified 2026-06-28)

- `/space/:space` (bare) is migrated: `TonkSpaceSealed` → sealed opaque-origin
  iframe (`<tonk-portal runtime>`) → fixed `<tonk-display model=tonk:site
  data-tonk-entity=site>`. The SW (`POST /api/site` → `register_site` in
  `rust/tonk-worker/src/router/session.rs`) resolves Level 0 (`resolve_path` in
  `rust/tonk-schema/src/space.rs`), matches `rest` against the durable
  `tonk:route` table with `matchit` (`match_route`), stamps the matched `concept`
  on the tab's `site:<client-id>` entity (`Site` concept,
  `rust/tonk-schema/src/site.rs`; `tonk:site` view in core.yaml nests into the
  matched concept). THIS is the working concept-router seam.
- Still Leptos (`rust/tonk-ui/src/components/launcher.rs`): `/` → `TonkHub`
  (`<tonk-hub>`), `/join` → `TonkJoin` (`<tonk-join>`) — both in-page
  `<tonk-display>` directory views, NOT sealed; `/space/:space/*subject` →
  `TonkDisplayView` (`display.rs`) — in-page, the hand-parser for
  `{entity}@{model}!{view}` + name resolution + 404.
- Today's `tonk:route` is `{path, concept}` only — NO param capture.
  `match_route` discards `matched.params`.
- Only one `route!` exists in core.yaml: `/` → `tonk:space/route`.
- `leptos` / `leptos_router` / `leptos-use` still in `rust/tonk-ui/Cargo.toml`.

## Engine design notes (elm/parser → route.flow → Tonk)

Background: https://package.elm-lang.org/packages/elm/parser/latest/ (the
parser-combinator foundation route.flow builds on).

What to carry from **elm/parser**:
- **Keep-vs-ignore pipeline** (`|=` keep, `|.` ignore). The `point` example
  (`succeed Point |. symbol "(" |= float |. symbol "," |= float |. symbol ")"`)
  IS our `{entity}@{model}!{view}` shape: `@`/`!` are ignored `symbol`s, the
  params are kept. Confirms intra-segment params need no special-casing — `@`/`!`
  are just literals between params.
- **`chompWhile`/`getChompedString` = greedy-to-delimiter.** A `{model}` param is
  "chomp until the next fixed literal (`@`/`!`/`/`-end)", slashes included. THIS
  is the `tonk/person` solution: a param's boundary is the next literal in the
  chain, not `/`. The Stage-2 compiler reads `{entity}@{model}!{view}` and builds
  `param(entity, upto '@') . symbol('@') . param(model, upto '!') . symbol('!')
  . param(view, upto end)`.
- **Commitment & backtracking semantics** (semantics.md): once a parser commits
  (matches a literal), `oneOf` won't try later branches; the discipline is
  "parse committed prefixes first, then `oneOf` on the divergent tail; use
  `backtrackable` only where required." Maps to route-table SPECIFICITY ordering:
  literal-heavy patterns (`/board`) tried before param patterns (`/{model}`);
  `/{entity}@{model}` vs `/{model}` diverge at `@`, so that divergence needs
  backtracking OR a specificity sort that tries the more-delimited pattern first.
  Prefer restructuring over `backtrackable` (elm's guidance: speed + clearer
  "nothing matched" errors).
- **Context + position errors** (`inContext`): a near-miss route should report
  which pattern got furthest and why — a real authoring win over matchit's opaque
  no-match (catches `route!` pattern typos / almost-matching URLs).

What **route.flow** adds (the part NOT in elm/parser, and the reason for the
bidirectional decision):
- **Bidirectionality** — every `segment`/`param` carries BOTH a parse and a
  format fn, so `format(parse(url)) == url`. elm's `Url.Parser` is parse-only;
  route.flow makes it round-trip. This is the half to port now.
- **Typed params** — `String`/`Integer`/`Float`/custom; parse returns `null` on
  type mismatch. Our `kind` set starts at `String` (greedy-to-delimiter) + `Uri`;
  numeric/custom later.

Rust shape (Stage 1):
- `Route<Params>` = `{ parse: &str -> Option<(Params, rest)>, format: Params ->
  String }`. Params accumulate via a `.segment(lit).param(name, kind)` chain.
- Primitives: `segment(literal)` (ignore), `param(name, kind)` (keep, consumes to
  next literal), `root`/end.
- Table match = `oneOf` over all `route!` patterns, specificity-ordered, errors
  carry furthest-progress pattern.

## Deferred

- Live router (subscribe to `tonk:route`, rebuild matcher per frame). Currently
  matched per-request; correct but not live-without-a-request.
- Containment (tie each `/api` request to its Level-0 `(repo,branch)`).
