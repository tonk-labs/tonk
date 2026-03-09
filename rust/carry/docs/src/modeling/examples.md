# Modeling by Example

This chapter walks through complete domain models from simple to complex, showing how attributes, concepts, and rules compose in practice.

## Task Tracker (Minimal)

The simplest useful model -- a task with a title, status, and optional priority:

```yaml
app:
  Task:
    description: A task to be completed
    with:
      title:
        description: Title of the task
        as: Text
      status:
        description: Current status
        as: [":todo", ":in-progress", ":done"]
    maybe:
      priority:
        description: Priority level
        as: [":low", ":medium", ":high"]
```

Usage:

```bash
carry assert app/schema.yaml
carry assert task title="Write documentation" status=todo priority=high
carry assert task title="Review PR" status=in-progress
carry query task
carry query task status=todo
```

Note the use of enumerated symbols (`:todo`, `:in-progress`, `:done`) to constrain allowed values.

## Recipe Book (Multi-Concept)

A cooking domain with recipes, ingredients, and steps. This demonstrates cross-concept references and `cardinality: many`:

```yaml
diy.cook:
  Recipe:
    description: Meal recipe
    with:
      title:
        description: The name of this recipe
        as: Text
      ingredient:
        description: Ingredients of the recipe
        cardinality: many
      steps:
        description: Steps of the cooking process
        cardinality: many
        as: .RecipeStep

  Ingredient:
    description: The meal ingredient
    with:
      name:
        description: Name of this ingredient
        as: Text
      quantity:
        description: Quantity of the ingredient
        as: Integer
      unit:
        description: The unit of measurement
        as: [":tsp", ":mls"]

  RecipeStep:
    description: The cooking step
    with:
      instruction:
        description: Instructions for this step
        as: Text
    maybe:
      after:
        description: Step to perform this after
        as: .RecipeStep
```

Key patterns:
- **`.RecipeStep`** references another concept within the same domain (the leading `.` means "relative to this domain").
- **`cardinality: many`** on `ingredient` and `steps` means a recipe can have multiple of each.
- **Self-reference**: `RecipeStep.after` references another `RecipeStep`, enabling ordered sequences.

## Meal Planner (Rules and Cross-Domain Joins)

A more complex model that spans two domains and uses rules for inference:

```yaml
diy.health:
  Allergy:
    description: The allergy a person has
    with:
      person:
        description: Person having an allergy
      substance:
        description: Substance the person is allergic to
        as: Text

diy.planner:
  Event:
    description: Event being planned
    with:
      title:
        description: Title of the event
        as: Text
      time:
        description: Time of the event
        as: Text

  Meal:
    description: The plan for the meal
    with:
      attendee:
        description: The meal attendee
      recipe:
        description: The meal recipe
      occasion:
        description: The occasion for the meal

  AllergyConflict:
    description: A conflict between a recipe ingredient and an allergy
    with:
      person:
        description: The person with the allergy
      recipe:
        description: The recipe with the allergenic ingredient

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

This model demonstrates:

1. **Cross-domain references**: The allergy conflict rule joins data from `diy.cook` (recipes and ingredients) with `diy.health` (allergies).
2. **Derived concepts**: `AllergyConflict` doesn't store data directly -- it's derived by a rule from existing data.
3. **Negation**: The `respect-dietary-restrictions` rule uses `unless` to exclude meals where an allergy conflict exists.
4. **Variable unification**: `?substance` in the `find-allergy-conflicts` rule must match in both the ingredient name and the allergy substance.

## User Profile (Real-World EAV Data)

For highly structured, multi-faceted data like a user profile, you can use EAV triples directly. Here's a condensed example from the `carry.profile` domain:

```yaml
# Person
- the: carry.profile/name
  of: keri-vasquez
  is: "Keri Vasquez"
- the: carry.profile/role_title
  of: keri-vasquez
  is: "Independent consultant"
- the: carry.profile/location
  of: keri-vasquez
  is: "Amsterdam, NL"

# Expertise (cardinality: many through multiple entities)
- the: carry.profile/person
  of: keri-exp-knowledge-mgmt
  is: keri-vasquez
- the: carry.profile/topic
  of: keri-exp-knowledge-mgmt
  is: "Knowledge management theory"
- the: carry.profile/expertise_level
  of: keri-exp-knowledge-mgmt
  is: carry.profile/deep

# Communication preferences
- the: carry.profile/person
  of: keri-comm-conclusion-first
  is: keri-vasquez
- the: carry.profile/description
  of: keri-comm-conclusion-first
  is: "Lead with the conclusion, then the reasoning"
- the: carry.profile/direction
  of: keri-comm-conclusion-first
  is: carry.profile/inbound
```

This pattern uses **satellite entities** (like `keri-exp-knowledge-mgmt`) linked back to a central person entity. Each satellite carries its own fields, enabling rich, queryable structures without nested objects.

## Tips for Modeling

1. **Start with raw domain assertions.** Don't design a schema upfront. Assert data as you have it and let patterns emerge.

2. **Use domains for namespacing, concepts for structure.** Domains prevent name collisions. Concepts give you validation and named queries.

3. **Prefer `cardinality: many` for lists.** Instead of comma-separated values in a text field, use a many-valued attribute. This makes each item independently queryable.

4. **Use entity references for relationships.** Instead of embedding data, reference other entities. This keeps your model normalized and composable.

5. **Rules are views, not triggers.** Rules derive data at query time. They don't materialize new claims into storage. Use them for joins, constraints, and derived aggregates.
