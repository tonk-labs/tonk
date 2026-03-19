# Rules

A **rule** derives new concept instances from existing data. Rules are Carry's mechanism for logic and inference -- they let you express constraints, joins, and derived views without writing application code.

Rules are evaluated at query time by the semantic layer, not stored as materialized views. They are declarative: you describe *what* should be true, and Dialog figures out *how* to compute it.

## Structure of a Rule

A rule has three parts:

| Part | Required | Description |
|---|---|---|
| `deduce` | Yes | The concept being derived -- what the rule produces |
| `when` | Yes | Positive premises -- conditions that must hold |
| `unless` | No | Negative premises -- conditions that must NOT hold |

## Example: Finding Allergy Conflicts

Given a cooking domain with recipes and ingredients, and a health domain with allergies, you can write a rule that finds conflicts:

```yaml
diy.planner:
  find-allergy-conflicts:
    description: Find conflicts between recipe ingredients and allergies
    deduce:
      AllergyConflict:
        person: ?person
        recipe: ?recipe
    when:
      - diy.cook/Recipe:
          this: ?recipe
          ingredient: ?ingredient
      - diy.cook/Ingredient:
          this: ?ingredient
          name: ?substance
      - diy.health/Allergy:
          this: ?this
          person: ?person
          substance: ?substance
    unless: []
```

This rule says: "An `AllergyConflict` exists when a recipe contains an ingredient whose name matches a substance someone is allergic to." The `?variables` unify across premises -- `?substance` must be the same value in both the `Ingredient` and `Allergy` matches.

## Example: Safe Meals

Building on the allergy conflict rule, you can define meals that respect dietary restrictions:

```yaml
diy.planner:
  respect-dietary-restrictions:
    description: A meal that respects dietary restrictions
    deduce:
      Meal:
        attendee: ?person
        recipe: ?recipe
        occasion: ?occasion
    when:
      - diy.planner/Meal:
          this: ?this
          attendee: ?person
          recipe: ?recipe
          occasion: ?occasion
    unless:
      - diy.planner/AllergyConflict:
          person: ?person
          recipe: ?recipe
```

This rule says: "A meal is safe if it's a planned meal AND there is no allergy conflict between the attendee and the recipe." The `unless` clause acts as negation.

## Variables

Variables in rules start with `?` and unify by name across all `when` and `unless` clauses:

| Variable | Meaning |
|---|---|
| `?this` | Binds to the entity being matched or derived |
| `?person` | User-defined variable; unifies across premises |
| `?recipe` | User-defined variable; unifies across premises |

All occurrences of the same variable name must bind to the same value for the rule to fire.

## Cross-Domain Rules

Rules can join data across multiple domains:

```yaml
user.rules:
  plan-event-meal:
    description: Suggest a meal for an event attendee using an available recipe
    deduce:
      diy.planner/Meal:
        attendee: ?person
        recipe: ?recipe
        occasion: ?event
    when:
      - diy.planner/Event:
          this: ?event
          title: ?title
      - diy.planner/Meal:
          this: ?this
          attendee: ?person
      - diy.cook/Recipe:
          this: ?recipe
          title: ?recipe-name
```

This rule joins across the `diy.planner` and `diy.cook` domains to match events with meals and recipes.

## Rules vs. Queries

Rules and queries are complementary:

- **Queries** ask "what data exists that matches this shape?"
- **Rules** define "given data of shape A, derive data of shape B."

Rules extend the space of queryable data without materializing it. When you query a concept that has rules, the rules fire and produce derived results alongside concrete data.

## Deductive vs. Inductive Rules

The rules described here are **deductive**: they interpret what is already in the database. Dialog also has a concept of **inductive rules** (inspired by [Dedalus](https://www2.eecs.berkeley.edu/Pubs/TechRpts/2009/EECS-2009-173.pdf)) that react to changes over time, producing new claims that get asserted back.

> [!NOTE]
> Inductive rules are prototyped but not yet fully implemented in Carry.
