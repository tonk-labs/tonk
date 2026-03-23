# Carry

**A local-first semantic database for humans and machines.**

Carry is a CLI tool for working with [Dialog DB](./dialog.md) -- a local-first, semantic database designed for structured data that both people and machines can read, write, and reason about.

```
carry init my-project
carry assert com.app.person name=Alice age=28
carry query com.app.person name age
```

## What is Carry?

Carry gives you a **private, local-first data repository** to which both you and your tools can read and write. Data lives on your machine, not in a cloud service. Carry provides a shared, durable place for your data to live.

At its core, Carry stores data as **claims** -- simple statements in the form *(the X of Y is Z)* -- organized into domains and composable schemas. This structure is flexible enough to model anything from a personal profile to a recipe database, while remaining queryable and human-readable.

## Key Properties

- **Local-first.** Your data lives on your machine. No cloud service holds your memory. Sync is optional and on your terms.
- **Schema-on-read.** You don't need to design a schema before writing data. Define concepts when you need them, and Dialog interprets your data at query time.
- **Composable.** Attributes combine into concepts. Concepts combine into rules. Rules derive new knowledge from existing data. Each layer builds on the one below.

## Who is Carry for?

- **Developers using multiple AI tools** who are tired of re-explaining context across Claude Code, Cursor, and others.
- **Data modelers** who want a local and lightweight tool for defining and querying structured data without standing up a database server.
- **Anyone** who wants to own their data, inspect it, and carry it with them.

## What's in these docs?

| Section | What you'll find |
|---|---|
| [Getting Started](./getting-started.md) | Installation and first steps |
| [Philosophy](./philosophy.md) | Why Carry exists and what it believes |
| [Core Concepts](./concepts/overview.md) | Claims, entities, domains, and asserted notation |
| [Domain Modeling](./modeling/attributes.md) | Defining attributes, concepts, and rules |
| [CLI Reference](./cli/overview.md) | Every command, flag, and option |
| [Use Cases](./use-cases/persistent-memory.md) | Concrete examples of what to build with Carry |
| [Dialog DB](./dialog.md) | The database engine underneath |

## Status

> [!CAUTION]
> Carry is **version 0.1** and under active development. The CLI and data model are stabilizing, but expect rough edges. Dialog DB itself is experimental -- binary encoding and index construction may change between releases without a migration path. 

That said, the data you write is yours. If the tooling changes, your data doesn't disappear.
