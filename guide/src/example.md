# An example

Let's build a counter: a number with a button that increases it. We add it one piece at a time, so each part has a reason to exist before the next arrives.

## Start with the data

A counter relates to one thing: a count. We model that as a **concept** (a named set of relations), and the relation it selects as an **attribute**:

```yaml
concept!: &counter
  description: A tally that can be incremented
  with:
    count:
      description: The current count
      the: xyz.tonk.counter/count
      as: unsigned-integer
```

`&counter` names this concept so we can refer to it as `counter` later. Every concept and attribute needs a `description`: it is stored with the data and makes concepts discoverable, so write it for a stranger.

Now assert one counter, starting at zero:

```yaml
counter!: &my-counter
  count: 0
```

Paste both blocks into the editor and evaluate them. You have a counter in your data, but nothing to look at yet.

## Add a view

A **view** says how a concept renders. It is an HTML template with `{placeholders}` filled from the data:

```yaml
view!:
  this: counter
  show:
    ui: |
      <div>
        <span>{count}</span>
      </div>
```

`this: counter` puts the template on the concept it renders — a view is the concept's own `show` dictionary, and `ui` is the facet `<tonk-display>` shows by default. `{count}` is replaced by the value. Render it with `<tonk-display>`:

```html
<tonk-display model="counter" entity="…your counter…"></tonk-display>
```

The screen shows `0`. Edit the data and it follows; edit the template and the screen updates.

## Add the button and what a click means

Put a button in the view. We stamp the counter's own identity onto it with `{this}` so a click can tell which counter it came from, and wire the button to a command named `increment`:

```yaml
view!:
  this: counter
  show:
    ui: |
      <div>
        <button onclick=increment data-counter={this}>+</button>
        <span>{count}</span>
      </div>
```

A **command** is what an interaction means, captured as data. The click becomes an `increment` that records which counter was clicked:

```yaml
command!: &increment
  description: A request to increase a counter by one
  with:
    counter:
      description: The counter to increment, read from the button's data-counter
      the: dom.event.current-target.dataset/counter
      as: entity
```

Now clicking fires an `increment`, but nothing reacts to it yet.

## React with a rule

A **rule** reacts: when an `increment` arrives for a counter, read its current count, add one, and write the result back.

```yaml
rule!:
  description: Applies an increment to a counter's count
  assert!: counter
  when:
    - assert: increment
      where: { counter: ?this }
    - assert: counter
      where: { this: ?this, count: ?current }
    - assert: math/sum
      where: { of: ?current, with: 1, is: ?count }
```

The `when:` block lists what must hold: there is an `increment` for some counter (`?this`), that counter has a current `count` (`?current`), and `?current + 1` is `?count` (the `math/sum` formula). When all three hold, the rule asserts the counter's `count` as `?count`.

That is the whole counter. Click `+`: the command fires, the rule computes the next count, the data changes, and the view re-renders. The [next chapter](./the-model.md) names the pattern you just assembled.
