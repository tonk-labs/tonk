# tonk

Headless CLI for reading and writing tonk data and views as
asserted-notation. You define concepts (schemas), assert facts against
them, and publish views that render those facts.

## Mental model: it's a datalog

Everything on a branch is a **fact** — entity, attribute, value — as
in datalog or Datomic. Nothing updates in place: you assert new claims,
and a cardinality-one attribute supersedes its old value while a
cardinality-many one accumulates. "Retract" is itself a claim that
invalidates an earlier one.

- **attribute** — a typed predicate (`the:` URI, `as:` type, cardinality)
- **concept** — a named schema over attributes (its `with:` map);
  a concept query matches entities carrying every required attribute
- **query** — pattern matching with unification: the same `?x` across
  expressions must take the same value, like datalog body clauses
- **view** — an HTML template rendered over a concept's instances
- **rule** — a datalog rule whose trigger premise is a transient
  command fact produced by a DOM event; its head asserts/retracts facts

Two consequences worth internalizing early: entity identity is
content-addressed (re-asserting an identical body is a no-op; changing
any field mints a NEW entity unless you bind the old one with `this:`),
and bare lowercase tokens are symbols resolved through the name table —
quote every string literal (`name: "alice"`, not `name: alice`).

## Spots

Commands run against the selected *spot* (a named fact store). The
cwd never locates site data — it's only a possible key into the
registry. Resolution order: `--spot <name>` > `TONK_SPOT` env > a
binding created by `tonk use <name>`. There is no global fallback.
In automation, pin the spot per-process (`TONK_SPOT=x tonk ...` or
`--spot x`), or bind a dedicated working directory once with `tonk
use <name>`. `tonk spot unbind` removes an exact binding. `tonk spot
list` shows what is registered, every bound directory, and what is
active for this invocation.

## The loop

1. Discover what's already on the branch — don't guess or memorize:
   - `tonk schema [<concept>]` — attributes and concepts, as re-submittable notation
   - `tonk concept ls` — the concepts this space defines (name + description)
   - `tonk view ls`    — entities carrying a renderable claim, and the model each renders
2. Define schema: `tonk concept add <name> --attr <field>:<type>:<card> …`
   (types enumerate on a miss; `one`/`many` cardinality). The concept is
   immediately usable: `tonk assert <name> --help` shows its typed flags.
3. Work the data with the argument verbs (each write auto-syncs):
   - `tonk assert <concept> --<field> <value> …`          — mint an instance
   - `tonk assert <concept> <entity> --<field> <value> …` — supersede fields
   - `tonk query <concept> [<entity>]` — read all or one (`--json`)
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
6. Look at it. In the shell: `/space/<space>/<model>` (directory),
   `/space/<space>/<entity>@<model>!<view>` (one entity, named view).
   Headlessly: `tonk render` on the same routes.
7. Hand the repo to someone else: `tonk invite`.

## Reference topics

Run `tonk guide <topic>` — e.g. `tonk guide notation`:

- `notation`  — asserted-notation syntax: queries, assertions, names,
                `this:`, fields, blanks, joins, built-ins.
- `views`     — display templates, how a view resolves from a model,
                the built-in elements (`<tonk-display>`, `<tonk-prose>`,
                `<tonk-code>`, `<tonk-table>` — full docs via
                `tonk guide views <element>`), and web components
                (`component!:`) for behaviour templates can't express.
- `events`    — interactivity: effects, rules, transient concepts, and
                `on<event>=<concept>` DOM bindings.
- `workspace` — building sheets for the tonk-ui workspace shell
                (app-layer; subject to change).
- `all`       — every topic at once.

Each subcommand also carries examples: `tonk <command> --help`.
