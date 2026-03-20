# Structured Data Modeling

Carry can serve as a lightweight, local database for structured data that doesn't warrant standing up PostgreSQL or managing a cloud service. If your data has entities, relationships, and you want to query across them -- Carry might be a good fit.

## When to Use Carry for Data

Carry works well for:

- **Highly structured data** with relationships between entities
- **Moderate data volumes** (hundreds to thousands of entities)
- **Data that benefits from semantic modeling** -- where the schema itself carries meaning
- **Data you want to query from AI tools** alongside your code
- **Private or sensitive data** that shouldn't leave your machine

Carry is less suited for:

- Large datasets requiring high-throughput analytics (use a proper OLAP database)
- Short bits of declarative data (plain markdown files are simpler)
- Data that needs concurrent multi-user write access (sync is still developing)

## Example: Lab Data Management

Research labs often struggle with inconsistent data entry across team members. Carry can provide lightweight schemas that enforce structure:

```yaml
lab.samples:
  Sample:
    description: A collected sample
    with:
      id:
        description: Sample identifier
        as: Text
      collector:
        description: Who collected it
        as: Text
      date:
        description: Collection date
        as: Text
      type:
        description: Sample type
        as: [":blood", ":tissue", ":soil", ":water"]
    maybe:
      location:
        description: Collection location
        as: Text
      notes:
        description: Collection notes
        as: Text
      ph:
        description: pH measurement
        as: Float

  Measurement:
    description: A measurement taken on a sample
    with:
      sample:
        description: The sample measured
      metric:
        description: What was measured
        as: Text
      value:
        description: The measurement value
        as: Float
      unit:
        description: Unit of measurement
        as: Text
    maybe:
      instrument:
        description: Instrument used
        as: Text
```

```bash
carry assert schema.yaml

carry assert sample @sample-001 \
  id=S-001 collector="Dr. Chen" date="2026-03-15" \
  type=water location="Station A" ph=7.2

carry assert measurement \
  sample=sample-001 metric="dissolved oxygen" \
  value=8.5 unit="mg/L" instrument="YSI ProDSS"

# Query all water samples
carry query sample id collector date type=water

# Find measurements for a specific sample
carry query measurement sample=sample-001
```

## Example: Small Business Operations

A small business tracking customers, orders, and inventory:

```yaml
biz:
  Customer:
    description: A customer
    with:
      name:
        description: Customer name
        as: Text
      email:
        description: Email address
        as: Text
    maybe:
      phone:
        description: Phone number
        as: Text
      since:
        description: Customer since
        as: Text

  Product:
    description: A product in inventory
    with:
      name:
        description: Product name
        as: Text
      price:
        description: Unit price
        as: Float
      stock:
        description: Units in stock
        as: UnsignedInteger
    maybe:
      category:
        description: Product category
        as: Text

  Order:
    description: A customer order
    with:
      customer:
        description: The customer who placed the order
      product:
        description: The product ordered
      quantity:
        description: Number of units
        as: UnsignedInteger
      date:
        description: Order date
        as: Text
```

Cross-entity queries:

```bash
# All orders for a customer
carry query order customer=did:key:zCustomer1

# All products in a category
carry query product category="electronics"
```

## Example: Analytics Engineering

For analytics engineers familiar with dbt-style transformations, Carry's rules can model data transformations declaratively:

```yaml
analytics:
  RawEvent:
    description: A raw analytics event
    with:
      event_type:
        description: Type of event
        as: Text
      user_id:
        description: User identifier
        as: Text
      timestamp:
        description: Event timestamp
        as: Text
    maybe:
      properties:
        description: Event properties
        as: Text

  ActiveUser:
    description: A user who has been active recently
    with:
      user_id:
        description: User identifier
        as: Text
      last_event:
        description: Most recent event type
        as: Text

  identify-active-users:
    description: Derive active users from raw events
    deduce:
      ActiveUser:
        user_id: ?user
        last_event: ?event_type
    when:
      - analytics/RawEvent:
          this: ?this
          user_id: ?user
          event_type: ?event_type
```

This approach gives you:
- **Documented transformations**: The rule description and concept descriptions serve as living documentation.
- **Testable logic**: Assert test data, run the rule, verify the output.
- **Version-controlled models**: Everything is YAML on disk.

## Tips

1. **Start with data, add schema later.** Assert raw domain claims first. When you see patterns, formalize them into concepts.

2. **Use separate repos for environments.** Keep production data in one repository and test data in another by using `--repo` to target different directories.

3. **Export with queries.** Need CSV? Pipe JSON output through `jq`:
   ```bash
   carry query sample --format json | jq -r '.[] | [.id, .collector, .date] | @csv'
   ```

4. **Rules are your transformation layer.** Instead of writing scripts to join and transform data, express the logic as rules. They're declarative, documented, and always up to date.
