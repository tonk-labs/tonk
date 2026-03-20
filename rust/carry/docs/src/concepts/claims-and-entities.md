# Claims and Entities

## Claims

A **claim** is the atomic unit of data in Carry. Every claim is a triple:

```
(the: relation, of: entity, is: value)
```

For example:

```yaml
- the: com.app.person/name
  of:  did:key:zAlice
  is:  Alice
```

This says: "the `com.app.person/name` of the entity `did:key:zAlice` is `Alice`."

When you retract a claim, it is removed from the indexes and no longer appears in query results. Each claim records a `Cause` link to its predecessor for basic provenance. A full temporal index preserving complete assertion/retraction history is planned but not yet implemented.

### Claim Components

| Component | What it is | Example |
|---|---|---|
| `the` | The relation -- identifies the kind of association. Composed of `domain/name`. | `com.app.person/name` |
| `of` | The entity this claim is about. Always a DID. | `did:key:zAlice` |
| `is` | The value being associated. Can be a scalar or a reference to another entity. | `Alice` |

## Entities

An **entity** is anything with an identity. In Carry, entities are identified by [DIDs](https://www.w3.org/TR/did-core/) (Decentralized Identifiers) in `did:key:z...` format.

Entities are not defined explicitly -- they come into existence when claims are asserted about them. An entity is simply the set of claims that reference it.

### Entity Identity

When you create a new entity via `carry assert`, its DID is derived **deterministically** from its content:

1. The field values are sorted and hashed with BLAKE3.
2. The hash is used as an Ed25519 signing key seed.
3. The public key is encoded as a `did:key:z...`.

This means that asserting the same data twice produces the same entity DID. Identity is content-derived, not randomly assigned.

```bash
# Both produce the same entity DID because the content is identical:
carry assert com.app.person name=Alice age=28
carry assert com.app.person name=Alice age=28
```

### Explicit Entity Targeting

You can target an existing entity with `this=`:

```bash
# Update Alice's age
carry assert com.app.person this=did:key:zAlice age=29
```

### Named Entities (Bookmarks)

Entities can be given human-readable names using the `@name` syntax:

```bash
carry assert attribute @person-name \
  the=com.app.person/name as=Text cardinality=one
```

The `@person-name` asserts `dialog.meta/name` on the entity, creating a bookmark. You can then reference this entity by name in other commands:

```bash
carry assert concept @person with.name=person-name
```

Names are shared across the repository and travel with synced data.

## Value Types

Claims support the following value types:

| Type | Description | Example |
|---|---|---|
| `Text` | UTF-8 string | `"Alice"` |
| `UnsignedInteger` | Non-negative integer | `28` |
| `SignedInteger` | Signed integer | `-5` |
| `Float` | Floating-point number | `3.14` |
| `Boolean` | True or false | `true` |
| `Symbol` | A namespaced symbol | `carry.profile/work` |
| `Entity` | Reference to another entity (a DID) | `did:key:zAlice` |
| `Bytes` | Raw bytes | *(binary data)* |

When asserting values via the CLI, Carry auto-detects the type: DIDs are recognized as entity references, numbers as integers or floats, `true`/`false` as booleans, and everything else as strings.
