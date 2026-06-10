# Slide — share plan

Status: draft, pre-implementation
Branch: `feat/slide`
Last updated: 2026-05-08

> Historical note: the `<tonk-concept>` custom element used in the
> examples below has since been removed. `<tonk-display>` covers the
> rendering surface it provided; the examples are preserved as the
> design proposed at the time.

## Goal

The agent runs slide locally, generates one URL, the human pastes it into
a browser. The browser claims the invite, pulls from the configured
remote, and lands on a live page rendering the agent's data.

Two share targets, in order of demo-readiness:

1. **Concept share** — `slide share concept <name>` produces a URL that
   lands on tonk-ui's auto-render at `/space/<repo>/branch/main/concept/<name>`
   (PR #445). Phase-1 demo target.

2. **View share** — `slide share view <name>` produces a URL that lands
   on tonk-ui's iframe viewer at `/space/<repo>/branch/main/view/<entity>`
   (PR #444), serving the agent-authored HTML body via the host route's
   `(the=text/html, of=<entity>)` lookup.

## How `text/html` claims get written

No notation or schema extension needed. Dialog accepts `text/html` as a
valid attribute URI today (see `dialog-query/src/attribute/the.rs` —
domain may be all-lowercase with no dot required). The
`tonk-notation` parser's "head must contain a dot to be a claim" rule
is just a tiebreaker — it doesn't gate what URIs the analyser will
emit when reached through the concept layer.

So views are authored as a normal concept whose one field is bound to
the `text/html` attribute. The seed schema:

```yaml
attribute! html-body:
  description: "HTML body of a slide-authored view"
  the:         text/html
  as:          Text
  cardinality: many

concept! view:
  description: "An HTML view, served via the host route"
  with:
    body: .html-body
```

After that one-shot definition, asserting a view is a one-liner:

```yaml
view! my-task-list:
  body: "<tonk-concept source=\"task\">…</tonk-concept>"
```

That commits:
- `(text/html, <body-derived-entity>, "<tonk-concept …>")` — read by the
  host route.
- `(dialog.meta/name, <entity>, "my-task-list")` — bookmark.
- The concept-membership claims so `view ?v:` queries find it.

Re-asserting the same body is idempotent. Different body produces a new
entity; `.my-task-list` rebinds (cardinality-one on `dialog.meta/name`
retracts the old binding). Retraction is `view! my-task-list: _`.

### Where the seed schema lives

Two options, decide later when the demo is wiring up:

- **Option A.** Slide ships the seed as part of `slide schema` output
  for any new repo, but does not auto-write it on `slide init`. The
  agent is told (via the guide) "if you want HTML views, paste this
  block first."
- **Option B.** `slide init --views` writes the seed schema as part of
  bootstrap. One extra commit at init time; nothing to memorise.

Lean A for v0 — keeps `slide init` pure. Revisit if agents trip on
forgetting to seed.

## URL shape

The launcher URL extends the existing invite URL with two query
parameters that the join page consumes:

```
<ui-base>/join
  ?access=<base58-ucan-chain>
  [&remote=<access-service-url>]
  [&name=<suggested-local-name>]
  [&then=<path-suffix>]
  [#<base58-seed>]
```

`tonk-invite::Invite::to_url` produces the first two pieces; `name=`
and `then=` are appended by slide. Reasoning:

- `tonk-invite` should stay agnostic — it's about delegation, not
  post-claim navigation. The launcher parameters live in slide and in
  the tonk-ui join component; tonk-invite never sees them.
- `then=` is a path *suffix* under the recipient's `/space/<name>/`,
  not an absolute URL. Tonk-ui prefixes whatever local name the
  recipient ends up with (which can differ from `name=` when the
  recipient renames the space on the join form, or when they already
  had the inviter's subject mounted under another name and land in
  the AlreadyMember auto-claim path). For a concept share the suffix
  is `branch/main/concept/<name>`.
- Tonk-ui drops malformed `then=` values silently — anything starting
  with `/`, anything containing a URL scheme — so a malicious or
  off-target value can't redirect the recipient out of the joined
  space.

## Subcommand surface

```
slide views                                    # list view-shaped entities
slide concepts                                 # list named concepts on the meta branch
slide share concept <name>  [--ui-base <url>]  # push + invite + URL
slide share view   <name|entity> [--ui-base <url>]
```

Reasoning for each:

- **`slide views`** — read-only. Joins `view ?e: body: ?html` against
  `dialog.meta/name ?e: ?name`. Prints `<name>  <entity>  <body-bytes>`
  in three columns. No new write paths.

- **`slide concepts`** — read-only. Lists user-defined concepts on the
  meta branch by name. 90% overlap with `slide schema` already; this
  is a slimmer single-purpose listing.

- **`slide share concept <name>`** — pushes the local repo to its
  upstream (calls `slide::sync::push` if the upstream is set; errors if
  not), mints an audience-open invite over the local repo (calls
  `slide::invite::mint` with the configured remote), assembles a URL
  with `&then=branch/main/concept/<name>`, prints it.

- **`slide share view <name|entity>`** — same shape as `share concept`,
  with `&then=branch/main/view/<entity>`. Accepts a bookmark name
  (resolved through `dialog.meta/name`) or a raw entity URI.

`<name|entity>` resolution: if it parses as `did:key:…`, treat as URI;
otherwise look up the bookmark via `dialog.meta/name`.

## tonk-ui change

Small (~10 lines) change to the join component: after a successful
claim, read the `then` query parameter and navigate to it. If absent,
fall back to the existing post-claim destination (probably
`/space/<repo>/branch/main`). Lives in `rust/tonk-ui/src/components/launcher.rs`
or wherever the join handler now lives — confirm during implementation.

This is a separate PR against tonk-ui; slide can ship its share commands
before that PR lands, but the URL won't auto-navigate until both are in.

## Test approach

Three layers, all native:

1. **Seed-schema round trip** — assert the `attribute!` + `concept!`
   block, assert a `view! …: body: …`, query `view ?v: body: ?html`
   and the host-side `(text/html, ?of)`, verify the same entity and
   body come back. Validates that no notation/schema extension is
   needed.

2. **Share-URL composition** — given a fixture `view! my-list: …`,
   `slide share view my-list --ui-base https://example.test/join`
   prints a URL that:
   - starts with the base
   - carries `access=`, `remote=`, `then=/space/main/branch/main/view/<entity>`
   - has the open-audience seed in the fragment

3. **`slide views` / `slide concepts`** — listing tests against a
   small fixture that asserts two views and one bare concept; verify
   each command surfaces only the relevant rows.

The tonk-ui `&then=` change gets a separate Leptos test in tonk-ui's
existing harness.

## Open questions

1. **Seed schema location.** Option A vs B above. Default A; revisit
   if agents stumble.

2. **Path shape after claim.** `/space/<repo>/branch/main/concept/<name>` —
   confirm this is the live route after PR #445. The handoff lists it
   that way; we'll verify in the implementation.

3. **`slide share` without an upstream.** What happens if the local
   repo has no remote configured? Two choices: (a) refuse with a
   pointer to `slide remote add`, (b) silently mint an invite without
   `remote=` so the human can still claim, but pulls won't work
   without manual config. Lean (a) — the share story breaks without a
   remote, so failing fast is more honest.

4. **`text/html` value type.** We declare the attribute as `Text` here.
   If we later want raw bytes (PNGs, etc.), a sibling concept with a
   `Bytes`-typed attribute under e.g. `image/png` is the natural
   extension — same pattern, different field. Out of scope for v0.

## Phasing

- **Phase 1 — concept share.** `slide share concept <name>`,
  `slide concepts` (read), tonk-ui `&then=` PR. Demo target: the
  worked example from PR #445's description (a `person` concept
  rendered live via `/concept/person`).

- **Phase 2 — view share.** Seed-schema documentation in `slide guide`,
  `slide share view`, `slide views`. Demo target: agent asserts a
  `view!` whose body is a `<tonk-concept source="task">` template,
  human gets a live task list rendered through PR #444's iframe
  viewer with PR #445's auto-rendered concept inside it.

Phase 2 stacks on Phase 1: the view's body uses the `<tonk-concept>`
custom element, so the live-update story is the same. The view layer
is just "agent-authored chrome around an auto-rendered concept."
