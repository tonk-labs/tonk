# Authoring Verbs + Auto-Surface Implementation Plan (CLI PR3)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the render-gap end to end: mutating data verbs auto-sync to the upstream, `tonk concept add` authors anchored schemas with typed/enumerated flags, `tonk view add` authors declarative views, and `tonk home <model>…` (plus `view add`'s auto-surface) re-points the `tonk/space` alias via the verified root-concept recipe so an agent's build actually lands on the space home.

**Architecture:** Same thin-front-end-over-eval pattern as the data verbs: pure notation builders (new `authoring.rs`), handlers in `data_ops.rs`-style modules, clap wiring in `bin/tonk.rs`. The auto-surface recipe is the one verified in `.superpowers/sdd/repoint-findings.md` (origin-keyed root concept + `<tonk-display model=X />` directory view + cardinality-one `name!` re-point). Mutating verbs route through `auto_sync::run_eval` so commits reach the upstream.

**Tech Stack:** Rust (`rust/tonk-cli`), clap 4, the existing `tonk_evaluator::evaluate` pipeline, `auto_sync`, `render` (for the end-to-end home test).

**Specs:** `docs/superpowers/specs/2026-07-08-agent-ergonomic-cli-design.md` (§Authoring verbs, §Auto-surfacing — the recipe and "Auto-surface + explicit override" decision) as amended by `docs/superpowers/specs/2026-07-13-dialog-native-verbs-design.md` (vocabulary; authoring stays noun-first). Ground truth for the recipe: `.superpowers/sdd/repoint-findings.md` (READ IT — the load-bearing facts section is the contract for Tasks 2/4).

## Global Constraints

- VCS is jj (colocated). Commit with `jj commit <paths> -m "…"` — never `git add`/`git commit`, never touch bookmarks (the controller moves `feat/agent-build`). Conventional Commits, scope `cli` (or `bench`/`docs`). No emojis anywhere.
- Commit-message footer on every commit:
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_01L8KJZ3gegT5ocgVztaGWwV`
- Test style: `#[dialog_common::test]`, `it_does_x`, `mod when_…`; shared fixtures in `tests/common.rs`; the crate sets `autotests = false` — a NEW test file needs its own `[[test]]` entry in `rust/tonk-cli/Cargo.toml` (name + path, matching the existing entries at lines 20–38).
- Per-task gate: `nix develop -c cargo test -p tonk-cli`, `nix develop -c cargo clippy -p tonk-cli --all-targets -- -D warnings`, `nix develop -c cargo fmt -p tonk-cli -- --check`. Full workspace gate in the final task.
- `#![warn(missing_docs)]` under `-D warnings`: every new pub item gets a doc comment.
- Vocabulary: user-facing copy says assert/retract/query; authoring verbs are noun-first (`tonk concept add`, `tonk view add`, `tonk home`); never "remove"/"delete" for retraction.
- Every error enumerates the fix (valid types, valid cardinalities, known concepts, the `--attr` grammar).
- The generated home recipe uses the STABLE identifiers `&space-home` (concept anchor), `space:home` (concept `this:`), `&space-home-view` / `id:space:home/view` (view), so repeat invocations overwrite rather than duplicate. Never name anything `workspace` (collides with tonk-layout).

---

### Task 1: Mutating data verbs auto-sync

The PR2 data verbs call `eval::run_against_site` directly, so `tonk assert`/`tonk retract` commit locally but never push — unlike `tonk eval`, which wraps commits in `auto_sync::run_eval` (pull-before / push-after when an upstream exists, `TONK_NO_SYNC` escape hatch). The parent spec's architecture claim ("the verbs inherit sync/commit semantics for free") requires the wrap. Reads (`query`/`get`/`instance_exists`/`schema_subset`) stay direct — they commit nothing.

**Files:**
- Modify: `rust/tonk-cli/src/data_ops.rs`
- Test: `rust/tonk-cli/tests/sync.rs` (new `mod when_asserting_with_an_upstream`; reuse `wire_sibling_upstream`/`upstream_revision` helpers already in that file)

**Interfaces:**
- Consumes: `auto_sync::run_eval(site, source, options, sync) -> Result<Outcome, EvalError>` and `auto_sync::enabled(no_sync_flag: bool) -> bool` (both existing, `src/auto_sync.rs`).
- Produces: `assert_op` and `retract` route their committing eval through `auto_sync::run_eval(site, source, options, auto_sync::enabled(false))`. Signatures unchanged. Tasks 3–4's authoring handlers use the same pattern.

- [ ] **Step 1: Write the failing test**

Add to `rust/tonk-cli/tests/sync.rs` (model on the existing `it_auto_pushes_the_commit_to_the_upstream` test; that file's helpers and `ATTRIBUTE_DECL`/`CONCEPT_DECL` imports are already set up — extend the `use crate::common::…` line if `CONCEPT_DECL` isn't imported):

```rust
mod when_asserting_with_an_upstream {
    use super::*;

    #[dialog_common::test]
    async fn it_auto_pushes_an_assert_to_the_upstream() -> Result<()> {
        let test = TestSite::new().await?;
        wire_sibling_upstream(&test).await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        let before = upstream_revision(&test).await?;

        tonk_cli::data_ops::assert_op(
            &test.site,
            "task",
            None,
            &[
                "--title".into(),
                "synced".into(),
                "--done".into(),
                "false".into(),
            ],
        )
        .await?;

        let after = upstream_revision(&test).await?;
        assert_ne!(
            before, after,
            "a committing assert must push to the upstream like eval does"
        );
        Ok(())
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `nix develop -c cargo test -p tonk-cli --test sync when_asserting_with_an_upstream`
Expected: FAIL on the `assert_ne!` — the upstream revision doesn't move because `assert_op` never pushes.

- [ ] **Step 3: Route the mutating verbs through auto_sync**

In `rust/tonk-cli/src/data_ops.rs`, add `use crate::auto_sync;` and replace the committing `eval::run_against_site(site, Source::Inline(doc), Options::default()).await?` calls in **`assert_op` (both the mint and supersede arms)** and **`retract`** with:

```rust
            let outcome = auto_sync::run_eval(
                site,
                Source::Inline(doc),
                Options::default(),
                auto_sync::enabled(false),
            )
            .await?;
```

(`enabled(false)` = no `--no-sync` flag on these verbs; the `TONK_NO_SYNC` env escape hatch still applies, and a branch with no upstream is a silent skip — so every existing no-upstream test is unaffected.) Do NOT touch `run_read`, `instance_exists`, or `schema_subset`. Add one line to `assert_op`'s and `retract`'s doc comments: "Commits sync to the upstream like `tonk eval` (pull-before / push-after; `TONK_NO_SYNC` opts out)."

- [ ] **Step 4: Run the tests to verify they pass**

Run: `nix develop -c cargo test -p tonk-cli --test sync` then the full crate `nix develop -c cargo test -p tonk-cli`
Expected: all PASS (data_verbs suite runs on no-upstream sites → silent skip, unaffected).

- [ ] **Step 5: Clippy + fmt, commit**

Run the per-task gate, then:

```bash
jj commit rust/tonk-cli/src/data_ops.rs rust/tonk-cli/tests/sync.rs -m "fix(cli): assert and retract auto-sync to the upstream like eval"
```

---

### Task 2: Authoring notation builders (`authoring.rs`)

Pure, unit-tested builders — no I/O. Three builders plus the type/cardinality vocabulary with enumerating errors.

**Files:**
- Create: `rust/tonk-cli/src/authoring.rs`
- Modify: `rust/tonk-cli/src/lib.rs` (add `pub mod authoring;` alongside the existing module list)

**Interfaces:**
- Consumes: nothing from the site — pure string building. Reuses nothing from `data.rs` (the quoting need here is identical; copy the private `quote_string` helper with a comment noting the deliberate duplication, matching how `data.rs`/`schema.rs` already each carry one).
- Produces (Tasks 3–4 call these exact signatures):

```rust
pub struct AttrSpec { pub field: String, pub type_name: String, pub cardinality: String }
pub enum AuthoringError { BadAttrSpec { raw: String }, BadType { raw: String, valid: Vec<&'static str> }, BadCardinality { raw: String }, EmptyTemplate }
pub fn parse_attr_spec(raw: &str) -> Result<AttrSpec, AuthoringError>
pub fn build_concept_decl(name: &str, description: Option<&str>, attrs: &[AttrSpec]) -> String
pub fn build_view_decl(anchor: &str, model: &str, template: &str) -> String
pub fn build_home_recipe(models: &[String]) -> String
```

- [ ] **Step 1: Write the failing unit tests**

In `rust/tonk-cli/src/authoring.rs`'s `#[cfg(test)] mod tests` (written first, alongside the skeleton):

```rust
    #[test]
    fn it_parses_an_attr_spec() {
        let spec = parse_attr_spec("title:text:one").unwrap();
        assert_eq!(
            (spec.field.as_str(), spec.type_name.as_str(), spec.cardinality.as_str()),
            ("title", "Text", "one")
        );
    }
    #[test]
    fn it_accepts_canonical_type_spellings_case_insensitively() {
        assert_eq!(parse_attr_spec("n:UnsignedInteger:one").unwrap().type_name, "UnsignedInteger");
        assert_eq!(parse_attr_spec("n:unsignedinteger:one").unwrap().type_name, "UnsignedInteger");
        assert_eq!(parse_attr_spec("n:boolean:many").unwrap().type_name, "Boolean");
    }
    #[test]
    fn it_rejects_an_unknown_type_enumerating_the_valid_ones() {
        let err = parse_attr_spec("n:string:one").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("Text") && msg.contains("Boolean"), "{msg}");
    }
    #[test]
    fn it_rejects_a_bad_cardinality() {
        let msg = format!("{}", parse_attr_spec("n:text:lots").unwrap_err());
        assert!(msg.contains("one") && msg.contains("many"), "{msg}");
    }
    #[test]
    fn it_rejects_a_malformed_spec() {
        let msg = format!("{}", parse_attr_spec("just-a-name").unwrap_err());
        assert!(msg.contains("<field>:<type>:<cardinality>"), "{msg}");
    }
    #[test]
    fn it_builds_an_anchored_concept_decl() {
        let attrs = vec![parse_attr_spec("title:text:one").unwrap()];
        let doc = build_concept_decl("note", Some("a note"), &attrs);
        assert!(doc.contains("attribute!: &note-title"));
        assert!(doc.contains("the:         xyz.tonk.note/title"));
        assert!(doc.contains("as:          Text"));
        assert!(doc.contains("concept!: &note"));
        assert!(doc.contains("title: note-title"));
    }
    #[test]
    fn it_builds_a_view_decl_with_a_stable_this() {
        let doc = build_view_decl("note-view", "note", "<b>{title}</b>");
        assert!(doc.contains("view!: &note-view"));
        assert!(doc.contains("this: id:note-view"));
        assert!(doc.contains("model: note"));
        assert!(doc.contains("<b>{title}</b>"));
    }
    #[test]
    fn it_builds_the_home_recipe_per_the_verified_shape() {
        let doc = build_home_recipe(&["habit".into(), "entry".into()]);
        assert!(doc.contains("concept!: &space-home"));
        assert!(doc.contains("this: space:home"));
        assert!(doc.contains("the: dialog.origin/subject"));
        assert!(doc.contains("view!: &space-home-view"));
        assert!(doc.contains("this: id:space:home/view"));
        assert!(doc.contains("<tonk-display model=habit />"));
        assert!(doc.contains("<tonk-display model=entry />"));
        assert!(doc.contains("this: id:tonk/space"));
        assert!(doc.contains("entity: space:home"));
    }
```

- [ ] **Step 2: Run to verify they fail** (`nix develop -c cargo test -p tonk-cli authoring` — module doesn't exist yet → compile failure)

- [ ] **Step 3: Implement**

Key content (full doc comments required; `//!` module doc explains these are the authoring-verb builders over the verified repoint recipe, citing `.superpowers/sdd/repoint-findings.md` by path):

```rust
/// Canonical `as:` type spellings the analyzer accepts, matching
/// `schema::type_to_notation`'s output (which `tonk schema` proves
/// re-submittable). Input is matched case-insensitively.
const VALID_TYPES: &[&str] = &[
    "Text", "Entity", "UnsignedInteger", "SignedInteger", "Float", "Boolean", "Symbol",
];

pub fn parse_attr_spec(raw: &str) -> Result<AttrSpec, AuthoringError> {
    let parts: Vec<&str> = raw.split(':').collect();
    let [field, ty, card] = parts.as_slice() else {
        return Err(AuthoringError::BadAttrSpec { raw: raw.into() });
    };
    let type_name = VALID_TYPES
        .iter()
        .find(|t| t.eq_ignore_ascii_case(ty))
        .ok_or_else(|| AuthoringError::BadType { raw: (*ty).into(), valid: VALID_TYPES.to_vec() })?;
    if !matches!(*card, "one" | "many") {
        return Err(AuthoringError::BadCardinality { raw: (*card).into() });
    }
    Ok(AttrSpec { field: (*field).into(), type_name: (*type_name).into(), cardinality: (*card).into() })
}
```

Error display texts (exact): `BadAttrSpec` → ``"--attr '{raw}' is malformed; expected <field>:<type>:<cardinality>, e.g. title:text:one"``; `BadType` → ``"unknown type '{raw}'; valid types: Text, Entity, UnsignedInteger, SignedInteger, Float, Boolean, Symbol"`` (join `valid`); `BadCardinality` → ``"unknown cardinality '{raw}'; valid: one, many"``; `EmptyTemplate` → ``"the view template is empty; pass --template <html> or --template-file <path>"``.

`build_concept_decl`: for each attr emit

```text
attribute!: &{name}-{field}
  description: "The {field} field of {name}."
  the:         xyz.tonk.{name}/{field}
  as:          {type_name}
  cardinality: {cardinality}
```

then `concept!: &{name}` with `description:` (quoted; default `"A {name}."` when `None` — the analyzer treats concept descriptions as optional but the schema-aware help reads them) and `with:` mapping each `{field}: {name}-{field}`.

`build_view_decl(anchor, model, template)`:

```text
view!: &{anchor}
  this: id:{anchor}
  model: {model}
  display: |
    {template lines, each indented 4 spaces}
```

`build_home_recipe(models)`: exactly the MINIMAL WORKING RECIPE from repoint-findings.md with `<ns>` = `space` — the origin-keyed `concept!: &space-home` (`this: space:home`, one `subject` field, `the: dialog.origin/subject`, `as: entity`, description on both the concept and the inline attribute — the inline attribute description is a hard analyzer requirement per findings §Recipe 4), then `view!: &space-home-view` (`this: id:space:home/view`, `model: space:home`, display = one `<tonk-display model={m} />` line per model, wrapped in `<section>` blocks with an `<h2>{m}</h2>` heading when there are 2+ models, bare single tag when one), then `name!:` with `this: id:tonk/space` / `entity: space:home`.

- [ ] **Step 4: Run to verify they pass**, then the full crate + clippy + fmt.

- [ ] **Step 5: Commit**

```bash
jj commit rust/tonk-cli/src/authoring.rs rust/tonk-cli/src/lib.rs -m "feat(cli): authoring notation builders for concept, view, and the home recipe"
```

---

### Task 3: `tonk concept add`

**Files:**
- Modify: `rust/tonk-cli/src/data_ops.rs` (new handler `concept_add`)
- Modify: `rust/tonk-cli/src/bin/tonk.rs` (new `Concept { … }` subcommand)
- Create: `rust/tonk-cli/tests/authoring.rs` + register `[[test]] name = "authoring" path = "tests/authoring.rs"` in `rust/tonk-cli/Cargo.toml`

**Interfaces:**
- Consumes: Task 2's `parse_attr_spec`/`build_concept_decl`/`AuthoringError`; `require_concept`; `schema::find_concept`; Task 1's auto-sync pattern.
- Produces: `pub async fn concept_add(site: &TonkSite, name: &str, attrs: &[String], description: Option<&str>) -> Result<String, DataOpError>`; a `DataOpError::Authoring(#[from] AuthoringError)` variant (AnalyzeError exit arm) and `DataOpError::ConceptExists { name: String }` (IoError arm, message: ``"concept '{name}' already exists; inspect it with `tonk schema {name}`"``).

- [ ] **Step 1: Write the failing integration tests** (`tests/authoring.rs`, `mod common;` at top like the other suites)

```rust
mod common;

use anyhow::Result;

use crate::common::TestSite;

mod when_adding_a_concept {
    use super::*;

    #[dialog_common::test]
    async fn it_authors_an_anchored_concept_usable_by_the_data_verbs() -> Result<()> {
        let test = TestSite::new().await?;
        tonk_cli::data_ops::concept_add(
            &test.site,
            "habit",
            &["name:text:one".into(), "target:text:one".into()],
            Some("a tracked habit"),
        )
        .await?;
        // The anchored concept is immediately usable end to end:
        // schema-aware assert, then query sees the instance.
        tonk_cli::data_ops::assert_op(
            &test.site,
            "habit",
            None,
            &["--name".into(), "Run".into(), "--target".into(), "5k".into()],
        )
        .await?;
        let out = tonk_cli::data_ops::query(&test.site, "habit", false).await?;
        assert!(out.contains("Run"), "authored concept round-trips:\n{out}");
        Ok(())
    }

    #[dialog_common::test]
    async fn it_rejects_an_existing_concept_name() -> Result<()> {
        let test = TestSite::new().await?;
        tonk_cli::data_ops::concept_add(&test.site, "habit", &["name:text:one".into()], None)
            .await?;
        let err = tonk_cli::data_ops::concept_add(
            &test.site,
            "habit",
            &["name:text:one".into()],
            None,
        )
        .await
        .unwrap_err();
        assert!(format!("{err}").contains("already exists"), "{err}");
        Ok(())
    }

    #[dialog_common::test]
    async fn it_enumerates_valid_types_on_a_bad_attr() -> Result<()> {
        let test = TestSite::new().await?;
        let err = tonk_cli::data_ops::concept_add(
            &test.site,
            "habit",
            &["name:string:one".into()],
            None,
        )
        .await
        .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("UnsignedInteger") && msg.contains("Text"), "{msg}");
        Ok(())
    }
}
```

- [ ] **Step 2: Verify they fail** (compile error — `concept_add` missing).

- [ ] **Step 3: Implement `concept_add`**

In `data_ops.rs`: parse every `--attr` via `parse_attr_spec` (collect the first error), reject an existing name via `schema::find_concept(site, name)` → `ConceptExists`, build the doc with `build_concept_decl`, run it through the Task 1 auto-sync pattern, and return ``format!("asserted concept {name} ({n} fields)\nnext: tonk assert {name} --help\n{}", outcome.stdout)``.

- [ ] **Step 4: Wire the CLI**

New subcommand in `bin/tonk.rs` (noun-first — mirrors `Remote`/`Share`):

```rust
    /// Author schema: concepts and their attributes.
    Concept {
        #[command(subcommand)]
        command: ConceptCommand,
    },
```

```rust
#[derive(Subcommand, Debug)]
enum ConceptCommand {
    /// Assert a new concept with typed attributes. Attributes are
    /// anchored (`&{concept}-{field}`), so the concept and its
    /// fields resolve by name immediately — `tonk assert <name>
    /// --help` shows the typed flags right after this succeeds.
    #[command(
        after_help = "Types: text, entity, unsigned-integer... run with a bad type to see the list.\n\nExamples:\n  tonk concept add habit --attr name:text:one --attr target:text:one --description \"a tracked habit\"\n  tonk concept add note --attr body:text:one --attr tag:text:many"
    )]
    Add {
        /// Name for the concept (also the anchor).
        #[arg(value_name = "NAME")]
        name: String,
        /// One field as `<field>:<type>:<cardinality>`; repeatable.
        #[arg(long = "attr", value_name = "FIELD:TYPE:CARD", required = true)]
        attrs: Vec<String>,
        /// Human description for the concept.
        #[arg(long, value_name = "TEXT")]
        description: Option<String>,
    },
}
```

`descriptor()` arm: `Command::Concept { command } => ("concept", Some(match command { ConceptCommand::Add { .. } => "add" }))`. Handler follows the `query_op` scaffolding, calling `data_ops::concept_add`.

- [ ] **Step 5: Verify green** (targeted suite, full crate, clippy, fmt).

- [ ] **Step 6: Commit**

```bash
jj commit rust/tonk-cli/src/data_ops.rs rust/tonk-cli/src/bin/tonk.rs rust/tonk-cli/tests/authoring.rs rust/tonk-cli/Cargo.toml -m "feat(cli): tonk concept add with typed, enumerated attribute flags"
```

---

### Task 4: `tonk view add` + auto-surface + `tonk home`

**Files:**
- Modify: `rust/tonk-cli/src/data_ops.rs` (handlers `view_add`, `home`, private `resolve_name` + `home_is_unset`)
- Modify: `rust/tonk-cli/src/bin/tonk.rs` (new `View { Add }` subcommand + top-level `Home`; extend the root `after_help` "Start here" block with one line: `tonk home <concept>   put a concept's directory on the space home`)
- Test: `rust/tonk-cli/tests/authoring.rs`

**Interfaces:**
- Consumes: Task 2's `build_view_decl`/`build_home_recipe`/`AuthoringError::EmptyTemplate`; `require_concept`; the auto-sync pattern; `eval::run_against_site` for pure queries; `site.repository.did()` for the URL line; `render::{render, RenderRoute}` for the end-to-end test.
- Produces:
  - `pub async fn view_add(site: &TonkSite, model: &str, name: Option<&str>, template: &str) -> Result<String, DataOpError>` — asserts the view (anchor = `name` or `{model}-view`); then, when the home alias is unset, auto-surfaces `model` via `home()` and appends its output.
  - `pub async fn home(site: &TonkSite, models: &[String]) -> Result<String, DataOpError>` — validates every model via `require_concept`, asserts `build_home_recipe(models)`, returns ``format!("home set: {models}\nlive at /space/{did}/\n{rest}", …)``.
  - Private `resolve_name(site, name: &str) -> Result<Option<String>, DataOpError>`: pure query `name:\n  this: id:{name}\n  entity: ?e\n` via `eval::run_against_site`, reading `matches_after` block `"name"` → first row's `entity` field (None when no rows). `home_is_unset`: `resolve_name("tonk/space")` is `None` or equals `resolve_name("tonk:blank")` (both-None counts as unset). Per repoint-findings, never trust an assertion's own echoed matches for the alias — always re-query.

- [ ] **Step 1: Write the failing tests** (append to `tests/authoring.rs`)

```rust
mod when_setting_the_home {
    use super::*;

    async fn seed_habit(test: &TestSite) -> Result<()> {
        tonk_cli::data_ops::concept_add(
            &test.site,
            "habit",
            &["name:text:one".into()],
            Some("a habit"),
        )
        .await?;
        tonk_cli::data_ops::assert_op(
            &test.site,
            "habit",
            None,
            &["--name".into(), "Run".into()],
        )
        .await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_repoints_the_space_alias_and_renders_the_data() -> Result<()> {
        let test = TestSite::new().await?;
        seed_habit(&test).await?;
        let out = tonk_cli::data_ops::home(&test.site, &["habit".into()]).await?;
        assert!(out.contains("/space/"), "home should print the live path:\n{out}");
        // End to end through the same resolution pipeline the browser
        // runs: the replica entity rendered at model tonk/space must
        // now show the habit data (repoint-findings recipe 3).
        let replica = tonk_cli::data_ops::query(&test.site, "tonk/replica", false).await?;
        let entity = replica
            .lines()
            .find_map(|l| l.trim().strip_prefix("this: ").map(str::to_owned))
            .expect("a fresh site has a replica entity");
        let route = tonk_cli::render::RenderRoute::parse(&format!("{entity}@tonk/space"))?;
        let html = tonk_cli::render::render(&test.site, &route).await?;
        assert!(
            html.contains("Run"),
            "the space home must render the habit directory:\n{html}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_errors_on_an_unknown_model() -> Result<()> {
        let test = TestSite::new().await?;
        let err = tonk_cli::data_ops::home(&test.site, &["nope".into()])
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("no concept named 'nope'"), "{err}");
        Ok(())
    }
}

mod when_adding_a_view {
    use super::*;

    #[dialog_common::test]
    async fn it_asserts_the_view_and_auto_surfaces_an_unset_home() -> Result<()> {
        let test = TestSite::new().await?;
        super::when_setting_the_home::seed_habit(&test).await?;
        let out = tonk_cli::data_ops::view_add(
            &test.site,
            "habit",
            None,
            "<b>{name}</b>",
        )
        .await?;
        assert!(out.contains("/space/"), "auto-surface should print the live path:\n{out}");
        Ok(())
    }

    #[dialog_common::test]
    async fn it_does_not_repoint_an_already_set_home() -> Result<()> {
        let test = TestSite::new().await?;
        super::when_setting_the_home::seed_habit(&test).await?;
        tonk_cli::data_ops::home(&test.site, &["habit".into()]).await?;
        let out = tonk_cli::data_ops::view_add(
            &test.site,
            "habit",
            Some("habit-alt"),
            "<i>{name}</i>",
        )
        .await?;
        assert!(
            !out.contains("home set:"),
            "an explicitly set home must not be re-pointed by view add:\n{out}"
        );
        Ok(())
    }
}
```

(Make `seed_habit` reachable from both mods — `pub(crate)` in `when_setting_the_home` or hoist it to file scope; implementer's choice, file scope is cleaner.)

- [ ] **Step 2: Verify they fail** (compile error — `home`/`view_add` missing). Note: the render-based test depends on `tonk/replica` being queryable through `data_ops::query` — if the concept name needs the `tonk/replica` spelling from repoint-findings and `require_concept` filters it as built-in or it's missing from `list_concepts`, fall back to a raw name-table/AttributeQuery lookup for the replica entity (see `schema.rs`'s `name_claims_by_entity` for the pattern) and record the substitution in the report. The RENDER assertion itself is the non-negotiable part of the test.

- [ ] **Step 3: Implement** `resolve_name`/`home_is_unset`/`home`/`view_add` in `data_ops.rs` per the Interfaces block. `home` output shape:

```rust
    let did = site.repository.did();
    Ok(format!(
        "home set: {}\nlive at /space/{did}/\n{}",
        models.join(", "),
        outcome.stdout
    ))
```

`view_add`: `require_concept(model)` → `EmptyTemplate` check → assert `build_view_decl` via auto-sync → if `home_is_unset(site).await?`, call `home(site, &[model.to_string()])` and append its output under a `"\n"` separator; else append ``"home already set; re-point it explicitly with `tonk home <concept>`\n"``.

- [ ] **Step 4: Wire the CLI**

```rust
    /// Author declarative views for a concept.
    View {
        #[command(subcommand)]
        command: ViewCommand,
    },

    /// Put one or more concepts' directories on the space home.
    /// Authors the origin-keyed root-concept recipe and re-points
    /// the `tonk/space` alias (cardinality-one — safe to re-run;
    /// each run replaces the home wholesale).
    #[command(after_help = "Examples:\n  tonk home habit\n  tonk home habit entry")]
    Home {
        /// Concept name(s) to surface, in order.
        #[arg(value_name = "CONCEPT", required = true)]
        models: Vec<String>,
    },
```

```rust
#[derive(Subcommand, Debug)]
enum ViewCommand {
    /// Assert a declarative view for a concept. When no home is set
    /// yet, the build is auto-surfaced onto the space home.
    #[command(after_help = "Examples:\n  tonk view add habit --template '<b>{name}</b>'\n  tonk view add habit --template-file card.html --name habit-card")]
    Add {
        /// The concept this view renders.
        #[arg(value_name = "CONCEPT")]
        model: String,
        /// Inline HTML template ({field} interpolation).
        #[arg(long, value_name = "HTML", conflicts_with = "template_file", required_unless_present = "template_file")]
        template: Option<String>,
        /// Read the template from a file instead.
        #[arg(long, value_name = "PATH")]
        template_file: Option<PathBuf>,
        /// Anchor name for the view (default: <concept>-view).
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
    },
}
```

Handlers follow the existing scaffolding (`--template-file` read via `tokio::fs::read_to_string`, error → `print_error`). `descriptor()`: `("view", Some("add"))`, `("home", None)`. Note existing `Views` (list) command stays as-is. Add the one-line `tonk home` mention to the root `after_help`.

- [ ] **Step 5: Verify green** (full crate, clippy, fmt). The render test is the gate: if it fails on the custom-view-vs-default sharp edge from repoint-findings (item template falls back to default), the assertion on the DATA (`"Run"`) should still hold — if even the data is absent, STOP and report with the rendered HTML.

- [ ] **Step 6: Commit**

```bash
jj commit rust/tonk-cli/src/data_ops.rs rust/tonk-cli/src/bin/tonk.rs rust/tonk-cli/tests/authoring.rs -m "feat(cli): tonk view add and tonk home auto-surface builds onto the space home"
```

---

### Task 5: Docs

**Files:**
- Modify: `rust/tonk-cli/README.md` (extend the data-verbs block with the authoring verbs)
- Modify: `.claude/commands/tonk.md` (new "Authoring" section between "Data verbs" and "Escape hatch")

**Interfaces:** consumes the shipped surface — verify every line against the built binary's `--help` before writing (`nix develop -c cargo build -p tonk-cli`).

- [ ] **Step 1: README** — after the retract lines in the data-verbs block, append:

```
# Authoring — schema, views, and the space home.
tonk concept add habit --attr name:text:one   # anchored concept + typed attributes
tonk view add habit --template '<b>{name}</b>'  # declarative view (auto-surfaces an unset home)
tonk home habit                               # put habit's directory on the space home
```

- [ ] **Step 2: .claude/commands/tonk.md** — insert this section after "Data verbs":

````markdown
## Authoring (schema, views, the space home)

```bash
tonk concept add <name> --attr <field>:<type>:<card> [--attr …] [--description <text>]
                                    # types: text, entity, unsigned-integer, …; card: one|many
tonk view add <concept> --template '<html>' | --template-file <path> [--name <anchor>]
tonk home <concept> [<concept> …]   # put concept directories on the space home
```

Notes:
- `concept add` anchors everything, so `tonk assert <name> --help` works
  immediately after.
- `view add` auto-surfaces your build onto the space home when no home is
  set yet; `tonk home` re-points it explicitly (safe to re-run — each run
  replaces the home).
- Writes sync to the upstream automatically (like `tonk eval`); set
  `TONK_NO_SYNC=1` to opt out.
````

- [ ] **Step 3: Commit**

```bash
jj commit rust/tonk-cli/README.md .claude/commands/tonk.md -m "docs(cli): document the authoring verbs and tonk home"
```

---

### Task 6: Full gates + interview-build re-baseline

**Files:**
- Modify: `bench/README.md`

- [ ] **Step 1: Full workspace gates** — `nix develop -c cargo test -p tonk-cli`, `nix develop -c cargo clippy --workspace --all-targets --all-features -- -D warnings`, `nix develop -c cargo fmt --check`. If a failure is outside this branch's changes, STOP with the evidence.

- [ ] **Step 2: Bench re-run (episode spend)** — `nix develop -c bench/bin/bench run interview-build`. Auth-failure rule as always (token_revoked in seconds → BLOCKED, never record). This scenario's 3/10 cap was the render-gap; the question is whether the episode discovers `tonk home`/`tonk view add` (grep episode.jsonl for `tonk home`, `tonk view add`, `tonk concept add`) and whether the build now lands on the space home (the `home` checkpoint screenshot + judge outcome).

- [ ] **Step 3: Record** — add the 2026-07-13 post-authoring-verbs row to `bench/README.md` (keep prior rows), noting verb-discovery evidence either way.

- [ ] **Step 4: Commit**

```bash
jj commit bench/README.md -m "docs(bench): interview-build baseline after authoring verbs and auto-surface"
```

---

## Self-review notes

- Spec coverage: authoring verbs (T2–T4), anchored concepts for name-addressability (T2/T3), auto-surface + explicit override exactly as decided (T4: `view add` surfaces only an unset home; `home` always re-points), the verified recipe verbatim with stable identifiers (T2 `build_home_recipe`), live-URL print (T4), enumerating errors for type/cardinality/grammar (T2/T3), the sync-semantics gap fix (T1), bench re-baseline (T6). The reactor per-item-view fallback from the findings is treated as orthogonal (data-presence is the gate) — flagged for STOP only if data itself is missing.
- Type consistency: `concept_add(site, &str, &[String], Option<&str>)`, `view_add(site, &str, Option<&str>, &str)`, `home(site, &[String])`, builders per Task 2's Interfaces — names match across tasks.
- New test file registered in Cargo.toml (autotests = false).
