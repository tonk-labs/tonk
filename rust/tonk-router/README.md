# tonk-router

Bidirectional URL routing as a parser-combinator grammar: a `Route` both
**parses** a URL into named params and **formats** params back into a URL — one
definition, two directions, round-trip by construction (`format(parse(url)) ==
url`).

This is a Rust adaptation of [subroute](https://github.com/Gozala/subroute) (the
author's TypeScript type-safe routing library), which in turn draws on
[elm/parser](https://package.elm-lang.org/packages/elm/parser/latest/) and the
[type-safe routing of Spock](https://www.spock.li/2015/04/19/type-safe_routing.html).
Where `subroute` uses the type checker to keep param names and link formatting in
sync, this crate carries the same idea into Rust and into Tonk's data-driven
routing, where a route's pattern is data on a branch and its param types come
from the route model it fills.

## Why not a path router (matchit / leptos_router)

Tonk URLs need two things a conventional path router can't express:

- **Intra-segment params.** `/space/{space}/{entity}@{model}!{view}` binds three
  params in one URL segment, split on the literals `@` and `!`. The literals are
  just text between params — no special case (this is `subroute`'s
  `/calculator/${{a:int}}/+/${{b:int}}`, where `+` is a literal between two
  params).
- **Slash-tolerant params.** A `{model}` capturing `tonk/person` (a name with a
  `/`) works because a param chomps up to the next *fixed* literal, slashes
  included — its boundary is the next literal, not `/`.

## The two axes of a param

A param has two orthogonal properties:

- **Extent** (`Kind`) — how far it reads: `Segment` (one segment, no `/`), `Path`
  (slash-tolerant, up to the next literal), `Rest` (the whole tail). Intrinsic to
  the path grammar; lives in this crate.
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
use tonk_router::{Kind, Route, Term};

// Compile a pattern string into a Route.
let route = Route::parse_pattern("/space/{space}/{entity}@{model}!{view}").unwrap();

// Parse a URL into params.
let params = route.parse("/space/home/id:x@trip!tonk:view").unwrap();
assert_eq!(params.get("model"), Some("trip"));

// Format params back into a URL.
assert_eq!(route.format(&params).as_deref(), Ok("/space/home/id:x@trip!tonk:view"));

// A slash-tolerant param captures a namespaced model whole.
let route = Route::parse_pattern("/space/{space}/{model:path}").unwrap();
let params = route.parse("/space/home/tonk/person").unwrap();
assert_eq!(params.get("model"), Some("tonk/person"));
```

## Pattern syntax

- `{name}` — a `Segment` param (one segment, no `/`).
- `{name:path}` — a `Path` param (slash-tolerant).
- `{name:rest}` — a `Rest` param (the whole tail; must be last).
- everything else is literal text (`/`, segment labels, `@`/`!` delimiters).

Param *value types* (`entity`/`unsigned`/…) are not written in the pattern — they
come from the route model field each param binds to, supplied by the binding
layer.
