# Why Carry

## The Problem

Structured data tends to end up in one of two bad places: a cloud service you don't control, or ad-hoc files you can't query.

Cloud services are convenient but fragile. Your data lives on someone else's machine, under their terms, queryable only through their API. If they change the product or shut it down, your data is gone or locked in an export format. If the data is sensitive, you've handed it to a third party by design.

Ad-hoc files -- markdown notes, CSV exports, JSON dumps, per-tool config files like `.cursorrules` or `CLAUDE.md` -- give you local control but sacrifice structure. They're hard to query across, drift out of sync, can't be written to by the tools that read them, and don't scale as the amount of data grows.

Neither option is good if you want data that is **private, durable, structured, and accessible to the tools you use**.

## Carry's Answer

Carry starts from a few beliefs:

### Your data should live where you put it

Carry stores everything on your filesystem in a `.carry/` directory. There's no server, no account, no cloud dependency. You can back it up however you like, and delete it by removing a directory.

Sync is optional. If you want it, you choose the remote -- your own bucket, a peer, or a Tonk relay.

### One repository, every tool

Instead of maintaining parallel copies of your data in every tool's proprietary format, Carry provides a single repository to which any tool can read and write. The same facts are available to a CLI script, an AI coding assistant, a custom agent, or anything else that can speak YAML.

### Human-readable means machine-readable

Carry presents your data as YAML or JSON -- [asserted notation](./concepts/asserted-notation.md). It looks like this:

```yaml
did:key:zAlice:
  com.app.person:
    name: Alice
    age: 28
```

There's no binary blob to decode, no proprietary format to reverse-engineer. If you can read YAML, you can read your data. Any tool that can read YAML can read your data too. The same format is used for query output and data input, so piping between commands works naturally:

```bash
carry query person --format triples | carry assert -
```

### Structure should be earned, not imposed

Many databases force you to define a schema before you can write anything. Carry inverts this. You can start by asserting raw claims in any domain you like:

```bash
carry assert com.my.notes title="Meeting notes" date="2026-03-18"
```

Later, when patterns emerge, you can define [attributes](./modeling/attributes.md) and [concepts](./modeling/concepts.md) to give your data structure. Dialog DB interprets schemas at read time, not write time. This means your data model can evolve without migrations.

### Attribution matters

Each repository has its own cryptographic identity, providing a foundation for knowing who contributed what. By using separate repositories for different tools, agents, or collaborators, you can keep contributions isolated.

Knowing the provenance of data matters whether the source is a person, a script, or an AI agent. Per-claim attribution (tracking who made each individual claim and when) is a planned feature.

## What Carry Is Not

- **Not a replacement for your tools.** Carry doesn't compete with Cursor, Claude, Obsidian, or any application you use. It gives them a shared, durable place to read and write structured data.
- **Not an application layer.** Carry is the store and the protocol to access it. What you build on top is up to you.
- **Not mandatory cloud.** Local-only is a first-class path.
- **Not a high-throughput database.** Carry works well for hundreds to thousands of entities. It's not designed for large-scale analytics or concurrent multi-user writes (sync is still developing).

## The Bigger Picture

Carry is built by [Tonk](https://tonk.xyz). The long-term vision is a world where your data is truly yours, where tools interoperate on your terms, and where the structure of your information is something you define and control, not something imposed by a vendor.
