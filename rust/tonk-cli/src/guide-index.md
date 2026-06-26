# tonk

Headless CLI for reading and writing tonk data and views as
asserted-notation. You define concepts (schemas), assert facts against
them, and publish views that render those facts.

## The loop

1. Discover what's already on the branch — don't guess or memorize:
   - `tonk schema`   — every attribute and concept, as re-submittable notation
   - `tonk concepts` — user-defined concepts (name + description)
   - `tonk views`    — entities carrying a renderable claim
2. Write a notation document with `tonk eval`:
   - `tonk eval -c '<doc>'`   — inline
   - `tonk eval <path>`       — from a file
   - `<doc> | tonk eval -`    — from stdin
   A committing eval auto-syncs (pull-before / push-after) unless `--no-sync`.
   Add `--dry-run` to preview a document without committing: queries run and
   matches come back, but the transaction is dropped and the branch is left
   untouched (zero claims, unchanged revision). Implies `--no-sync`.
3. Share a live view: `tonk share display <entity> --view <name>`.
4. Render a view to HTML headlessly: `tonk render <route>` —
   `tonk render person` (directory), `tonk render alice@person` (one
   entity), `tonk render alice@person!card` (explicit view). Writes
   HTML to stdout, or `--out <file>`.

## Reference topics

Run `tonk guide <topic>` — e.g. `tonk guide notation`:

- `notation`  — asserted-notation syntax: queries, assertions, names,
                `this:`, fields, blanks, joins, built-ins.
- `views`     — display templates, `<tonk-display>`, and how a view
                resolves from a model.
- `events`    — interactivity: effects, rules, transient concepts, and
                `on<event>=<concept>` DOM bindings.
- `workspace` — building sheets for the tonk-ui workspace shell
                (app-layer; subject to change).
- `all`       — every topic at once.

Each subcommand also carries examples: `tonk <command> --help`.
