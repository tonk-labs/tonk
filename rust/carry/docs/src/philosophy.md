# Why Carry

## The Problem

People using AI tools run into three compounding issues:

**Session amnesia.** Tools don't remember across sessions. You re-explain your context every time you open a new chat or switch to a different app. Agents forget mid-conversation.

**Cross-tool silos.** Each app has its own memory. Nothing is shared. Cursor has `.cursorrules`, Claude has `CLAUDE.md`, ChatGPT has its own internal memory. Conventions, style guides, and project facts must be re-entered across every tool.

**Privacy and data control.** Any solution to the first two problems must respect that memories contain sensitive data. Users need to know where their data lives and who can access it. Cloud memory services fail here by design.

Current workarounds -- `.cursorrules`, Memory Bank markdown files, `CLAUDE.md`, copy-paste, per-tool configs -- are manual, fragile, and don't scale across tools or teams.

## Carry's Answer

Carry starts from a few beliefs:

### Your data should live where you put it

Carry stores everything on your filesystem in a `.carry/` directory. There's no server, no account, no cloud dependency. You can back it up however you like, and delete it by removing a directory.

Sync is optional. If you want it, you choose the remote -- your own bucket, a peer, or a Tonk relay.

### One memory, every tool

Instead of maintaining parallel copies of your context in every AI tool's proprietary format, Carry provides a single repository to which any tool can read and write. The same facts are available to Cursor, Claude, and anything else that can use the CLI.

### Human-readable means machine-readable

Carry presents your data as YAML or JSON -- [asserted notation](./concepts/asserted-notation.md). It looks like this:

```yaml
did:key:zAlice:
  com.app.person:
    name: Alice
    age: 28
```

There's no binary blob to decode, no proprietary format to reverse-engineer. If you can read YAML, you can read your data. If your AI tool can read YAML, it can read your data too. The same format is used for query output and data input, so piping between commands works naturally:

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

Each space has its own cryptographic identity, providing a foundation for trust in human-agent collaboration. By using separate spaces for different tools or agents, you can keep contributions isolated and merge them selectively.

As AI tools become more autonomous, knowing the provenance of data becomes essential. Per-claim attribution (tracking who made each individual claim and when) is a planned feature but not yet implemented. In the meantime, spaces provide coarse-grained separation of data sources.

## What Carry Is Not

- **Not a replacement for your AI tools.** Carry doesn't compete with Cursor, Claude, or ChatGPT. It gives them a shared, durable place to read and write context.
- **Not an AI model or chat UI.** Carry is the store and the protocol to access it.
- **Not mandatory cloud.** Local-only is a first-class path.
- **Not a full personal data lake** (yet). The focus is on structured, queryable data for AI/agent context, with room to grow.

## The Bigger Picture

Carry is built by [Tonk](https://tonk.xyz). The ideas that power Carry are general-purpose. Carry is one application of these ideas, focused on the specific problem of persistent memory for AI tools.

The long-term vision is broader: a world where your data is truly yours, where tools interoperate on your terms, and where the structure of your information is something you define and control, not something imposed by a vendor.
