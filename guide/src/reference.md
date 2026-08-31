# Reference

A quick lookup for the notation. For the full reference, run `tonk guide`.

## Heads

A head ending in `!` writes; without `!` it queries.

| Head            | Effect                                          |
|-----------------|-------------------------------------------------|
| `concept!:`     | Define a concept (a named set of relations).    |
| `command!:`     | Define a command (a transient, event-fed concept). |
| `rule!:`        | Define a rule (a deduction).                     |
| `view!:`        | Define a model's views: `this:` is the model, `show:` its templates keyed by facet (`ui`, `directory`, `label`, …). |
| `<concept>!:`   | Assert an instance of a concept.                |
| `<concept>:`    | Query instances of a concept.                   |

## Body keys

| Key     | Meaning                                                          |
|---------|-----------------------------------------------------------------|
| `this:` | The entity to operate on: omitted (derived from content), `?v` (variable), bare name, or `did:key:…`. |
| `with:` | In `concept!`/`command!`, the field-name to attribute map.       |
| `when:` | In `rule!`, the list of premises that must hold.                |
| `..: _` | Retract every attribute the concept selects that is not named.   |

## Field values

| Value             | Meaning                                       |
|-------------------|-----------------------------------------------|
| `"Alice"`         | String literal.                               |
| `28`, `1.5`       | Number literal.                               |
| `?name`           | Variable (binds, and joins across premises).  |
| `_`               | Blank: query matches any; assertion retracts. |
| `person` (bare)   | Symbol, resolved through the name table.       |
| `id:foo`, `did:key:…` | URI, used directly.                       |
| `blob:<hash>`     | Content-addressed reference to bytes stored with `tonk blob add`. |

## Attribute fields

| Field         | Values                                         |
|---------------|------------------------------------------------|
| `the`         | `domain/name` URI, the attribute's identity.   |
| `as`          | `text`, `entity`, `unsigned-integer`, `float`. |
| `cardinality` | `one` (default) or `many`.                     |
| `description` | Required.                                      |

## `<tonk-display>`

| Attribute | Meaning                                                       |
|-----------|--------------------------------------------------------------|
| `model`   | The concept to render. Required.                             |
| `entity`  | The single entity to render. Absent renders every instance.  |
| `view`    | The `show` facet to render (`label`, `title`, …). Omitted uses `ui` (entity set) or `directory`. |

## Template placeholders

| Form        | Meaning                                          |
|-------------|--------------------------------------------------|
| `{field}`   | A field of the rendered entity.                  |
| `{this}`    | The entity's own id.                             |

## Command sources

Read from the DOM event with the `the:` of a command field.

| URI                                            | Reads                              |
|------------------------------------------------|------------------------------------|
| `dom.event.current-target/value`               | the target's `value`               |
| `dom.event.current-target.dataset/foo`         | the target's `data-foo`            |
| `dom.event.detail/foo`                          | a custom event's `detail.foo`      |

## Blobs

Binary data (images, files, anything that isn't a fact) lives outside
the associative memory, referenced from it by a content-addressed
`blob:<hash>` URI.

| Command                 | Does                                                    |
|--------------------------|---------------------------------------------------------|
| `tonk blob add <file>`   | Ingest a file, print its `blob:<hash>` reference.        |
| `tonk blob cat <blob:hash>` | Write a blob's bytes to stdout.                       |
| `tonk blob ls`           | List the branch's blobs from their metadata facts: reference, content type, name. |

The reference is just another URI value, so assert it onto any
concept like `id:` or `did:key:…`:

```yaml
photo!:
  this: id:vacation
  image: blob:5Pj4ZaADcKEv2D7udVLP44edvv2ZbTkEpqZdMdBRZkmt
```

Blobs sync with the rest of the branch: `tonk push`/`tonk pull` carry
both the facts and the bytes they reference.

In the web frontend, an image blob renders inline: embed

```
<tonk-display with="{branch}@{repo}" entity=blob:<hash> model="tonk:blob" />
```

in a view and the display mounts an `<img>` served from
`/api/repository/{repo}/branch/{branch}/blob/{entity}` with the blob's
recorded content type.

## CLI

| Command        | Does                                            |
|----------------|-------------------------------------------------|
| `tonk guide`  | Print the full notation reference.              |
| `tonk schema` | List the concepts on the current branch.        |
| `tonk join …` | Join a space from an invite link.               |
