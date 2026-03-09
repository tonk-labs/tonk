# Quick Start

This guide walks you through creating a repository, defining a schema, asserting data, and querying it -- all in about five minutes.

## 1. Initialize a Repository

```bash
carry init my-project
```

Output:
```
Initialized my-project repository in /path/to/.carry/did:key:zAbc123
```

This creates a `.carry/` directory in your current working directory. Carry looks for a repository in the current directory first, then seeks through parent directories until it finds one. Repositories hold all the data created when using Carry.

## 2. Assert Some Data

Let's start with raw domain assertions -- no schema needed:

```bash
carry assert com.app.person name=Alice age=28
```

Output:
```
did:key:zNewEntity123
```

The output is the DID of the newly created entity. Every piece of data in Carry is an entity with a globally unique, content-derived identity.

Add another person:

```bash
carry assert com.app.person name=Bob age=35
```

## 3. Query Your Data

```bash
carry query com.app.person name age
```

Output:
```yaml
did:key:zAlice123:
  com.app.person:
    name: Alice
    age: 28

did:key:zBob456:
  com.app.person:
    name: Bob
    age: 35
```

Filter by a field value:

```bash
carry query com.app.person name="Alice" age
```

Output:
```yaml
did:key:zAlice123:
  com.app.person:
    name: Alice
    age: 28
```

## 4. Define a Schema

Raw domain assertions work fine, but attributes and concepts give you reusable, named structures. Let's define some:

```bash
# Define attributes
carry assert attribute @person-name \
  the=com.app.person/name as=Text cardinality=one \
  description="Name of a person"

carry assert attribute @person-age \
  the=com.app.person/age as=UnsignedInteger cardinality=one \
  description="Age of a person"

# Define a concept grouping those attributes
carry assert concept @person \
  description="A person" \
  with.name=person-name \
  with.age=person-age
```

Now you can query using the concept name instead of the domain:

```bash
carry query person
```

Output:
```yaml
did:key:zAlice123:
  person:
    name: Alice
    age: 28

did:key:zBob456:
  person:
    name: Bob
    age: 35
```

## 5. Use a YAML File

For anything beyond a few fields, YAML files are more convenient. Create a file called `schema.yaml`:

```yaml
task-title:
  attribute:
    description: Title of a task
    the: com.app.task/title
    as: Text
    cardinality: one

task-status:
  attribute:
    description: Current status
    the: com.app.task/status
    as: Text
    cardinality: one

task:
  concept:
    description: A task to be completed
    with:
      title: task-title
      status: task-status
```

Assert it:

```bash
carry assert schema.yaml
```

Then add data:

```bash
carry assert task title="Write docs" status=in-progress
carry assert task title="Ship v0.1" status=todo
```

Query:

```bash
carry query task
```

## 6. Update and Retract

Update an existing entity by passing `this=`:

```bash
carry assert task this=did:key:zTask1 status=done
```

Retract a field entirely:

```bash
carry retract task this=did:key:zTask1 status
```

## 7. Pipe Between Commands

Query output is valid input for assert and retract. This enables Unix-style composition:

```bash
# Copy data by piping query output back as assertions
carry query person --format triples | carry assert -

# Retract all matching data
carry query person name="Alice" --format triples | carry retract -
```

## Next Steps

- [Core Concepts](./concepts/overview.md) -- understand claims, entities, and domains
- [Domain Modeling](./modeling/attributes.md) -- define attributes, concepts, and rules
- [CLI Reference](./cli/overview.md) -- every command and flag
- Use Cases:
  - [Persistent Memory for AI Tools](./use-cases/persistent-memory.md) -- shared context across Cursor, Claude, and others
  - [Personal Knowledge Management](./use-cases/personal-knowledge.md) -- contacts, research notes, reading lists
  - [Structured Data Modeling](./use-cases/data-modeling.md) -- lightweight local database for any structured data
