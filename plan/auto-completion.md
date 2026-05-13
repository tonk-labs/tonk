# Auto-completion

LSP completion + hover for `tonk-language-server`. Audience: anyone
editing the LSP, the worker side that mounts it, or the editor
client that consumes it.

## Why

Editing carry-asserted notation is unguided otherwise — you have to
remember every concept on the branch, every field a concept exposes,
and every variable already introduced in the document. Completion
and hover turn the editor into a discoverable surface without a
docs round-trip.

## Status

Shipped:

- Concept-name completion in head position — built-ins + branch
  concepts.
- Field-name completion in body position — `descriptor.with()` plus
  the `this:` meta-key.
- Variable completion after `?` — document-local, prior occurrences
  only.
- Hover on concept names (description + field list) and on body
  field names (qualified attribute, type, cardinality, description).

Deferred:

- **Reference completion at value position** (the bare-symbol
  `name: person-name` form). Spec'd below; not wired. The trigger
  decision (no `:`, only `Ctrl+Space` + symbol-prefix) still holds.
- **`..:` rest-retraction** inside `head!:` bodies. Reserved field;
  no completion entry yet.
- **`descriptor.maybe()`** optional fields. Reserved on the
  descriptor; nothing to surface yet.

## Trigger model

Two trigger characters advertised: `\n` (fresh line — head or field
depending on indent) and `?` (variable). `:` is intentionally
**not** a trigger because value position is too varied (variable,
literal, or reference) to auto-fire usefully.

`Ctrl+Space` works at any position and dispatches by parser-derived
cursor location — triggers are convenience, not contract.

## Sources

The set returned depends on **where the cursor sits in the parse
tree**, not which trigger fired.

### 1. Concept names — head position

Cursor at column zero on a fresh line. Returns:

- Built-ins from `tonk_schema::builtin::concept_registry()`.
- Branch-published concepts via `BranchIntrospection::list_concepts`
  ∩ `list_named_entities` (only published names — the user types
  the name, not the URI). Built-ins win on collision (matches
  analyzer resolution order).

Each item: bare name as `label`/`insertText`, descriptor description
as doc, `CompletionItemKind::Class`. The user adds `!` themselves
to switch query → assertion form.

### 2. Field names — body position

Cursor inside an indented body. Reads the enclosing head, resolves
its descriptor (built-in registry first, then introspection),
returns every entry in `descriptor.with()` plus `this:`.

Each item: `label = field`, `insertText = "field: "` (cursor lands
at value position), backing attribute description as doc,
`CompletionItemKind::Field`. Resolution failures (unknown head)
return no suggestions — falls through to free-form typing rather
than a stale list.

### 3. Variables — `?` position

Cursor immediately after `?`. Walks the current document for
`?<name>` and `&<name>` occurrences strictly *before* the cursor
position. Branch-free — no resolver call. The active in-flight
token (the variable being typed) is dropped from its own
suggestion list.

Each item: bare name without `?` (already typed), `insertText =
name`, `CompletionItemKind::Variable`.

Forward references (variables introduced *below* the cursor) are
intentionally excluded — the analyzer's join scope is linear, so
offering them would mislead.

### 4. References — value position [deferred]

Value position can carry a bare-symbol reference resolved through
the name table (`name: person-name`). Plan when wired:

- Surface only on `Ctrl+Space` or after the user types the first
  symbol-charset character — never on `:` itself.
- Source: `BranchIntrospection::list_named_entities` (every
  published name on the branch).
- Item shape: bare name as `label`/`insertText`, no `?` prefix,
  `CompletionItemKind::Reference`.

Variables (3) and references (4) can both be valid at the same
position; the user disambiguates by typing `?` (variable) vs any
other symbol char (reference).

## Hover

`textDocument/hover` resolves the identifier under the cursor and
dispatches by indent — head vs body — same way completion does.

- **Head identifier**: lookup via built-in registry first, then
  `BranchIntrospection::lookup_concept`. Renders concept name,
  description, and field list with type / cardinality / per-field
  description.
- **Body identifier**: walks the enclosing concept's `with()` set,
  finds the matching attribute, renders qualified name
  (`domain/name`), serialized type, cardinality, description.
- **Off-identifier**: returns `null` (no error, no tooltip).

Body output is Markdown. Type renders via `serde_json` (the type
enum has no `Display`); same shape the user sees in error
messages.

## URI → branch routing

Cell URIs follow the existing convention from
`rust/tonk-ui/src/components/space.rs:editor_source`:

```
tonk-buffer:///<repo>/<branch>/<cell-suffix>
```

Built-in concept and field completions are URI-independent. For
branch sources the LSP delegates to an `IntrospectionFactory` the
host injects via `Server::with_introspection`. The worker's
`ReactorIntrospectionFactory` parses the URI back to `(repo,
branch)` and acquires the reactor session on demand. When the URI
doesn't parse, or no factory is plugged in, the LSP falls through
to built-ins + document-local sources only — tests run on that
path.

The factory contract lives in `tonk-introspect` (no dialog deps)
and is implemented by the worker. The LSP itself never touches
dialog directly.

## What we don't do

- **Snippet expansion** beyond `field: ` — no `person!:\n  name:\n
  age:\n` skeletons. Reserved for a future code-action surface.
- **Forward variable references** — see source (3).
- **Server-side fuzzy ranking** — LSP returns the full candidate
  set; the editor filters and ranks. Keeps each client free to
  tune its own UX.
- **Two-phase `completionItem/resolve`** — full item ships on the
  first response. Becomes worth doing once doc payloads grow.

## Capabilities

```rust
completion_provider: Some(CompletionOptions {
    trigger_characters: Some(vec!["\n".into(), "?".into()]),
    resolve_provider: Some(false),
    ..
}),
hover_provider: Some(HoverProviderCapability::Simple(true)),
```
