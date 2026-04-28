# Transact route — design notes

`POST /api/repository/{repo}/branch/{branch}/transact` accepts an
asserted-notation document (JSON or YAML) and commits all
derived facts atomically. The handler is a thin glue layer over
`tonk-notation` and `tonk-schema`.

## Pipeline

```
body bytes
   │  Content-Type
   ▼
parse / parse_json   ──→  Parsed { syntax, diagnostics }
   │
   ▼
syntax (tonk_notation::Syntax)
   │
   ▼ open(branch)
   │
BranchResolver  ──→  interpret(syntax, resolver)
   │
   ▼ Vec<Claim>
   │
branch.transaction().assert(claim).commit()
```

Parse errors surface as `400 Bad Request` carrying the joined
diagnostic messages. Anything past parse is a `500` if it fails
inside dialog or a `400` if the interpreter rejected the
document semantically (anonymous subject, unknown bookmark,
non-attribute reference, …).

## BranchResolver

Implements `tonk_schema::interpret::Resolver` over an open
`Branch`:

- **`resolve_attribute(name)`** — `Named` query for an entity
  with `dialog.meta/name = <name>`, then `AttributeFacts`
  against the resolved entity, then descriptor reconstruction.
- **`resolve_attribute_by_entity(entity)`** — same fact-set
  query, skipping the name lookup. Used when a concept's
  `with` value is a `the:…` URI literal.

Both go through the same `fetch_attribute(entity)` helper —
single source of truth for descriptor reconstruction from
branch facts.

## Why parse → interpret → commit (not parse → commit)

Earlier revisions inlined claim emission into the parser. Two
problems:

1. **Bookmark resolution needed branch I/O**, but the parser
   was sync. Pushing the I/O into the parser made it async,
   which spread to every consumer (the language server doesn't
   want a branch handle to underline a syntax error).
2. **The same logic was duplicated** between the route (which
   asserts) and a hypothetical LSP completion backend (which
   would query attributes by name to offer hover info). With
   the split, the LSP can call the same `Resolver` trait
   against the same branch and get back a `ResolvedAttribute`.

Splitting parse and interpret keeps each concern testable
in isolation: the parser doesn't need a branch, the interpreter
doesn't need to know the surface syntax.

## Async-trait dance

`Resolver` uses `#[cfg_attr(not(target_arch = "wasm32"), async_trait)]`
and the `?Send` variant on wasm. axum's handler bound needs
`Send` on native, but the actual runtime is `wasm_bindgen_futures`
where things aren't `Send`. The cfg_attr split is what dialog
itself does for `Source` / `Provider` traits.

## Bookmark uniqueness

Currently we write `dialog.meta/name` as `cardinality: one` per
entity, which means asserting the same name twice on the same
entity overwrites. It does **not** retract the name from any
*other* entity that previously held it.

So if `person-name` initially points at `the:abc…` and a later
transaction redefines `person-name` (changed cardinality, say,
producing a new `the:def…`), both entities end up carrying the
name claim. The branch resolver returns the first hit — order
isn't deterministic.

The cleanup-on-rename story is unimplemented. For now: don't
redefine attributes through the same bookmark. If you need to,
it's a follow-up to add a "find any other entity carrying this
name and retract" pass before the assert.
