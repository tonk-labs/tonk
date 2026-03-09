
```
                        ._.
                        |_|
            *-----------+-+-----------*
            |  .                   .  |
            |         [CARRY]         |
            |=========================|
            |  .                   .  |
            *-------------------------*
```


# Carry

**A local-first semantic database for humans and machines.**

Carry is a CLI tool for asserting, querying, and managing structured data in a local [Dialog DB](https://github.com/dialog-db/dialog-db) repository. Your data lives on your filesystem.

```bash
carry init my-project
carry assert com.app.task title="Write docs" status=todo
carry query com.app.task title status
```

> **Status:** v0.1, under active development. The CLI and data model are stabilizing, but expect rough edges. Dialog DB itself is experimental; binary encoding and index formats may change between releases without a migration path.



## Use case #1: Better persistent memory than your markdown files

- LLMs lose the thread between sessions. A shared persistent store means agents pick up where they left off without re-explanation.

- Markdown files work until an agent has to read 200 of them to answer one question. `carry query` returns only what matches, saving you time and tokens.

- Freeform documents drift. One file calls it `status`, another writes `state`. When an LLM writes back into prose the signal degrades with each pass. Carry enforces schema at write time so the data stays consistent regardless of which agent touched it last.

- Queries are auditable in a way file grepping isn't. You can see exactly what was pulled in, instead of trusting that the agent found the right files.

- The store is CRDT-based. Parallel agents reading and writing at the same time merge correctly without coordination.



## What it does

Carry stores data as **claims** (expressions of the form `the X of Y is Z`), indexed in a local prolly tree for efficient querying and future sync.

- **Local-first.** Everything lives in a `.carry/` directory on your machine.
- **Schema-on-read.** Assert data in any domain without defining a schema first. Add attributes and concepts later, no migrations required.
- **Composable.** Attributes combine into concepts. Concepts combine into rules. Rules derive new data from existing data at query time.
- **Human-readable.** Data is stored and queried as YAML. Query output is valid input.
- **Cryptographic identity.** Each repository space has an Ed25519 keypair. Entity DIDs are content-addressed (BLAKE3), so the same data always produces the same identity.



## Use cases

- **Persistent memory for AI agents** Claude Code and other tools share one queryable context across sessions; use it with a local LLM to keep all your information on your device.
- **Personal knowledge base** structured, queryable notes, contacts, research findings, reading lists, ...
- **Local and semantic database** for your heavy-duty data modeling needs without a database server.



## Installation

**Quick install (macOS / Linux):**

```bash
curl -fsSL https://raw.githubusercontent.com/tonk-labs/tonk/main/install.sh | sh
```

Installs `carry` to `/usr/local/bin` with shell completions for zsh, bash, and fish.

**From source (requires Rust):**

```bash
git clone https://github.com/tonk-labs/tonk.git
cd tonk
cargo build --release --package carry
# binary at target/release/carry
```

**Nix:**

```bash
nix build github:tonk-labs/tonk#carry
```



## Quick start

```bash
# Create a repository
carry init my-project

# Assert data (no schema needed)
carry assert com.app.task title="Write docs" status=todo
carry assert com.app.task title="Ship v0.1" status=in-progress

# Query it back
carry query com.app.task title status

# Filter by field value
carry query com.app.task status=todo title

# Update an existing entity
carry assert com.app.task this=<DID> status=done

# Retract a field
carry retract com.app.task this=<DID> status
```

**Define a schema for validation and named queries:**

```yaml
# schema.yaml
task:
  concept:
    description: A task to be completed
    with:
      title:
        description: Title of the task
        the: com.app.task/title
        as: Text
      status:
        description: Current status
        the: com.app.task/status
        as: [":todo", ":in-progress", ":done"]
```

```bash
carry assert schema.yaml
carry assert task title="Review PR" status=todo
carry query task
```

**Pipe between commands:**

```bash
carry query task --format triples | carry assert -
carry query task status=done --format triples | carry retract -
```



## Commands

| Command | Description |
| --- | --- |
| `carry init [LABEL]` | Create a new repository |
| `carry assert <TARGET> [FIELD=VALUE ...]` | Add or update data |
| `carry query <TARGET> [FIELD[=VALUE] ...]` | Query by domain or concept |
| `carry retract <TARGET> this=<DID> [FIELD ...]` | Remove data |
| `carry status` | Show repository |


## Documentation

Full documentation is in [`docs/`](./docs/src/), built with [mdBook](https://rust-lang.github.io/mdBook/).

| Section | Contents |
| --- | --- |
| [Getting Started](./docs/src/getting-started.md) | Installation and first steps |
| [Philosophy](./docs/src/philosophy.md) | Why Carry exists |
| [Core Concepts](./docs/src/concepts/overview.md) | Claims, entities, domains |
| [Domain Modeling](./docs/src/modeling/attributes.md) | Attributes, concepts, rules |
| [CLI Reference](./docs/src/cli/overview.md) | Every command and flag |
| [Use Cases](./docs/src/use-cases/persistent-memory.md) | Practical examples |
| [Dialog DB](./docs/src/dialog.md) | The database engine underneath |



*Built by [Tonk](https://tonk.xyz)*
