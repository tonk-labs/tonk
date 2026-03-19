# Personal Knowledge Management

Carry can serve as a structured personal knowledge base -- a place to organize notes, expertise, contacts, and project information in a format that's both human-readable and machine-queryable.

## Why Not Just Use Markdown?

Markdown files work well for short, declarative content (rules, patterns, quick notes). But they fall short when:

- You want to **query across** your knowledge: "Which contacts work in data engineering?"
- You need **structure that's consistent**: Every person should have name, role, and context.
- You want your **AI tools to reason** over your data, not just read it.
- Your knowledge base **grows beyond** what fits in a single context window.

Carry's fact-based model lets you store structured data that's both inspectable in a text editor and queryable via the CLI.

## Example: Personal CRM

### Define the Schema

```yaml
contacts:
  Person:
    description: A person in my network
    with:
      name:
        description: Full name
        as: Text
      role:
        description: Their role or title
        as: Text
    maybe:
      company:
        description: Where they work
        as: Text
      context:
        description: How I know them
        as: Text
      last_contact:
        description: When we last spoke
        as: Text
      notes:
        description: Freeform notes
        as: Text
```

### Add Data

```bash
carry assert schema.yaml
carry assert person name="Alex Good" role="Core maintainer, Automerge" \
  context="Collaborated on WASM implementation" \
  last_contact="2026-02"

carry assert person name="Peter Van Hardenburg" role="Researcher, Ink & Switch" \
  context="Shared trail-runner project" \
  notes="Interested in local-first tools for thought"
```

### Query

```bash
# Find everyone I know at a company
carry query person company="Ink & Switch"

# List all contacts with their roles
carry query person name role

# Find people I haven't talked to recently
carry query person name last_contact
```

## Example: Research Notes

Track research topics, findings, and open questions:

```yaml
research:
  Topic:
    description: A research topic I'm exploring
    with:
      title:
        description: Topic name
        as: Text
      status:
        description: Current status
        as: [":active", ":paused", ":completed"]
    maybe:
      summary:
        description: Current understanding
        as: Text
      open_questions:
        description: Unresolved questions
        as: Text
        cardinality: many

  Finding:
    description: A specific finding or insight
    with:
      topic:
        description: Related topic
      claim:
        description: The finding
        as: Text
      confidence:
        description: How confident I am
        as: [":high", ":medium", ":low", ":speculative"]
    maybe:
      source:
        description: Where this came from
        as: Text
```

Usage:

```bash
carry assert schema.yaml

carry assert topic @crdt-sync \
  title="CRDT Synchronization" \
  status=active \
  summary="Exploring efficient sync for local-first databases"

carry assert finding \
  topic=crdt-sync \
  claim="Prolly trees enable efficient diff-based sync" \
  confidence=high \
  source="Dialog DB implementation"
```

## Example: Reading List

```bash
carry assert com.me.reading \
  title="Designing Data-Intensive Applications" \
  author="Martin Kleppmann" \
  status=finished \
  rating=5 \
  takeaway="Replication and partitioning fundamentals"

carry assert com.me.reading \
  title="A Philosophy of Software Design" \
  author="John Ousterhout" \
  status=in-progress

# What have I finished reading?
carry query com.me.reading title author status=finished
```

## Advantages Over PKM Tools

| | Obsidian/Notion/Roam | Carry |
|---|---|---|
| Data format | Proprietary or semi-structured markdown | Structured YAML claims |
| Query language | Limited (Dataview, formulas) | Full EAV queries + rules |
| AI access | Plugin-dependent | Direct via CLI |
| Schema | Informal, drifts over time | Explicit, validated |
| Offline | Varies | Always (local-first) |

Carry doesn't replace your PKM tool for long-form writing and linking. It complements it by providing a structured, queryable layer for the data that benefits from consistency and machine access.
