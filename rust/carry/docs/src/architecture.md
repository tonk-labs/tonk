# Architecture

This page describes how Carry is built and how the pieces fit together.

## Repository Layout

A Carry repository lives in a `.carry/` directory:

```
project/
  .carry/
    @active                        # Plain text: DID of the active space
    did:key:zSpace1/
      credentials                  # 32-byte Ed25519 secret key (mode 0600)
      facts/                       # Dialog DB storage (prolly trees)
    did:key:zSpace2/
      credentials
      facts/
  src/
  ...
```

### Repo

A **repo** is any directory containing a `.carry/` subdirectory. Carry discovers repos by walking up from `$PWD` toward `$HOME`. The `--repo` flag or `CARRY_REPO` environment variable can override this.

### Space

A **space** is a subdirectory of `.carry/` named by its `did:key:z...` DID. Each space contains:

- **credentials**: A 32-byte Ed25519 private key. The corresponding public key determines the space's DID.
- **facts/**: Dialog DB's on-disk storage using prolly trees.

The active space is tracked in `.carry/@active` as a plain text DID.

## Cryptographic Identity

Every space has an Ed25519 keypair:

1. **Private key**: Stored in `credentials`. Used to sign claims and authenticate during sync.
2. **Public key**: Encoded as a `did:key:z...` using the multicodec Ed25519 prefix + base58-btc encoding. This is the space's identity.

Entity DIDs are generated from a BLAKE3 hash of the entity's content, used as an Ed25519 seed:

```
content fields -> sort -> BLAKE3 hash -> Ed25519 signing key -> public key -> did:key:z...
```

This makes entity identity **deterministic and content-addressed**: the same data always produces the same DID.

## Storage

Dialog DB stores data in [prolly trees](https://www.dolthub.com/blog/2024-03-08-prolly-trees/) -- a probabilistic data structure that enables efficient synchronization. Prolly trees are a variant of B-trees where split points are determined by content hashing rather than fixed sizes, making structural diffs between two trees efficient.

Claims are indexed in three operative indexes:

| Index | Lookup pattern | Use case |
|---|---|---|
| EAV | Entity -> Attribute -> Value | "What are all the claims about Alice?" |
| AVE | Attribute -> Value -> Entity | "Who has name = Alice?" |
| VAE | Value -> Attribute -> Entity | "What references this entity?" |

Each claim records a `Cause` link to its predecessor for basic provenance tracking. A full temporal index is planned but not yet implemented.

## Data Flow

### Assert

```
CLI input -> Parse target/fields -> Resolve concept (if applicable)
          -> Generate entity DID -> Create claims -> Write to Dialog DB
```

1. The target is parsed: domain (contains `.`) or concept (no `.`).
2. For concepts, the concept definition is loaded and fields are validated.
3. An entity DID is generated from the content (or `this=` is used for an existing entity).
4. Claims are constructed and written to the prolly tree indexes.

### Query

```
CLI input -> Parse target/fields -> Resolve concept (if applicable)
          -> Build selector -> Scan indexes -> Filter -> Format output
```

1. The target is resolved to a domain or concept.
2. For concepts, the concept definition determines which attributes to query.
3. An `ArtifactSelector` is built with appropriate filters.
4. The EAV and AVE indexes are scanned.
5. Results are filtered by any specified field values.
6. Output is formatted as YAML, JSON, or triples.

### Retract

```
CLI input -> Parse target/fields -> Find matching claims -> Retract from operative indexes
```

Retractions remove claims from the indexes (EAV/AVE/VAE). The original claim data still exists and can be queried for specifically, but won't show up in standard queries.

## Dependencies

| Component | Crate | Purpose |
|---|---|---|
| CLI framework | `clap` | Command parsing and help generation |
| Dialog DB | `dialog-query`, `dialog-artifacts` | Database engine |
| Tonk | `tonk-space` | Space management and filesystem backend |
| Crypto | `ed25519-dalek`, `blake3`, `bs58` | Key generation, hashing, encoding |
| Async runtime | `tokio` | Async I/O |
| Serialization | `serde_yaml`, `serde_json` | YAML and JSON parsing/formatting |

## Platform Support

Carry compiles for native targets (macOS, Linux). The crate is gated with `#[cfg(not(target_arch = "wasm32"))]` -- it compiles to an empty `main()` on wasm32 targets since the CLI requires filesystem access.

## Future: Sync

Carry's sync capabilities are being developed. The planned architecture:

- **Push/Pull**: `carry push` and `carry pull` synchronize with a configured upstream remote.
- **Invite/Join**: `carry invite` generates invite URLs with UCAN delegations. `carry join` configures an upstream from an invite URL.
- **Structural merge**: Dialog's prolly tree storage enables efficient structural diffs and deterministic merging of divergent replicas.

Sync uses UCAN (User Controlled Authorization Networks) for capability-based access control. Each invite delegates specific capabilities from the space owner to the invitee.
