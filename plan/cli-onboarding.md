# CLI onboarding implementation plan

**Goal:** Make a blank-slate agent's first hour succeed on the nominal command/projection model, by removing every documented path that is factually broken and routing the remaining surfaces toward one canonical, executable example.

**Approach:** Establish the list-append example already in `tonk guide events` as the single source of truth, prove it executable by evaluating the shipped binary's own guide text, then delete or correct every surface that teaches a contradictory model. Extend `tonk`'s routing and the nominal loop's terminal steps so the example ends in something visible and verified rather than in a valid-but-invisible projection.

**Constraints:**

- `rust/tonk-cli/src/guide-events.md` is already correct and nominal-first. It is the reference the other surfaces are corrected *against*, not a file to rewrite.
- The embedded guides must stay self-contained. An agent in a sandbox with no repo access reads only `tonk guide`, so the canonical example stays inline in the guide text rather than moving to a file the binary does not carry.
- `tonk.context.v1` stays. Additive fields do not bump the version; the version-change rule gets documented instead (see "Context contract").
- Compatibility structural `dom.event/*` remains supported in the runtime. This plan changes what documentation *recommends*, never what the runtime *accepts*.
- No new dependencies. The example-extraction helper is an `awk` invocation, shared verbatim by the Rust test and the bench episode.
- Findings 6 (full docs/CLI contract tests) and 7 (bench exit-code and rubric enforcement) are out of scope; see "Deferred".

## Decisions

Three decisions were taken before planning and are settled:

1. **Delete `guide/` entirely.** The mdBook is an unowned hand-written fork: two commits ever (`0932fa66d`, `f9d906132`), no CI builds or deploys it, served only by `mdbook serve` in `flake.nix`. It is the staleest surface precisely because it forked material the binary already carries. `tonk guide` becomes the only reference.
2. **Findings 1–5 are one PR-sized slice.** Findings 6 and 7 become follow-up plans.
3. **Keep `tonk.context.v1` and write the version policy.** The string's only consumer is `rust/tonk-cli/tests/context.rs:58`. Bumping on a purely additive change would break strict pinners while helping no one; the discipline is preserved by stating when the version *does* change.

## Canonical example

The list-append block in `guide-events.md:22-98` becomes the one definition. It currently exists in at least three drifted copies:

| Copy | Divergence |
|------|------------|
| `guide-events.md:22-98` | canonical; uses `xyz.tonk.todo/*` URIs |
| `bench/scenarios/list-append/scripted.sh:8-77` | same shape, `bench.todo/*` URIs, **plus** a trailing `tonk home todo/list` |
| `rust/tonk-cli/tests/{project,commands,schema_read}.rs` | per-test fragments |

Two substantive facts fall out of that table. The bench copy has drifted to a different URI namespace, and the bench copy carries a `tonk home todo/list` step the guide omits — so an agent who copy-runs the guide gets a working projection with **no visible home**. Fix both by making the guide's block the only copy and adding the missing terminal step to it.

**Mechanism.** The guide text is shipped inside the binary, so the binary can hand its own example back:

```sh
tonk guide events | awk '/^```yaml$/{n++} n==1 && !/^```/{print} /^```$/{if(n==1)exit}'
```

Add that as `bench/bin/guide-example.sh` (or a small shared function). Then:

- `bench/scenarios/list-append/scripted.sh` pipes it into `tonk eval -` instead of inlining its own copy. The bench now proves the *shipped binary's own guide* evaluates and produces a working app — which is the executable-contract property finding 6 wants, obtained here for one example at near-zero cost.
- A new test in `rust/tonk-cli/tests/notation.rs` (`mod when_serving_the_guide`) extracts the same block from `guide::EVENTS`, evaluates it against a fresh `TestSite`, and asserts the transaction commits. This is the minimum that makes "executable" true; the broader per-example contract suite stays deferred.
- The bench verifier's expected URIs move from `bench.*` to whatever the guide declares. `verify.sh` already pins `id:todo-list` and `id:todo/add`, which are anchor-derived and unaffected by the attribute-URI change.

The guide block also gains the missing terminal step, so the example ends visible:

```sh
tonk eval todo.notation
tonk home todo/list          # the step the guide currently omits
tonk render todo/list        # headless check
```

## Delete the mdBook

- Remove `guide/` (`book.toml`, `src/`, `mermaid.min.js`, `mermaid-init.js`) — 2.6MB, of which the four `src/*.md` files are the stale content: `example.md:75` teaches `dom.event.current-target.dataset/counter`, `reference.md:12` defines commands as "transient, event-fed", `reference.md:70-72` is a `dom.event.*` source table, `the-model.md:20-24` frames commands as transient facts.
- Remove the `mdbook serve` block in `flake.nix:211-229` (the `GUIDE_PORT=3001` / `pkill` / `serve` lines and `GUIDE_PID`), plus any later `GUIDE_PID` teardown in the same shell hook.
- Remove `mdbook` and `mdbook-mermaid` from the devshell package list (`flake.nix:67-68`) if nothing else uses them — grep before removing.
- Remove the `/guide/` `[[proxies]]` entry from `Trunk.toml`.
- Grep for and fix inbound links to `/guide/` in the web shell and README before deleting, so nothing 404s.

## Correct the embedded guides

Three files still point at the compatibility model. Each is a small, surgical edit — the nominal replacement is `onchange=<projection>` plus a `projection!:` with a `detail:` or `target:` source, all already documented in `guide-events.md:110-124`.

| Location | Current | Change |
|----------|---------|--------|
| `guide-views.md:152-157` | editors persist by "fire a command … (read `dom.event.detail/…`)" | describe the projection: `onchange=<projection>` with `{ detail: "value" }` |
| `guide-views.md:200-203` | "`dom.event.detail/amount` fields on the command" | `{ detail: "amount" }` argument source on the projection |
| `guide-element-tonk-code.md:19-22` | "read it in the command with `dom.event.detail/value`" | `{ detail: "value" }` source |
| `guide-index.md:26-27` | rule trigger described as "a transient command fact produced by a DOM event" | "a nominal command invocation projected from a DOM event" |

**Terminology collision to resolve in `guide-workspace.md`.** Line 28 calls the workspace commands "`command!:` transients", and line 31 calls `workspace/active-sheet` "the durable `{active}` projection a rule writes". The second sense of "projection" now collides head-on with `projection!:`, which is an event-to-argument mapping and never durable. Reword 31 to "the durable `{active}` fact a rule writes"; reword 28 to drop "transients". Whether the workspace's own declarations get migrated off the compatibility path is a separate question and not in this slice.

`guide.rs` needs no change: once the four files above are corrected, `tonk guide all` (`guide.rs:59-68`) stops concatenating two models, because there is only one left.

## Fix the READMEs

**The account wall comes first.** Both quickstarts open with `tonk spot new`, which fails on a clean machine:

```
error: failed to initialize site: failed to bootstrap repository 'main':
A Tonk account is required; run `tonk account link`
```

The root `README.md` never mentions `tonk account link` anywhere. Add it as the first step of the quick start, before `tonk spot new`.

`README.md:103-110` — replace the quick start. `tonk eval -c 'person:'` exits 4 on a fresh spot (verified), because `person` is undeclared; a query against an undeclared concept is not a "hello world". Replace with the canonical path: account link, spot new, `concept add`, `assert`, `view add`, `render`.

`rust/tonk-cli/README.md`:

- line 22 area: same undeclared-`person:` problem, same replacement.
- line 42: **`tonk get habit <entity>` does not exist.** There is no `Get` variant in `tonk.rs`'s subcommand enum. Delete the line; `tonk query habit <entity>` is the real form and is already documented two lines up.
- Note `tonk account link` as a prerequisite here too, above `tonk spot new` at line 18.

## Bare `tonk` routing

Root help (`tonk.rs:35`) currently routes only to `context`, `query`, and `assert` — nothing points at `project`, `commands`, `guide events`, `view add`, or `home`. An agent that needs interactivity has no signal it exists.

Extend `after_help` to three named tasks rather than a flat command list: ordinary CRUD (`query`/`assert`), make something visible (`concept add` → `view add` → `render`), add interactivity (`tonk guide events` → `project`).

Extend `ContextReport::render_markdown` (`context.rs:243-320`) with the same routing, and **drop the redundant final step** from `empty_spot_workflow` (`context.rs:220-225`): `tonk home note` immediately follows `tonk view add note`, which already auto-surfaces an unset home (`data_ops.rs:516-521`, documented in its own doc comment). `tests/context.rs:66` pins `empty_spot_workflow[0]`, which is `concept add`, so removing index 3 is safe.

### Context contract

Add an interactivity lane to the JSON so it stays authoritative — `context.rs:314` advertises `tonk context --json` as "the complete contract", and routing that exists only in the text would make that line false.

Keep `schema_version = "tonk.context.v1"`. Document the rule on `SCHEMA_VERSION` (`context.rs:17-18`):

> The version changes only when a field is removed or its meaning reinterpreted. Consumers must ignore unknown fields; additive growth keeps the version.

Add a test asserting an unknown field does not break a consumer's deserialization, so the tolerance half of that contract is enforced rather than assumed.

## Complete the nominal loop

`tonk help project` has **no** `after_help` at all — no fixture shape, no example — while `guide-index.md` promises "Each subcommand also carries examples". Same for `tonk commands`. An agent can therefore reach `tonk project` without ever learning the fixture format.

- Add `after_help` to `Project` (`tonk.rs:125-143`) showing a minimal fixture inline and the read-only → `--transact` progression.
- Add `after_help` to `Commands`/`Inventory` (`tonk.rs:144-154`).
- Correct the `Schema` doc comment (`tonk.rs:110-114`): it claims "Every named attribute and concept", but `schema::render` also emits commands (`schema.rs:158-161`) and projections (`schema.rs:162-166`). Update the help and the `render` doc comment (`schema.rs:133-136`), which has the same omission.
- Fix the anchor rule in `rust/tonk-notation/guide.md:124-127`. It states anchor names permit no `/`, which forbids the canonical `&todo/add` and `&todo/item` the guide, the bench, and the runtime all use. Establish which is true — the analyzer accepts `&todo/add`, so the guide text is the error — and correct the guide to describe the rule the analyzer actually enforces.

The loop the slice must leave working end to end, discoverable from `tonk` alone: discover → declare → dry-run `project` → `--transact` → `query` the durable result → `home` → `render` → mounted check.

## Verification

- `cargo fmt --check` and workspace `cargo clippy --all-targets --all-features`; `nix flake check` for the gate.
- New test: the guide's extracted example evaluates and commits against a fresh `TestSite`.
- New test: unknown-field tolerance on `ContextReport`.
- `tonk-cli` test suite, especially `tests/context.rs` (step removal), `tests/notation.rs`, `tests/project.rs`, `tests/commands.rs`, `tests/schema_read.rs`.
- Run the list-append bench scenario scripted, confirming it still passes once `scripted.sh` sources the example from `tonk guide events` and the URI namespace changes. Note that a bench pass currently proves less than it appears to — see Deferred.
- Manually walk the corrected root README quick start on a spot-isolated `TONK_SPOTS_STATE`, from `account link` forward, and confirm every command succeeds in order.
- `grep -rn 'dom\.event' rust/tonk-cli/src/*.md` returns only intentional compatibility mentions (`guide-events.md:191-196`).
- `grep -rn '/guide/'` finds no live inbound links after the mdBook deletion.

## Deferred

**Finding 6 — executable doc contracts.** This slice makes *one* example executable. The general problem stands: `tests/notation.rs:429-441` asserts only substring presence (`contains("rule!")`), and the example is duplicated across 14 files. A follow-up should generalize the extract-and-evaluate harness to every fenced example in every embedded guide.

**Finding 7 — the bench does not gate.** `bench/bin/run.sh` captures `episode_status`, `bridge_status`, `interaction_status`, and `verifier_status` into JSON files (lines 157, 161, 171, 179) and then exits 0 regardless (lines 190-195). A failed episode currently reports as a completed run. Separately, `rubric.md:5-12` requires no-navigation, a clean console, and consumer isolation; `verify.sh` checks the durable count, `browser.passed`, and claim shape, and none of the other three. Both are independently testable and neither blocks this slice, but until they land, "the bench passes" is weak evidence.

**Bench artifact hygiene.** Retained `chrome-profile/chrome_debug.log` files contain complete `http://127.0.0.1:8787/join?access=…` invite URLs. The exposure is low — an ephemeral local bench worker, and `bench/runs/` is gitignored (`.gitignore:60`), so nothing was committed or published — but the logs should be scrubbed or excluded from retention.

**Finding 8 — unscripted measurement.** No blank-slate trial is proposed here. Once this slice lands, a three-episode unscripted run would measure whether the routing changes reduce calls-to-first-working-app, but that needs finding 7 first to produce a trustworthy result.
