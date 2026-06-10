# slide

Headless CLI for reading and writing tonk data and views as
asserted-notation. You define concepts (schemas), assert facts against
them, and publish views that render those facts.

## The loop

1. Discover what's already on the branch — don't guess or memorize:
   - `slide schema`   — every attribute and concept, as re-submittable notation
   - `slide concepts` — user-defined concepts (name + description)
   - `slide views`    — entities carrying a renderable claim
2. Write a notation document with `slide eval`:
   - `slide eval -c '<doc>'`   — inline
   - `slide eval <path>`       — from a file
   - `<doc> | slide eval -`    — from stdin
   A committing eval auto-syncs (pull-before / push-after) unless `--no-sync`.
3. Share a live view: `slide share display <entity> --view <name>`.

## Reference topics

Run `slide guide <topic>` — e.g. `slide guide notation`:

- `notation`  — asserted-notation syntax: queries, assertions, names,
                `this:`, fields, blanks, joins, built-ins.
- `views`     — display templates, `<tonk-display>` / `<tonk-concept>`,
                and how a view resolves from a model.
- `events`    — interactivity: effects, rules, transient concepts, and
                `on<event>=<concept>` DOM bindings.
- `workspace` — building sheets for the tonk-ui workspace shell
                (app-layer; subject to change).
- `all`       — every topic at once.

Each subcommand also carries examples: `slide <command> --help`.
