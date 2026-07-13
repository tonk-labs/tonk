# tonk

Headless CLI for reading and writing tonk data and views as
asserted-notation. You define concepts (schemas), assert facts against
them, and publish views that render those facts.

## The loop

1. Discover what's already on the branch — don't guess or memorize:
   - `tonk schema [<concept>]` — attributes and concepts, as re-submittable notation
   - `tonk concepts` — user-defined concepts (name + description)
   - `tonk views`    — entities carrying a renderable claim
2. Define schema: `tonk concept add <name> --attr <field>:<type>:<card> …`
   (types enumerate on a miss; `one`/`many` cardinality). The concept is
   immediately usable: `tonk assert <name> --help` shows its typed flags.
3. Work the data with the argument verbs (each write auto-syncs):
   - `tonk assert <concept> --<field> <value> …`          — mint an instance
   - `tonk assert <concept> <entity> --<field> <value> …` — supersede fields
   - `tonk query <concept>` / `tonk get <concept> <entity>` — read (`--json`)
   - `tonk retract <concept> <entity> [--field <f>]`      — retract claims
4. Give it a view and put it on the space home — a build nobody can see
   isn't done:
   - `tonk view add <concept> --template '<b>{field}</b>'` — declarative
     view; auto-surfaces onto the home when none is set yet
   - `tonk home <concept> [<concept> …]` — pin the space home explicitly
   - `tonk render <route>` — check headlessly: `tonk render person`
     (directory), `tonk render alice@person!card` (explicit view)
5. Escape hatch for anything the verbs don't cover (rules, effects,
   multi-statement documents): `tonk eval -c '<doc>'`, `tonk eval <path>`,
   or `<doc> | tonk eval -`. A committing eval auto-syncs unless
   `--no-sync`; `--dry-run` previews without committing (queries run,
   transaction dropped, branch untouched).
6. Share a live view: `tonk share display <entity> --view <name>`.

## Reference topics

Run `tonk guide <topic>` — e.g. `tonk guide notation`:

- `notation`  — asserted-notation syntax: queries, assertions, names,
                `this:`, fields, blanks, joins, built-ins.
- `views`     — display templates, `<tonk-display>`, how a view
                resolves from a model, and web components
                (`component!:`) for behaviour templates can't express.
- `events`    — interactivity: effects, rules, transient concepts, and
                `on<event>=<concept>` DOM bindings.
- `workspace` — building sheets for the tonk-ui workspace shell
                (app-layer; subject to change).
- `all`       — every topic at once.

Each subcommand also carries examples: `tonk <command> --help`.
