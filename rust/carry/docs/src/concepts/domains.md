# Domains

A **domain** is a namespace for grouping related relations. Domains use reverse-DNS notation, similar to Java package names or Android app identifiers.

## Naming Convention

```
com.myapp.person
diy.cook
xyz.tonk.carry
```

Each relation is qualified by its domain:

```
com.app.person/name     -- the "name" relation in the "com.app.person" domain
com.app.person/age      -- the "age" relation in the same domain
diy.cook/quantity       -- the "quantity" relation in the "diy.cook" domain
```

## Reserved Domains

Domains starting with `dialog.` are reserved for Dialog DB internals:

| Domain | Purpose |
|---|---|
| `dialog.attribute` | Stores attribute identity fields (`/id`, `/type`, `/cardinality`) |
| `dialog.concept.with` | Stores required concept membership by field name |
| `dialog.concept.maybe` | Stores optional concept membership by field name |
| `dialog.meta` | Universal metadata: `/name` and `/description` for any entity |

Do not assert claims into `dialog.*` domains directly. Carry manages these when you use `carry assert attribute`, `carry assert concept`, etc.

The `xyz.tonk.carry` domain is used by Carry itself for repository-level metadata like labels and settings.

## Using Domains

### In Commands

When a target in `carry assert` or `carry query` contains a `.`, it's treated as a domain:

```bash
# Assert into a domain
carry assert com.app.person name=Alice age=28

# Query from a domain
carry query com.app.person name age
```

### In YAML Files

Domains appear as the second level in [asserted notation](./asserted-notation.md):

```yaml
did:key:zAlice:
  com.app.person:
    name: Alice
    age: 28
```

### Choosing Domain Names

Pick a domain name that represents your use case or organization:

- `com.mycompany.project` -- for company or project data
- `me.myname.notes` -- for personal data
- `diy.recipes` -- for hobby projects

The domain name itself has no special meaning to Carry beyond namespacing. Two claims with the same field name but different domains are completely independent.
