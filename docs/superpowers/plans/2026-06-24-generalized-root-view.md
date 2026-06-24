# Generalized Root View + Sheets-as-Template Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop forcing every Tonk space into the binder/sheets shell; make the space render one editable root composition that shows a launchpad when empty and a chosen template (starting with the current sheets UI) when applied.

**Architecture:** All of this lives in the seeded notation library `rust/tonk-core/assets/library/core.yaml`, plus a small `demo.yaml` change. We introduce a `tonk:root` pointer concept and a `tonk:apply-template` command, repoint the `tonk/space` alias from `tonk:binder` to `tonk:root`, relocate the topbar/dialogs and the binder/sheet machinery into clearly-bounded sections, and add a launchpad empty-state. The existing `<tonk-display>` dynamic `entity`/`model`/`view` binding (already proven by the sheet mount) is the delegation mechanism. No Rust element behavior changes; `space_sealed.rs` is untouched.

**Tech Stack:** dialog-yaml notation (the seed library), the `tonk-analyzer` `analyze_local` resolver (native test gate), Rust custom elements in `tonk-workspace`/`tonk-display` (unchanged here), `#[dialog_common::test]`.

## Global Constraints

- **Notation file:** all seed edits are in `rust/tonk-core/assets/library/core.yaml` unless a task says otherwise. There is a built copy at `rust/tonk-ui/dist/library/core.yaml` — do NOT hand-edit it; it is regenerated from the source asset by the build.
- **Naming:** do NOT name any new concept `workspace` — it collides with tonk-layout. Use `tonk:root`, `tonk:sheets-page`, `tonk:apply-template`.
- **No phase/RFC references** in notation comments or code. Comments stand on their own.
- **Native lint gate:** `cargo clippy --all -D warnings` must pass. wasm-only Rust helpers must be gated with `#[cfg(target_arch = "wasm32")]` or they read as native-dead.
- **Notation resolves natively:** every notation change must keep `analyze_local` over the seeded library green (Task 1 adds the regression test; reuse it after every later task).
- **Interim seam — conceptual routing is NOT landed.** The `route`/`site` concepts, the service-worker routing table, and the dissolved Leptos shell are parallel future work. Therefore:
  - The topbar stays declarative *inside the space root shell view* (the view `tonk/space` resolves to), decoupled from the binder so it can later lift into the `route/space` view unchanged. It does NOT move to a route view in this plan.
  - Local-replica scoping of the sheets binder (`site.replica`) is deferred. Because fresh repos now default to the launchpad (not the binder), the old per-replica binder duplication is no longer the default experience; it only appears after the sheets template is applied. Leave a comment marking the scoping as a follow-up.
- **VCS:** colocated jj/git. Commit with conventional-commit subjects. End commit messages with the `Co-Authored-By` trailer the repo uses.

---

## File Structure

- `rust/tonk-core/assets/library/core.yaml` — all seed notation. Within it, work in these regions:
  - the `tonk:sheet` concept (currently ~140-178) and the sheet commands/rules (~186-231, ~834-895, ~1398-1481).
  - the `tonk:shell` directory view (currently ~1545-1813) — the topbar + dialogs + binder mount.
  - the `tonk:sheet-binder` directory view (currently ~1816-1872).
  - the `tonk/space` pinned alias (currently ~1394-1396).
- `rust/tonk-core/assets/library/demo.yaml` — applies the sheets template so the bundled `home` demo still opens to sheets.
- A native regression test (Task 1) wherever the seeded library is already parsed in tests; locate the existing harness (search for `include_str!` of `core.yaml`, or for `analyze_local` test usage in `rust/tonk-analyzer/src/analyzer.rs` and `rust/tonk-core`).

> **Notation reality:** this is a bespoke notation system. For every NEW block below, the target notation is given concretely, but you MUST iterate it against `analyze_local` (and, where noted, a running app) until it resolves and renders. When a mechanic is uncertain, the task names an existing block to copy the pattern from. Treat "iterate until the gate passes" as part of the implementation step, not a placeholder.

---

### Task 1: Root pointer concept + apply-template command + regression gate

Introduces the data model for "which template fills the space," additively. Nothing renders differently yet — the `tonk/space` alias is repointed only in Task 5.

**Files:**
- Modify: `rust/tonk-core/assets/library/core.yaml` (add new blocks near the other workspace concepts/commands, e.g. just after the `tonk:sheet` concept ~line 178)
- Test: the native seed-resolves regression test (create if absent — see Step 1)

**Interfaces:**
- Produces:
  - concept `tonk:root` with fields `entity` (entity), `model` (entity), `view` (entity), all `cardinality: one`.
  - pinned alias `id:tonk/root` → the singleton root entity.
  - command `tonk:apply-template` with fields `entity`, `model`, `view` read from `dom.event.current-target.dataset/*`.
  - rule asserting `tonk:root` onto `id:tonk/root` from a `tonk:apply-template` command.

- [ ] **Step 1: Add (or locate) the native seed-resolves regression test**

Search for an existing test that loads `core.yaml` and runs `analyze_local`. If none loads the real asset, add this test in the `tonk-analyzer` test module (alongside the example at `rust/tonk-analyzer/src/analyzer.rs` ~line 989). It is the gate reused by every later task.

```rust
/// The seeded standard library must resolve under the env-free local
/// path. Guards every notation edit to core.yaml.
#[dialog_common::test]
fn it_resolves_the_seeded_standard_library() {
    let source = include_str!(
        "../../tonk-core/assets/library/core.yaml"
    );
    let syntax = must_parse(source);
    let result = super::analyze_local(&syntax);
    assert!(
        result.is_ok(),
        "core.yaml must resolve under analyze_local: {:?}",
        result.err(),
    );
}
```

Adjust the `include_str!` relative path to the crate that hosts the test. If `must_parse` is not in scope there, reuse the same parse helper the neighbouring `analyze_local` tests use.

- [ ] **Step 2: Run it to confirm the baseline passes**

Run: `cargo nextest run -p tonk-analyzer it_resolves_the_seeded_standard_library`
Expected: PASS (the unmodified library already resolves). If it fails, fix the test wiring before continuing — you need a green baseline.

- [ ] **Step 3: Add the `tonk:root` concept and singleton alias**

Insert after the `tonk:sheet` concept block. Verify the singleton-alias form against the existing `id:tonk/space` alias (~line 1394) and adjust to match its exact shape.

```yaml
# The space's root composition pointer. A single per-repo fact (not
# per-replica) naming which template fills the space: the entity to
# display and the model/view to display it through — the same triple a
# sheet carries. Seeded WITHOUT these fields, so the `tonk:root` query
# does not match until a template is applied; an unmatched root renders
# the launchpad. `apply-template` asserts the fields onto `id:tonk/root`.
concept!: &root
  this: tonk:root
  description: The space's root composition pointer — which template fills the space.
  with:
    entity:
      description: Entity the active template displays.
      the: xyz.tonk.root/entity
      cardinality: one
      as: entity
    model:
      description: Model concept the active template uses.
      the: xyz.tonk.root/model
      cardinality: one
      as: entity
    view:
      description: View concept the active template uses.
      the: xyz.tonk.root/view
      cardinality: one
      as: entity

# Stable address of the single root pointer for this space.
name!:
  this: id:tonk/root
  entity: tonk:root
```

- [ ] **Step 4: Add the `tonk:apply-template` command and its assert rule**

```yaml
# Apply a layout template to the space root. A launchpad control (or any
# template-picker) submits this with the chosen template's entity/model/
# view on its dataset; the rule points the root pointer at them. Marker
# `template` gives this transient an attribute no other command carries.
command!: &apply-template
  this: tonk:apply-template
  description: Apply a layout template to the space root.
  with:
    entity:
      description: Entity the chosen template displays.
      the: dom.event.current-target.dataset/entity
      as: entity
    model:
      description: Model concept the chosen template uses.
      the: dom.event.current-target.dataset/model
      as: entity
    view:
      description: View concept the chosen template uses.
      the: dom.event.current-target.dataset/view
      as: entity
    marker:
      description: Per-command marker (data-template) distinguishing this transient.
      the: dom.event.current-target.dataset/template
      as: entity
    prevent-default:
      description: Stop the form submission from reloading the page.
      the: dom.event.do/prevent-default

# Point the space root at the applied template's composition. The head's
# `this` unifies to the singleton `id:tonk/root`, the fields come from the
# command.
rule!:
  description: Points the space root at the applied template's composition.
  assert!: root
  when:
    - assert: apply-template
      where:
        this: ?cmd
        entity: ?entity
        model: ?model
        view: ?view
    - assert: ==
      where:
        this: ?this
        is: id:tonk/root
```

Verify the `==` unification head shape against the existing create-sheet rules (~line 1430), which use the same `assert: ==` / `this`/`is` pattern.

- [ ] **Step 5: Run the regression gate**

Run: `cargo nextest run -p tonk-analyzer it_resolves_the_seeded_standard_library`
Expected: PASS. If it fails, the resolver error names the missing/unresolved attribute — fix the new notation (most often an undefined `the:` namespace or a mis-shaped rule head) and rerun.

- [ ] **Step 6: Commit**

```bash
jj commit -m "feat(core): add tonk:root pointer and apply-template command

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Sheets template package (single-entity wrapper over the sheet directory)

Wraps the existing sheet directory in one addressable template entity, so the uniform root delegation (`entity` + `model` + `view`) can mount it. The existing `tonk:sheet`, `tonk:sheet-binder`, and sheet commands/rules are untouched — they become the template's internals.

**Files:**
- Modify: `rust/tonk-core/assets/library/core.yaml` (add a clearly-commented "sheets template" section near the `tonk:sheet-binder` view ~line 1816)

**Interfaces:**
- Consumes: the `tonk:sheet` concept and `tonk:sheet-binder` directory view (existing).
- Produces:
  - concept `tonk:sheets-page` (a single marker field so an instance is matchable).
  - pinned alias `id:tonk/sheets-page` → the singleton page entity.
  - view `tonk:sheets-page-view` (entity view for `tonk:sheets-page`) that embeds `<tonk-display model=tonk:sheet>` (directory → the existing sheet-binder view).
  - a seeded `tonk:sheets-page` instance so the template renders once when applied.

- [ ] **Step 1: Add the sheets-page concept, alias, and a seeded instance**

```yaml
# ── Sheets template ──────────────────────────────────────────────────
# The sheets layout as an appl-able template: one addressable page entity
# whose view embeds the sheet directory. Applying the sheets template
# points the space root at this entity (model `tonk:sheets-page`, view
# `tonk:sheets-page-view`). The binder/sheet machinery below is the
# template's internals, no longer the mandatory space shell.
concept!: &sheets-page
  this: tonk:sheets-page
  description: The sheets layout as a single applicable template entity.
  with:
    kind:
      description: Marker field so the page entity is matchable.
      the: xyz.tonk.sheets-page/kind
      cardinality: one
      as: entity

name!:
  this: id:tonk/sheets-page
  entity: tonk:sheets-page

# Seed the single sheets-page instance so the template renders when
# applied. `kind` is a self-describing marker.
sheets-page!:
  this: id:tonk/sheets-page
  kind: tonk:sheets-page
```

Verify the instance-assertion form (`sheets-page!:` with `this:` an alias) against an existing seeded instance such as the create-sheet `empty-artifact!` rule head shapes; adjust if instances must anchor differently.

- [ ] **Step 2: Add the sheets-page entity view embedding the sheet directory**

```yaml
# The sheets-page view: render the existing sheet directory. `<tonk-display
# model=tonk:sheet>` with no entity is directory mode, resolving the
# `tonk:sheet-binder` view — the same tab strip + panels as before.
view!:
  this: tonk:sheets-page-view
  model: tonk:sheets-page
  display: |
    <tonk-display model=tonk:sheet></tonk-display>
```

- [ ] **Step 3: Run the regression gate**

Run: `cargo nextest run -p tonk-analyzer it_resolves_the_seeded_standard_library`
Expected: PASS. Fix any unresolved namespace/field the resolver names, rerun.

- [ ] **Step 4: Commit**

```bash
jj commit -m "feat(core): wrap the sheet directory as an applicable sheets template

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Launchpad empty-state fragment

The chrome shown when the root pointer is unset: a "paste a link to an agent" prompt and a "choose a template" control that applies the sheets template. Authored as a standalone fragment here; Task 4 mounts it in the root view's empty slot.

**Files:**
- Modify: `rust/tonk-core/assets/library/core.yaml`

**Interfaces:**
- Consumes: `tonk:apply-template` (Task 1), `id:tonk/sheets-page` + `tonk:sheets-page` + `tonk:sheets-page-view` (Task 2).
- Produces: a reusable launchpad markup fragment (kept inline in the root view in Task 4; defined here as the exact markup to use).

- [ ] **Step 1: Define the launchpad markup**

This is the markup Task 4 places in the root display's empty slot. The template-picker button submits `apply-template` with the sheets template triple on its dataset; `data-template` is the marker the command reads.

```html
<div class="launchpad" data-state-only="empty">
  <p class="launchpad__lead">This space is empty.</p>
  <p class="launchpad__hint">Paste a link into an agent to start working, or choose a template.</p>
  <form
    id="apply-template-form"
    data-template="tonk:apply-template"
    data-entity="id:tonk/sheets-page"
    data-model="tonk:sheets-page"
    data-view="tonk:sheets-page-view"
    onsubmit=tonk:apply-template
  ></form>
  <button class="launchpad__template" type="submit" form="apply-template-form">Sheets</button>
</div>
```

The `data-*` attributes live on the FORM, not the button: on submit the command reads `dom.event.current-target.dataset/*`, and the current-target is the submitted form. This mirrors `share-form` (`<form id="share-form" data-invite="tonk:invite" onsubmit=tonk:invite>`, shell view ~line 1770), which carries its marker on the form. When more templates are added later (follow-up specs), per-choice selection will need either a form per choice or a different event wiring — out of scope here (single sheets template).

- [ ] **Step 2: Verify the empty-state gating mechanism**

The launchpad must show only when the root `<tonk-display>` is empty. Two existing mechanisms:
- `<tonk-display>`'s `data-state="empty"` plus a child gated via the `tonk-display`-fallback element (`rust/tonk-display/src/fallback.rs`), OR
- a `slot="no-entity"` child (used by the sync chip and share dialog `<tonk-display>`s).

Read both and pick the one that fires for a *directory/entity display with zero matching instances* (the root pointer unset case). Record the chosen attribute/slot; Task 4 uses it verbatim. The `data-state-only="empty"` attribute above is a placeholder name — replace it with the real gating attribute/slot you confirm here.

- [ ] **Step 3: Commit the note**

No notation lands standalone in this task (the fragment is mounted in Task 4). If you added scratch CSS/markup, revert it. Record the confirmed gating mechanism in the plan margin or commit message of Task 4. Skip a commit if nothing changed.

---

### Task 4: Root shell view — topbar + dialogs + delegation + launchpad

The new directory view for `tonk:root`. It carries the relocated topbar and dialogs (decoupled from the binder) and delegates to the active template via dynamic binding, with the launchpad in the empty slot.

**Files:**
- Modify: `rust/tonk-core/assets/library/core.yaml` (add the new `view/directory!` for `model: tonk:root`; relocate the chrome out of the existing `tonk:shell` view ~lines 1678-1813)

**Interfaces:**
- Consumes: `tonk:root` (Task 1), launchpad fragment (Task 3), the topbar/dialog markup currently in `tonk:shell` (~1678-1813), the sync/identity/share/pause/enable-sync wiring (unchanged).
- Produces: directory view `tonk:root-shell` for `model: tonk:root`.

- [ ] **Step 1: Create the root shell view by relocating the chrome**

Add a `view/directory!` with `this: tonk:root-shell`, `model: tonk:root`. Move into it, VERBATIM, the chrome currently inside the `tonk:shell` view (current core.yaml ~lines 1546-1808):
- the `<style>` block (sync chip CSS etc.),
- the `<div class="workspace__topbar">…</div>` (brand, crumb, repo title, sync chip, spacer, identity chip, share button),
- the three `<wa-dialog>` blocks (enable-sync, pause-sync, share-repo) and the `pause-sync-form` / `share-form` forms.

Then REPLACE the old binder mount line (`<tonk-display model=workspace/sheet data-active={active} />`, ~line 1811) with the delegation + launchpad:

```html
<!-- The editable root composition. The root pointer carries the active
     template's entity/model/view; mount it through the same dynamic
     <tonk-display> binding the sheet mount uses. When the pointer is
     unset the display is empty and the launchpad shows. -->
<tonk-display entity={entity} model={model} view={view}>
  <!-- LAUNCHPAD: paste the Task 3 fragment here, using the empty-state
       gating attribute/slot confirmed in Task 3 Step 2. -->
</tonk-display>
```

Keep the outer `<div class="workspace-directory">` wrapper and its flex-fill `<style>` so the topbar pins and the composition fills — the same layout the old shell used.

- [ ] **Step 2: Confirm the dynamic-binding precedent**

The `entity={entity} model={model} view={view}` binding must read the root instance's fields. Confirm against the sheet mount, which binds `entity`/`model`/`view` from sheet fields the same way (the sheet's `tonk:sheet` concept fields feed `<tonk-display entity={entity} model={model} view={view}>`). If directory-mode field binding needs the row root anchored (see the known single-occurrence `{field}` gotcha), anchor the bindings on the delegation element as the row root.

- [ ] **Step 3: Run the regression gate**

Run: `cargo nextest run -p tonk-analyzer it_resolves_the_seeded_standard_library`
Expected: PASS. The old `tonk:shell` view still exists at this point (it is removed in Task 5); both resolving is fine.

- [ ] **Step 4: Commit**

```bash
jj commit -m "feat(core): add tonk:root shell view with relocated topbar and launchpad

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Cutover — repoint tonk/space and delete the old shell

The behavioral switch: `tonk/space` resolves to `tonk:root`, fresh repos open to the launchpad, and the old binder-welded shell is deleted.

**Files:**
- Modify: `rust/tonk-core/assets/library/core.yaml`

**Interfaces:**
- Consumes: everything from Tasks 1-4.
- Produces: `tonk/space` → `tonk:root`; removal of `tonk:shell`.

- [ ] **Step 1: Repoint the `tonk/space` alias**

Change the alias (current ~lines 1394-1396) from `tonk:binder` to `tonk:root`:

```yaml
# tonk/space resolves to the root composition pointer (was tonk:binder,
# the mandatory sheets shell). The space now renders one editable root:
# launchpad when unset, the applied template when set.
name!:
  this: id:tonk/space
  entity: tonk:root
```

- [ ] **Step 2: Delete the old `tonk:shell` view**

Remove the entire `view/directory!` `this: tonk:shell` block (the chrome was relocated in Task 4; the binder mount is replaced by the delegation). Leave the `tonk:sheet-binder` view and all sheet concepts/commands/rules in place — they are the sheets template's internals now.

- [ ] **Step 3: Confirm fresh-repo default is the launchpad**

The standard seed must NOT assert a `tonk:root` instance with layout fields and must NOT apply any template. Confirm no seed block asserts `tonk:root` entity/model/view. Result: a fresh repo has zero matching `tonk:root` instances → root display empty → launchpad.

- [ ] **Step 4: Run the regression gate**

Run: `cargo nextest run -p tonk-analyzer it_resolves_the_seeded_standard_library`
Expected: PASS. If the resolver reports a dangling reference to `tonk:binder` or `tonk:shell`, search the file for remaining references and remove/repoint them.

- [ ] **Step 5: Commit**

```bash
jj commit -m "feat(core): render the space as a generalized root, not the binder shell

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: Simplification sweep, demo seed, native lint

Removes residual coupling, keeps the bundled demo opening to sheets, and confirms the native build is clean.

**Files:**
- Modify: `rust/tonk-core/assets/library/core.yaml` (residual cleanup only)
- Modify: `rust/tonk-core/assets/library/demo.yaml` (apply the sheets template for the `home` demo)

**Interfaces:**
- Consumes: the sheets template (Task 2), `tonk:root` (Task 1).

- [ ] **Step 1: Sweep for dead binder-as-shell coupling**

Search core.yaml for `tonk:binder`, `tonk:shell`, `workspace/sheet data-active`, and `tonk/binder`. Every remaining reference must be either (a) inside the sheets-template internals (binder/sheet behavior — keep), or (b) genuinely dead shell wiring (remove). Add a one-line comment at the `tonk:sheet-binder` view marking the deferred `site.replica` local-replica scoping (per Global Constraints), e.g.:

```yaml
    # NOTE: the binder still renders one panel per stored replica entity.
    # Scope to the local replica once site.replica is available; until then
    # the launchpad-default keeps this off the fresh-repo path.
```

- [ ] **Step 2: Apply the sheets template in `demo.yaml`**

So the bundled `home` demo still opens to sheets rather than the bare launchpad, assert the root pointer directly in `demo.yaml`:

```yaml
# The home demo opens to the sheets layout. Point the space root at the
# sheets template entity (fresh user repos stay on the launchpad).
root!:
  this: id:tonk/root
  entity: id:tonk/sheets-page
  model: tonk:sheets-page
  view: tonk:sheets-page-view
```

Confirm `demo.yaml` is concatenated after `core.yaml` for the home seed (the seeding flow in `rust/tonk-worker/src/router/repository.rs` fetches both for the default repo), so `id:tonk/sheets-page` is defined when this asserts.

- [ ] **Step 3: Run the regression gate over both libraries**

Run: `cargo nextest run -p tonk-analyzer it_resolves_the_seeded_standard_library`
Expected: PASS. If you added a demo-resolves test, run it too. If `analyze_local` does not see `demo.yaml`, validate the concatenation resolves by parsing `core.yaml + "\n" + demo.yaml` in a scratch test and asserting `analyze_local` is ok.

- [ ] **Step 4: Native lint**

Run: `cargo clippy --all -- -D warnings`
Expected: clean. No Rust changed in this plan, so this guards against incidental breakage and confirms no wasm-only helper became native-dead.

- [ ] **Step 5: Commit**

```bash
jj commit -m "chore(core): retire binder-as-shell coupling; demo opens to sheets

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 7: Render validation and acceptance

`analyze_local` proves the notation resolves; it does not prove it renders. This task runs the app and walks the acceptance checklist.

**Files:** none (validation only).

- [ ] **Step 1: Build and run the app**

Use the project's run path (see the `run` skill / project launch docs). Build the wasm bundle so the source `core.yaml` is rebuilt into `rust/tonk-ui/dist/library/core.yaml`. Confirm the dist copy reflects your source edits (it is build-generated — never hand-edited).

- [ ] **Step 2: Acceptance checklist**

Verify each:
- Create a fresh repo and open its space → the **launchpad** shows ("paste a link to an agent" / "choose a template"), not a binder.
- Click the **Sheets** template → the root renders the sheet binder; creating/activating/closing tabs works exactly as before.
- The **topbar** (brand, `< all repos`, editable repo name, sync chip, identity chip, share) renders above the composition in both the launchpad and the sheets states, and persists across applying the template.
- The **home demo** opens directly to sheets (via the `demo.yaml` root assertion).
- Editing the page composition cannot remove the topbar (it lives in the root shell view, not the applied template).

- [ ] **Step 3: Record known interim limitations**

In the PR description, note the two deferred seams from Global Constraints: the topbar moves to the `route/space` view when conceptual routing lands, and the binder gains `site.replica` local-replica scoping then. Neither is a regression — fresh repos default to the launchpad.

- [ ] **Step 4: wasm render tests (if the harness exists)**

If there are wasm-gated seed/render tests (per the repo's testing skill), run them under the Safari/Chrome automation route. If none cover the root/launchpad path and the harness is available, add one asserting the root display mounts the launchpad when no `tonk:root` instance matches. Otherwise document that render coverage is manual for now.

- [ ] **Step 5: Final commit / PR**

```bash
jj bookmark create generalized-root-view -r @-
jj git push -b generalized-root-view
# then open the PR with gh, body noting the interim seams from Step 3
```

---

## Self-Review Notes (for the implementer)

- **Spec coverage:** generalize root (Tasks 1,4,5), launchpad entry (Tasks 3,4,5), extract topbar — interim decouple (Task 4) with full route-view move deferred (Global Constraints), sheets-as-first-template (Tasks 2,6), simplification map realized (Tasks 5,6).
- **The two genuinely uncertain mechanics**, both with named precedents to copy: (a) the singleton root that renders once and falls to the launchpad when its layout fields are absent — verify against `id:tonk/space` aliasing and the `data-state="empty"`/`no-entity` slot behavior; (b) dynamic `entity/model/view` delegation — verify against the sheet mount. If (a) does not behave (e.g. an instance with missing required fields errors instead of simply not matching), fall back to: seed a bare `tonk:root` instance carrying only a marker field, gate the launchpad on the *layout* fields being absent via the `no-entity` slot on the inner delegation `<tonk-display>`.
- **Naming:** no new `workspace` concept (tonk-layout collision) — used `tonk:root`, `tonk:sheets-page`, `tonk:apply-template`.
