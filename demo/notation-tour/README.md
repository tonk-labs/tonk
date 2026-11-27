# The notation tour (demo)

A space whose home is a notebook that tours the Dialog notation: every
```` ```notation ```` block in it is a cell the space's worker reads, expands,
analyzes and compiles with `dialog-notation` (at the revision this branch
pins), runs against the space's branch, and answers with what each form did
and what each stage of the pipeline gave.

Everything here is demo scaffolding on a proof-of-concept branch.

## See it

On a machine with the repository's normal tooling:

```bash
nix develop -c dev:web                 # serves the app on :8080
```

In a container without nix (as the `run-app` skill does):

```bash
.claude/skills/run-app/dev-web.sh      # serves the app on :8080; logs in /tmp/tonk-run/
```

A cold build took about 80 minutes in this container at two build jobs
(`CARGO_BUILD_JOBS=2`); it is much less on a workstation, and seconds once
warm. In the browser each cell runs in 70 to 150 ms.

Open http://localhost:8080/ and let the new profile finish its onboarding.
On the hub, open `troubleshooting`, then "can I use tonk without an
agent?", and follow "start from the notation tour". That opens the `/seed`
page for the bundled tour (`/seed/%2Flibrary%2Fnotation-tour.yaml`, the
same link on any origin); name the space (say "Notation tour") and create
it. The link is demo scaffolding: a template picker over `space/create`'s
`template` is where a starting point like this belongs. The space opens on
the tour notebook. Press Run on each cell from the top (each cell reads the
definitions of the cells above it, and the facts the cells above wrote).
Under the Views cell, press Watch live before running the cells below it:
the to-dos they add arrive in the watched card as the driver's patches.

To run the whole tour headless and check every cell:

```bash
cd demo/notation-tour
ln -sf "$(npm root -g)" node_modules
node tour.mjs                          # screenshots in ./shots/
```

To run every cell without a browser (the cell evaluator over an in-memory
branch):

```bash
cargo test -p tonk-analyzer --test notation_tour
```

## Where it lives

| piece | file |
|---|---|
| the tour, as Markdown | `rust/tonk-analyzer/tests/notation-tour/tour.md` (inside the crate, because the nix build takes only `rust/` as source) |
| the seed a space is created from, generated | `rust/tonk-core/assets/library/notation-tour.yaml`, written by `the_tour_seed_is_current` in `rust/tonk-analyzer/tests/notation_tour.rs` (`TONK_TOUR_WRITE_SEED=1 cargo test -p tonk-analyzer --test notation_tour`; the test fails when the seed is stale against `tour.md` or `notebook.yaml`) |
| what the tour found in dialog | `demo/notation-tour/findings.md`, `rust/tonk-analyzer/tests/notation-tour/findings/*.notation`, `rust/tonk-analyzer/tests/notation_findings.rs` |
| a cell, in the notebook | `rust/tonk-inspector/src/notation_cell.rs` (two hooks in `notebook.rs`) |
| the body's content type through the host | `rust/tonk-host/src/{consumer,ops}.rs` (`evaluate_as`, `contentType`) |
| the worker side, on the existing `/evaluate` route | `rust/tonk-worker/src/router/notation_tour.rs` (`evaluate_route` in `evaluate.rs`) |
| the live view, on the existing `/query` route | `notation_tour.rs` (`live_frames`, one branch in `query.rs`) and `notation_cell.rs` (`apply_patch`) |
| the evaluator | `rust/tonk-analyzer/src/notation/tour.rs` |
| every cell runs | `rust/tonk-analyzer/tests/notation_tour.rs` |
| the browser proof | `demo/notation-tour/tour.mjs` |

## What is real, what is scaffolding

Real:

- the notation's own pipeline, `dialog-notation` at the pinned revision
  (`parse_document`, `check_forms`, the analyzed kernel trees, `compile`'s
  steps);
- every write committed to the space's branch through the branch's stack
  under its writer lock, like any `/evaluate` commit; queries answered by
  Dialog's engine;
- views rendered, watched and patched by the notation's driver
  (`Live::render`, `Live::watch`).

Scaffolding:

- the cell UI and its JSON answer; the content-type branch on `/evaluate`
  and the body branch on `/query` (no route added);
- the tour evaluator writes the `syntax.form/is` records itself, as
  Dialog's own tests do; Dialog's `check` writes no syntax;
- a live view is polled every 250 ms in the worker, not woken on commit;
- a live view's rules stay on the branch's session overlay after the page
  goes (the known `Live::lay` leak, being fixed in Dialog);
- events from a live view post their command as notation text to the tour
  evaluator, not through tonk's command path (`delegate.rs` → `/transact`);
- routing: the space is made by the seed link and opens on its home view,
  which shows the notebook; no route of the notation's is involved, and the
  notebook installed is `notebook.yaml`, not `notebook.clj`;
- the Dialog pin is one exact revision of the notation query pipeline
  (`2d569109`), the same in the workspace and in the three crates outside
  it;
- the space's title pill at the top right is tonk's own chrome and overlaps
  the first lines of a long notebook; left as it is.

## Known limits

- The tour demonstrates separate and chained `syntax/parse`,
  `syntax/analyze` and `syntax/compile` selects. Notation cells show
  inline diagnostics, completion and reference hover using declarations in
  that notebook's earlier cells.
- Definitions live in the analyzer's memory only, so a cell re-reads the
  cells above it when Run is pressed. The editor retains checked cell
  contexts and refreshes downstream cells when an earlier cell changes.
- Natively the live stream is not run.
- Two behaviours of the notation the tour found are written up in
  `findings.md`; both are fixed at the pinned revision, and
  `notation_findings.rs` holds each fixed answer.
- A check written as a rule can only warn, over the expander's few built-in
  premises (a form's head, a command's name and domain, the names that have
  syntax rules, the domains installed rules read); it cannot refuse, and it
  cannot read the branch.
