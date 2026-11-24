# The notation tour (demo)

A space whose home is a notebook that tours the Dialog notation: every
```` ```notation ```` block in it is a cell the space's worker reads, expands,
analyzes and compiles with `dialog-notation` (at the revision this branch
pins), runs against the space's branch, and answers with what each form did
and what each stage of the pipeline gave.

Everything here is demo scaffolding on a proof-of-concept branch.

## See it

```bash
.claude/skills/run-app/dev-web.sh      # serves the app on :8080 (~10 min cold)
```

Then open, in a browser:

    http://localhost:8080/seed/http://localhost:8080/library/notation-tour.yaml

name the space (say "Notation tour") and create it. The space opens on the
tour notebook. Press Run on each cell from the top (each cell reads the
definitions of the cells above it, and the facts the cells above wrote).

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
| the tour, as Markdown | `demo/notation-tour/tour.md` |
| the seed a space is created from, generated | `rust/tonk-core/assets/library/notation-tour.yaml` (`python3 demo/notation-tour/build.py`) |
| a cell, in the notebook | `rust/tonk-inspector/src/notation_cell.rs` (two hooks in `notebook.rs`) |
| the body's content type through the host | `rust/tonk-host/src/{consumer,ops}.rs` (`evaluate_as`, `contentType`) |
| the worker side, on the existing `/evaluate` route | `rust/tonk-worker/src/router/notation_tour.rs` (one branch in `evaluate.rs`) |
| the evaluator | `rust/tonk-analyzer/src/notation/tour.rs` |
| every cell runs | `rust/tonk-analyzer/tests/notation_tour.rs` |
| the browser proof | `demo/notation-tour/tour.mjs` |
