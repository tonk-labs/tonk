# View anchors and render parity

**Goal:** Make view identity explicit at authoring time and make `tonk render`
show every view entry that the browser would mount, so an accidental duplicate
is visible from the CLI instead of being hidden by first-row selection.

**Approach:** Replace the view-specific `--name` flag with the more precise
`--anchor` flag and expose the derived `id:<anchor>` identity in command output.
Then replace the headless renderer's single-row view resolution with the same
entity-keyed, all-row behavior already used by `tonk-display`.

**Constraints:**

- Remove `--name` immediately from `tonk view add`; do not retain an alias or
  deprecation path. Other commands' unrelated `--name` flags are unchanged.
- Keep ordinary `view add --help` concise. It must not include the sentence
  about selecting an arbitrary entity or the `tonk assert` update command.
- An anchor containing `:` is treated as entity-like and rejected before any
  write. The targeted error may explain that it would derive `id:<anchor>` and
  show the existing `tonk assert` remediation.
- Keep slash-containing anchors such as `vault/frame-view` valid.
- Do not add a new `--entity` flag. Existing arbitrary-entity updates remain an
  `assert` operation.
- Preserve `--notation` as raw, re-evaluable notation; its `this: id:<anchor>`
  line already makes the identity explicit.
- Browser behavior is authoritative: all renderable matching rows are mounted
  in view-entity order, and fallback views are used only when the requested
  model has no renderable matches.
- Add no dependencies and do not change lock files.
- Rust verification currently needs more free disk space: the last focused
  build stopped before running tests with `No space left on device`. Check
  capacity before compiling and do not remove shared build artifacts without
  explicit approval.

## File map

| File | Responsibility | Planned change |
|---|---|---|
| `rust/tonk-cli/src/bin/tonk.rs` | Clap command surface and parser tests | Rename only `view add --name` to `--anchor`; update examples and rejection tests. |
| `rust/tonk-cli/src/authoring.rs` | Default anchors and view declaration construction | Add entity-like anchor validation and focused unit tests. |
| `rust/tonk-cli/src/data_ops.rs` | `view_add` orchestration and user-facing result | Validate the resolved anchor and report both anchor and derived entity. |
| `rust/tonk-cli/src/guide-views.md` | Embedded view guide | Use `--anchor` and explain the `id:<anchor>` derivation. |
| `rust/tonk-cli/src/guide-events.md` | Embedded event/view example | Use `--anchor`; remove the implication that only one matching view is valid. |
| `rust/tonk-render/src/page/orchestrate.rs` | Headless view selection and page rendering | Resolve, order, and render every matching view instead of calling `.next()`. |
| `rust/tonk-cli/tests/authoring.rs` | CLI authoring integration coverage | Prove explicit/default identities, dry-run hints, and pre-write rejection. |
| `rust/tonk-cli/tests/render.rs` | End-to-end headless render coverage | Reproduce the duplicate-view case and prove deterministic all-entry output. |

## Task 1: Rename the view identity input and make derivation explicit

**Interfaces:** `ViewCommand::Add` consumes `--anchor`; `ViewKind::default_anchor`
still supplies the omitted value; `build_view_decl` still emits
`this: id:<anchor>`; `data_ops::view_add` reports the resolved pair.

- [x] Add parser tests first in `rust/tonk-cli/src/bin/tonk.rs` that accept
  `tonk view add ... --anchor note-card` and reject the former
  `tonk view add ... --name note-card` spelling. Keep representative tests for
  unrelated `--name` flags intact.
- [x] Rename the `ViewCommand::Add` field and value name to `anchor` / `ANCHOR`,
  pass it through `view_op`, and update command examples. Use concise help such
  as: `Stable anchor used to derive the view entity id:<ANCHOR> (default depends
  on --kind).` Do not add the rejected arbitrary-entity paragraph.
- [x] Add a focused validator in `rust/tonk-cli/src/authoring.rs`. It must reject
  `tonk:vault/shell`, mention the unintended `id:tonk:vault/shell` derivation,
  and provide an actionable `tonk assert` update hint; it must accept
  `vault/frame-view` and generated defaults.
- [x] Add failing integration tests in `rust/tonk-cli/tests/authoring.rs` for an
  explicit anchor, an omitted/default anchor, dry-run output, and an
  entity-like anchor. For rejection, capture the current revision before the
  command and prove it is unchanged afterward.
- [x] Rename `data_ops::view_add`'s parameter, validate the resolved value before
  evaluation or commit, and make non-notation output state both
  `anchor: <anchor>` and `entity: id:<anchor>`. Preserve `WriteOptions` wording
  for committed versus dry-run writes rather than claiming create versus
  update semantics that the operation has not checked.
- [x] Update `guide-views.md` and `guide-events.md` to use `--anchor`. Explain
  that omitting it generates a stable kind-specific anchor and that supplying
  it derives `this: id:<anchor>`; state that every matching view entry renders.
- [x] Run the new parser and authoring tests, then the complete authoring test
  target after the implementation is green.

## Task 2: Render every matching view in browser-equivalent order

**Interfaces:** `view_by_model_query` already produces every matching row.
`tonk-render` should turn those rows into `Vec<ResolvedView>` values carrying
their entity identity, sort them by that identity, and concatenate the rendered
siblings. `tonk-display` remains unchanged and supplies the reference behavior.

- [x] Add a failing regression in `rust/tonk-cli/tests/render.rs` using a fresh
  site with exactly two views for one model. Give the views reverse insertion
  order and stable entity IDs, then assert that both distinct templates occur
  exactly once and appear in ascending entity-ID order.
- [x] Replace `query_view` / `resolve_view` in
  `rust/tonk-render/src/page/orchestrate.rs` with plural equivalents. Drop rows
  without `display`, retain the view entity in `ResolvedView`, sort by entity,
  and query `tonk:_` only when the requested model produces no renderable rows.
- [x] Refactor `render_at_depth` so it queries the target entity or directory
  once, renders each inline template against the same conclusions, expands
  nested views within each result, and concatenates the sibling HTML without a
  new wrapper element.
- [x] Preserve the browser's frame-level portal rule: if any matched view is a
  portal, render every matched display as a portal sibling in the same entity
  order and skip ordinary entity interpolation for that frame.
- [x] Run the new render regression first, followed by the complete
  `tonk-cli --test render` target and `tonk-render` tests.

## Verification

- [x] Check free space with `df -h .` before compiling. If capacity is still
  insufficient, report the blocked commands rather than deleting artifacts.
- [x] Run `cargo fmt --all -- --check`.
- [x] Run the focused parser, authoring, and render tests with one test thread.
- [x] Run `cargo test -p tonk-cli -- --test-threads=1` and
  `cargo test -p tonk-render -- --test-threads=1`.
- [x] Run
  `cargo clippy -p tonk-cli -p tonk-render --all-targets --all-features -- -D warnings`.
- [x] Run `git diff --check` and inspect the final diff to confirm no unrelated
  `--name` surface or browser rendering code changed.
