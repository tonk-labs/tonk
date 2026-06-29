# tonk-router

A small extension over [matchit](https://docs.rs/matchit): the same `{name}`
single-segment params and static literals, plus **multiple multi-segment (`{*name}`)
spans in one route** with literals between them. A `Route` is also bidirectional —
it both **parses** a URL into named params and **formats** params back into a URL,
round-trip by construction (`format(parse(url)) == url`).

matchit already has `{name}` (one segment) and `{*name}` (a multi-segment
catch-all) — but the catch-all may only appear ONCE, at the end. That can't
express Tonk's two needs:

- **Intra-segment params.** `/space/{space}/{*entity}@{*model}!{*view}` binds
  three spans in one URL segment, split on the literals `@` and `!`. Each span
  chomps up to the next literal; the literals are just text between params.
- **Slash-tolerant refs anywhere.** `{*model}` capturing `tonk/person` (a
  namespaced ref) or `{*entity}` capturing `did:key:…/x` works because a span's
  boundary is the next literal, not `/` — and several may appear in one route.

So this crate keeps matchit's shape and adds the one capability it lacks: any
number of `{*name}` spans, each bounded by the surrounding literals. The
combinator/round-trip machinery draws on [subroute](https://github.com/Gozala/subroute)
and [elm/parser](https://package.elm-lang.org/packages/elm/parser/latest/).

## The two axes of a param

A param has two orthogonal properties:

- **Extent** (`Kind`) — how far it reads: `Segment` (`{name}`, one segment, stops
  at `/`) or `Span` (`{*name}`, slash-tolerant up to the next literal; a terminal
  span takes the rest). Intrinsic to the path grammar; lives in this crate.
- **Type** (`Type` / `ValueType`) — what the captured text must be (`text`,
  `entity`, `unsigned`, …). The engine validates *through* the type during
  matching, so two structurally identical routes are told apart by param type: a
  `page` param typed `unsigned` accepts `42`, a `model` param typed `entity`
  accepts `tonk:person`. **Types are never written in the pattern** — a param's
  type comes from the route *model field* it fills (`as: entity` / `as: unsigned`
  / …). `parse_pattern` defaults every param to `text`; the binding layer then
  calls `Route::with_types` with a `name -> Type` lookup built from the model's
  field descriptors. So the engine stays dependency-free (it ships only `text`)
  and `entity`/`unsigned`/`float` validators are injected, not parsed from the
  URL grammar.

## Example

```rust
use tonk_router::Route;

// Multiple spans in one route, split on `@` and `!`.
let route = Route::parse_pattern("/space/{space}/{*entity}@{*model}!{*view}").unwrap();

// Parse a URL into params.
let params = route.parse("/space/home/id:x@trip!tonk:view").unwrap();
assert_eq!(params.get("model"), Some("trip"));

// Format params back into a URL.
assert_eq!(route.format(&params).as_deref(), Ok("/space/home/id:x@trip!tonk:view"));

// A span captures a namespaced ref whole (slashes included).
let route = Route::parse_pattern("/space/{space}/{*model}").unwrap();
let params = route.parse("/space/home/tonk/person").unwrap();
assert_eq!(params.get("model"), Some("tonk/person"));
```

## Pattern syntax

- `{name}` — a single-segment param (stops at `/`), like matchit's `{name}`.
- `{*name}` — a multi-segment span (slash-tolerant, up to the next literal or
  end). Unlike matchit, several may appear in one route with literals between
  them.
- everything else is literal text (`/`, segment labels, `@`/`!` delimiters).

Param *value types* (`entity`/`unsigned`/…) are not written in the pattern — they
come from the route model field each param binds to, supplied by the binding
layer.
