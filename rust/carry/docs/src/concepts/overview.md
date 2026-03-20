# Core Concepts

Carry organizes data using a small set of composable primitives. Understanding these will help you make sense of everything else in the documentation.

## The Data Model at a Glance

```
Claim           The atomic unit: "the X of Y is Z"
  Entity        Anything with an identity (a DID)
  Relation      What kind of association: domain/name
  Value         The data: text, numbers, booleans, references to other entities

Domain          A namespace that groups related relations
Attribute       A relation with type and cardinality constraints
Concept         A reusable schema: a named group of attributes
Rule            Logic that derives new concepts from existing data
```

Everything in Carry reduces to **claims**. Schemas, concepts, and rules are themselves stored as claims. The system is self-describing.

## How the Pieces Fit Together

1. You **assert claims** -- facts like "the name of Alice is Alice" -- into a domain like `com.app.person`.

2. When patterns emerge, you define **attributes** that refine relations with types (`Text`, `UnsignedInteger`, etc.) and cardinality (`one` or `many`).

3. You compose attributes into **concepts** -- named schemas like "person" that group related attributes together.

4. You write **rules** that derive new concept instances from existing data, like "a safe meal is one where no attendee has an allergy to any ingredient."

5. All of this lives in a `.carry/` repository with its own cryptographic identity.

Each of these is explained in its own section:

- [Claims and Entities](./claims-and-entities.md)
- [Domains](./domains.md)
- [Asserted Notation](./asserted-notation.md)
