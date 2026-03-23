# Persistent Memory for AI Tools

The flagship use case for Carry: a shared, private memory layer that multiple AI tools can read from and write to, eliminating session amnesia and cross-tool silos.

## The Problem

If you use more than one AI tool -- Cursor, Claude Code, ChatGPT, Copilot -- you've experienced this:

1. You explain your project conventions to Claude. Next session, it's forgotten.
2. You set up rules in Cursor via `.cursorrules`. Claude doesn't know about them.
3. You build a useful pattern in ChatGPT. There's no way to share it with your coding tools.
4. You copy-paste context between tools manually. It's fragile and tedious.

Each tool has its own memory silo. The workarounds -- markdown files, per-tool configs, copy-paste -- don't scale.

## How Carry Solves This

Carry provides a **single `.carry/` repository** that any tool can access:

```
Your AI Tools                   Carry Repository
                                 (.carry/)
  Cursor    ---read/write--->
  Claude    ---read/write--->    Shared Memory
  ChatGPT   ---read/write--->    (Dialog DB)
  Ollama    ---read/write--->
```

### Step 1: Initialize

```bash
carry init my-context
```

### Step 2: Add Your Context

Migrate existing context from tool-specific files:

```bash
# Assert your project conventions
carry assert com.me.conventions \
  language=TypeScript \
  style="functional, no classes" \
  testing="vitest, co-locate tests"

# Assert your preferences
carry assert com.me.preferences \
  tone="direct, no preamble" \
  format="structured markdown" \
  english_variant=british
```

Or assert from a YAML file for more complex context:

```yaml
# context.yaml
_:
  com.me.project:
    name: "My App"
    description: "A local-first task manager"
    stack: "Rust + TypeScript + Leptos"
    conventions: "Prefer composition over inheritance"

_:
  com.me.rules:
    rule: "Always write tests for new functions"
    rule: "Use Result types, not exceptions"
    rule: "Document public APIs with examples"
```

```bash
carry assert context.yaml
```

### Step 3: Connect Your Tools

Carry can be exposed to any agentic AI tool with shell permissions and access to the Carry CLI.

Through agentic calls to Carry, your tools share the same context. Cursor knows your conventions. Claude knows your project structure. A new chat session can pick up with all the context built in the last one.

### Step 4: Let Tools Write Back

When an AI tool discovers something useful -- a pattern, a decision, a convention that emerged during a coding session -- it can write that back to Carry:

```bash
carry assert com.app.decisions \
  decision="Use SQLite for local storage" \
  date="2026-03-15" \
  reason="Simpler than PostgreSQL for single-user local-first"
```

These decisions are then available to every tool, creating a growing, shared knowledge base.

## What Makes This Different

### vs. `.cursorrules` / `CLAUDE.md`

These are static files that one tool reads. They can't be written to by the tool, can't be shared across tools, and have no structure or query capability.

Carry's data is structured, queryable, writable by any connected tool, and can evolve over time without manual maintenance.

### vs. Cloud Memory (Mem0, OpenAI Memory, Zep)

Cloud services lock your data in a vendor. You don't control where it lives, who can access it, or how it's used.

Carry stores everything locally. No account, no API keys for the storage layer, no data leaving your machine unless you explicitly sync.

### vs. Copy-Paste / Manual Memory

Manual approaches don't scale. They break when you forget, when the format changes, or when you switch tools.

Carry is persistent and structured. Once data is asserted, it stays until you retract it. The format is stable and machine-readable.

## Separation: Who Wrote What?

Per-claim provenance tracking (recording who asserted each individual claim and when) is a planned feature. In the meantime, you can use separate repositories for different tools or agents, keeping contributions isolated.

## Example: Developer Profile

A practical example of persistent AI context -- your developer profile:

```yaml
- the: carry.profile/name
  of: dev-profile
  is: "Jane Developer"

- the: carry.profile/preferred_language
  of: dev-profile
  is: "Rust"

- the: carry.profile/style
  of: dev-profile
  is: "Functional, minimal dependencies, explicit error handling"

- the: carry.profile/testing_preference
  of: dev-profile
  is: "Property-based tests where possible, integration tests for IO boundaries"

- the: carry.profile/communication_style
  of: dev-profile
  is: "Direct, conclusion first, flag uncertainty explicitly"
```

Every AI tool that reads from this repository knows your preferences from the first message.
