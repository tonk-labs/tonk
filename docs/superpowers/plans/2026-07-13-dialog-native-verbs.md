# Dialog-Native Verbs Implementation Plan (CLI rename)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename the PR2 data verbs to dialog-native terminology — `add`+`set` merge into a unified `assert`, `rm` becomes `retract`, `list` becomes `query`, `describe` folds into `schema <concept>` — closing the supersede-form entity backdoor and locking cardinality-many behavior with tests along the way.

**Architecture:** Pure front-end rename over the existing thin-verbs-over-eval design. `data.rs` builders and `data_ops.rs` handlers are renamed; `assert` gains an entity-existence pre-check (a pure concept query) before the supersede form; `schema` gains an optional concept positional that emits a filtered notation subset. No change to the eval pipeline, the analyzer, or the notation grammar.

**Tech Stack:** Rust (`rust/tonk-cli`), clap 4 (derive + a runtime-built `clap::Command` for the schema-derived flags), the existing `tonk_evaluator::evaluate` pipeline.

**Spec:** `docs/superpowers/specs/2026-07-13-dialog-native-verbs-design.md` (read it before starting any task — it is the contract).

## Global Constraints

- VCS is jj (colocated). Commit with `jj commit <paths> -m "…"` — never `git add`/`git commit`, never touch bookmarks (the controller moves `feat/agent-build`). Conventional Commits, scope `cli` (or `docs`/`bench` where noted). No emojis anywhere.
- Commit-message footer on every commit:
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_01L8KJZ3gegT5ocgVztaGWwV`
- Test style: `#[dialog_common::test]`, names `it_does_x`, grouped in `mod when_…` blocks; shared fixtures in `tests/common.rs` (the crate sets `autotests = false` with explicit `[[test]]` entries — `data_verbs.rs` is already registered).
- Lint gate: `nix develop -c cargo clippy -p tonk-cli --all-targets -- -D warnings` per task; the full workspace gate (`--all-features`, all crates) runs in the final task. `rust/tonk-cli` has `#![warn(missing_docs)]` under `-D warnings` — every new `pub` item needs a doc comment.
- Copy rule (from the spec): user-facing `retract` copy says "retract", never "remove" or "delete", and notes a retraction is itself an assertion invalidating an old claim.
- No aliases or deprecation shims for the old verb names — replace outright.
- Every task must leave the whole crate compiling and its tests green (`cargo test` builds the binary too, so lib renames and their `bin/tonk.rs` call sites move in the same task).

---

### Task 1: Rename the notation builders in `data.rs`

**Files:**
- Modify: `rust/tonk-cli/src/data.rs`
- Modify: `rust/tonk-cli/src/data_ops.rs` (import + 3 call sites only — mechanical, keeps the crate green)

**Interfaces:**
- Consumes: nothing new.
- Produces: `pub fn build_assert(descriptor: &ConceptDescriptor, concept: &str, fields: &[(String, String)]) -> Result<String, DataError>` (was `build_add`); `pub fn build_supersede(descriptor: &ConceptDescriptor, concept: &str, entity: &str, fields: &[(String, String)]) -> Result<String, DataError>` (was `build_set`); `pub fn build_retract(concept: &str, entity: &str, field: Option<&str>) -> String` (was `build_rm`). Bodies unchanged. Task 2 calls these names.

- [ ] **Step 1: Rename the three builders and their doc comments**

In `rust/tonk-cli/src/data.rs`:
- `build_add` → `build_assert`. Doc comment becomes: `/// Build a `<concept>!: { … }` assertion document from (field, raw)` / `/// pairs, resolving each field's type through `descriptor` — the` / `/// mint form of `tonk assert` (no entity).`
- `build_set` → `build_supersede`. Doc comment becomes: `/// Build a `<concept>!: { this: <entity>, … }` assertion document —` / `/// superseding claims against an existing entity (the entity form` / `/// of `tonk assert`).`
- `build_rm` → `build_retract`. Doc comment becomes: `/// Build a retraction document: a single field (`field: _`) or the` / `/// whole entity (`..: _`) when `field` is `None`. A retraction is` / `/// itself an assertion — a claim invalidating an old one — not a` / `/// deletion.`
- In the `#[cfg(test)]` module, rename `it_builds_an_rm_field_retraction` → `it_builds_a_field_retraction` and `it_builds_an_rm_whole_entity_retraction` → `it_builds_a_whole_entity_retraction`, updating the `build_rm(` calls to `build_retract(`.

- [ ] **Step 2: Update the three call sites in `data_ops.rs`**

Change the import at the top of `rust/tonk-cli/src/data_ops.rs`:

```rust
use crate::data::{build_assert, build_retract, build_supersede};
```

and the calls: `build_add(` → `build_assert(` (in `add`), `build_set(` → `build_supersede(` (in `set`), `build_rm(` → `build_retract(` (in `rm`). Nothing else in this file changes yet.

- [ ] **Step 3: Verify green**

Run: `nix develop -c cargo test -p tonk-cli` then `nix develop -c cargo clippy -p tonk-cli --all-targets -- -D warnings`
Expected: all tests pass (rename is behavior-neutral), clippy clean.

- [ ] **Step 4: Commit**

```bash
jj commit rust/tonk-cli/src/data.rs rust/tonk-cli/src/data_ops.rs -m "refactor(cli): rename notation builders to assert/supersede/retract"
```

---

### Task 2: Unified `assert`, `retract`, `query` — handlers, errors, and the CLI surface

This is the core task: `data_ops` gains `assert_op` (merging `add`+`set` with the entity-existence check and the missing-required hint), `rm` → `retract`, `list` → `query`; `bin/tonk.rs` swaps the `Add`/`Set`/`Rm`/`List` variants for `Assert`/`Retract`/`Query`. `describe` and `Schema` are untouched here (Task 4).

**Files:**
- Modify: `rust/tonk-cli/src/data_ops.rs`
- Modify: `rust/tonk-cli/src/bin/tonk.rs`
- Test: `rust/tonk-cli/tests/data_verbs.rs`

**Interfaces:**
- Consumes: Task 1's `build_assert`/`build_supersede`/`build_retract`; existing `flags::parse_field_flags(descriptor, concept, argv, all_required) -> Result<Vec<(String,String)>, clap::Error>`; existing `require_concept(site, concept) -> Result<schema::ConceptInfo, DataOpError>`; `eval::run_against_site`; `EvaluateResponse.matches_after: Vec<QueryMatchBlock>` where each block has `label: String` and `results: Vec<_>`.
- Produces: `pub async fn assert_op(site: &TonkSite, concept: &str, entity: Option<&str>, argv: &[String]) -> Result<String, DataOpError>`; `pub async fn retract(site: &TonkSite, concept: &str, entity: &str, field: Option<&str>) -> Result<String, DataOpError>`; `pub async fn query(site: &TonkSite, concept: &str, json: bool) -> Result<String, DataOpError>`; new `DataOpError::NoInstance { concept: String, entity: String }` and `DataOpError::MissingRequired(clap::Error)` variants. Tasks 3–4 call these names.

- [ ] **Step 1: Write the failing tests**

In `rust/tonk-cli/tests/data_verbs.rs`, rename the call sites and add the new coverage. The full new shape of the affected modules (leave `when_describing_a_concept` and `when_the_concept_is_unknown` alone for now — Task 4 rewrites them):

Rename `mod when_reading_instances`'s two `list` calls to `tonk_cli::data_ops::query(...)` (same arguments). Rename `mod when_adding_an_instance` → `mod when_asserting_a_new_instance` and change its two calls from `data_ops::add(&test.site, "task", &argv)` to `data_ops::assert_op(&test.site, "task", None, &argv)`. Rename `mod when_setting_and_removing` → `mod when_superseding_and_retracting` and change: `data_ops::set(&test.site, "task", "t1", &[…])` → `data_ops::assert_op(&test.site, "task", Some("t1"), &[…])` (likewise `t1b`), `data_ops::rm(...)` → `data_ops::retract(...)` (three call sites). The `select_claims` helper moves from inside `when_setting_and_removing` to file scope (just below the `use` lines) unchanged — Task 3's module needs it too.

Then add the two new behaviors to `mod when_superseding_and_retracting`:

```rust
    #[dialog_common::test]
    async fn it_rejects_superseding_a_nonexistent_entity() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        // No instance seeded: a typo'd (or missing) entity must not
        // silently mint a partial orphan — the validation-backdoor
        // test from the spec.
        let err = tonk_cli::data_ops::assert_op(
            &test.site,
            "task",
            Some("no-such-task"),
            &["--title".into(), "x".into()],
        )
        .await
        .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("no task instance at 'no-such-task'"),
            "supersede against a nonexistent entity must fail loudly:\n{msg}"
        );
        assert!(
            msg.contains("tonk query task"),
            "the error should point at `tonk query`:\n{msg}"
        );
        // And nothing landed: the branch has no task rows at all.
        let out = tonk_cli::data_ops::query(&test.site, "task", false).await?;
        assert!(
            !out.contains("\"x\""),
            "no partial orphan may be minted:\n{out}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_hints_the_supersede_form_on_missing_required_fields() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        // Mint form with a required field missing — the agent who
        // meant to supersede and forgot the entity lands here, so
        // the error must point at the right fix.
        let err = tonk_cli::data_ops::assert_op(
            &test.site,
            "task",
            None,
            &["--title".into(), "only-title".into()],
        )
        .await
        .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("pass the entity before the flags"),
            "missing-required error should hint the supersede form:\n{msg}"
        );
        Ok(())
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop -c cargo test -p tonk-cli --test data_verbs`
Expected: compile error — `assert_op`, `retract`, `query` don't exist yet. That is the failure mode for a rename task.

- [ ] **Step 3: Implement `data_ops` changes**

In `rust/tonk-cli/src/data_ops.rs`:

(a) New error variants on `DataOpError` (place after `NoConcept`):

```rust
    /// The supersede form of `assert` named an entity that doesn't
    /// currently match the concept (every `with:` field bound).
    /// Closes the validation backdoor: without this, a typo'd
    /// entity would silently mint a partial orphan instance,
    /// bypassing the mint form's required-field check.
    #[error("no {concept} instance at '{entity}'; run `tonk query {concept}` to see what exists")]
    NoInstance {
        /// Concept the entity was checked against.
        concept: String,
        /// The entity reference that didn't resolve.
        entity: String,
    },
    /// The mint form of `assert` was missing one or more required
    /// fields. Rendered like [`DataOpError::Flags`], plus a hint
    /// for the agent who intended the supersede form and forgot
    /// the entity — clap's own message points at the wrong fix.
    #[error("{}\nto update an existing instance, pass the entity before the flags: tonk assert <concept> <entity> --<field> <value>", strip_clap_error_header(.0))]
    MissingRequired(clap::Error),
```

(b) `NoFields`'s message and doc comment adopt the vocabulary:

```rust
    /// The supersede form of `assert` was called with no `--field
    /// value` pairs at all — asserting against an existing entity
    /// with nothing to change would commit nothing.
    #[error("assert with an entity needs at least one --field to change")]
    NoFields,
```

(c) `exit_code` mapping: add `DataOpError::NoInstance { .. }` to the `NoConcept | Io` arm (`ExitCode::IoError`), and `DataOpError::MissingRequired(_)` to the `Data | Flags | NoFields` arm (`ExitCode::AnalyzeError`).

(d) The existence check (private helper, place near `query_doc`):

```rust
/// True iff `entity` currently matches `concept` — every `with:`
/// field bound, the same completeness [`get`] requires. A partial
/// instance (a field retracted) does not count; repairing one is
/// `tonk eval` territory.
async fn instance_exists(
    site: &TonkSite,
    descriptor: &dialog_query::ConceptDescriptor,
    concept: &str,
    entity: &str,
) -> Result<bool, DataOpError> {
    let doc = query_doc(descriptor, concept, Some(entity));
    let outcome = eval::run_against_site(site, Source::Inline(doc), Options::default()).await?;
    Ok(outcome
        .response
        .matches_after
        .iter()
        .find(|block| block.label == concept)
        .is_some_and(|block| !block.results.is_empty()))
}
```

(e) Replace `add` and `set` with the unified entry point (delete both old fns):

```rust
/// Assert claims against `concept` — dialog's one write operation.
/// With `entity: None`, mints a new instance: every non-optional
/// field is required (`all_required=true`), so a partial mint fails
/// clap's required-argument check before anything is built. With
/// `entity: Some(_)`, asserts superseding claims on that entity:
/// every field is optional, at least one must be supplied, and the
/// entity must already match the concept ([`instance_exists`]) —
/// otherwise the call is rejected as [`DataOpError::NoInstance`]
/// instead of silently minting a partial orphan.
///
/// A `--help` anywhere in `argv` is not an error: it returns
/// `Ok(help_text)` so the caller prints it and exits successfully —
/// the mint form renders required markers, the entity form renders
/// everything optional. A missing required field on the mint form
/// maps to [`DataOpError::MissingRequired`], whose display hints
/// the supersede form; any other flag rejection is
/// [`DataOpError::Flags`].
pub async fn assert_op(
    site: &TonkSite,
    concept: &str,
    entity: Option<&str>,
    argv: &[String],
) -> Result<String, DataOpError> {
    let info = require_concept(site, concept).await?;
    let all_required = entity.is_none();
    let pairs = match flags::parse_field_flags(&info.descriptor, concept, argv, all_required) {
        Ok(pairs) => pairs,
        Err(e) if e.kind() == clap::error::ErrorKind::DisplayHelp => return Ok(e.to_string()),
        Err(e)
            if all_required
                && e.kind() == clap::error::ErrorKind::MissingRequiredArgument =>
        {
            return Err(DataOpError::MissingRequired(e));
        }
        Err(e) => return Err(DataOpError::Flags(e)),
    };
    match entity {
        None => {
            let doc = build_assert(&info.descriptor, concept, &pairs)?;
            let outcome =
                eval::run_against_site(site, Source::Inline(doc), Options::default()).await?;
            Ok(format!("asserted {concept}\n{}", outcome.stdout))
        }
        Some(entity) => {
            if pairs.is_empty() {
                return Err(DataOpError::NoFields);
            }
            if !instance_exists(site, &info.descriptor, concept, entity).await? {
                return Err(DataOpError::NoInstance {
                    concept: concept.to_string(),
                    entity: entity.to_string(),
                });
            }
            let doc = build_supersede(&info.descriptor, concept, entity, &pairs)?;
            let outcome =
                eval::run_against_site(site, Source::Inline(doc), Options::default()).await?;
            Ok(format!("asserted {entity}\n{}", outcome.stdout))
        }
    }
}
```

(f) Rename `list` → `query` (body unchanged; doc comment: `/// Query every instance of `concept`, with every field bound —` / `/// reads are queries in dialog. Rendered as notation by default,` / `/// or as JSON when `json` is `true`.`).

(g) Rename `rm` → `retract`; result copy becomes:

```rust
    Ok(match field {
        Some(f) => format!("retracted {f} from {entity}\n{}", outcome.stdout),
        None => format!("retracted {entity}\n{}", outcome.stdout),
    })
```

with doc comment: `/// Retract one field, or the whole instance, from `entity`. A` / `/// retraction is itself an assertion — a claim invalidating an old` / `/// one — not a deletion. With `field: Some(f)`, retracts `f`` / `/// (validated against `concept`'s descriptor first, enumerating the` / `/// valid fields on a miss); on a many-cardinality field this` / `/// retracts every value. With `field: None`, retracts the whole` / `/// instance ([`build_retract`]'s `..: _` form).`

(h) The module doc comment's verb list and the `DataOpError` doc's verb list update to `assert`/`retract`/`query`/`get` (drop `add`/`set`/`list`/`rm`).

- [ ] **Step 4: Update `bin/tonk.rs`**

(a) Replace the `Add`, `Set`, `Rm`, `List` variants of `enum Command` with:

```rust
    /// Query every instance of a concept, with every field bound —
    /// reads are queries in dialog. Read-only; nothing commits.
    /// Filter flags (e.g. `--where`) are the intended future
    /// direction; today the whole concept is returned.
    #[command(after_help = "Examples:\n  tonk query task\n  tonk query task --json")]
    Query {
        /// Name of the concept to query.
        #[arg(value_name = "CONCEPT")]
        concept: String,
        /// Emit `EvaluateResponse` as pretty JSON instead of notation.
        #[arg(long)]
        json: bool,
    },

    /// Assert claims — dialog's one write operation. With no entity,
    /// mints a new instance of the concept (every non-optional field
    /// required); with an entity, asserts superseding claims on it
    /// (only the named fields change, and the entity must already
    /// match the concept). The flags after `<CONCEPT>` are built at
    /// runtime from the concept's own schema — run `tonk assert
    /// <concept> --help` to see them.
    ///
    /// `--help` is deliberately NOT handled by clap here
    /// (`disable_help_flag`): with clap's automatic `-h`/`--help`
    /// left on, it would intercept a trailing `--help` before it
    /// ever reached `rest`, so `tonk assert task --help` would show
    /// this static text instead of `task`'s real flags. Disabling
    /// it routes any `--help`/`-h` after `<CONCEPT>` into `rest`,
    /// where `data_ops::assert_op` builds the concept's own dynamic
    /// `clap::Command` and renders its help instead.
    #[command(
        disable_help_flag = true,
        after_help = "Examples:\n  tonk assert task --title \"Write the plan\" --done false\n  tonk assert task <entity> --done true\n  tonk assert task --help"
    )]
    Assert {
        /// Name of the concept to assert against.
        #[arg(value_name = "CONCEPT")]
        concept: String,
        /// Optional entity (a leading non-flag token selects the
        /// supersede form) followed by schema-derived `--field
        /// value` flags, captured raw (including a bare `--help`)
        /// so the dynamic per-concept parser — not clap's static
        /// subcommand parser — decides how to handle them.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        rest: Vec<String>,
    },

    /// Retract a single field, or a whole instance, from a concept.
    /// A retraction is itself an assertion — a claim invalidating an
    /// old one — not a deletion. Omit `--field` to retract the whole
    /// instance; on a many-cardinality field, `--field` retracts
    /// every value (value-level retraction is not yet surfaced).
    #[command(
        after_help = "Examples:\n  tonk retract task alice --field done\n  tonk retract task alice"
    )]
    Retract {
        /// Name of the concept the instance belongs to.
        #[arg(value_name = "CONCEPT")]
        concept: String,
        /// Bookmark name or `did:key:…` entity URI of the instance.
        #[arg(value_name = "ENTITY")]
        entity: String,
        /// Retract just this field instead of the whole instance.
        #[arg(long)]
        field: Option<String>,
    },
```

(b) `descriptor()` swaps the four old arms for:

```rust
        Command::Query { .. } => ("query", None),
        Command::Assert { .. } => ("assert", None),
        Command::Retract { .. } => ("retract", None),
```

(c) The `main` dispatch swaps `Add`/`Set`/`Rm`/`List` for:

```rust
        Command::Query { concept, json } => query_op(concept, json).await,
        Command::Assert { concept, rest } => assert_cmd(concept, rest).await,
        Command::Retract {
            concept,
            entity,
            field,
        } => retract_op(concept, entity, field).await,
```

(d) Replace the `add_op`/`set_op`/`rm_op`/`list_op` handler fns with (the site-open/print scaffolding is identical to the removed ones):

```rust
/// Query every instance of `concept` as rendered by
/// [`data_ops::query`].
async fn query_op(concept: String, json: bool) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return print_error(format!("could not determine current directory: {e}")),
    };
    let site = match site::TonkSite::discover_and_open(&cwd).await {
        Ok(s) => s,
        Err(err) => return print_error(err.to_string()),
    };

    match data_ops::query(&site, &concept, json).await {
        Ok(text) => {
            let mut stdout = std::io::stdout().lock();
            if let Err(e) = stdout.write_all(text.as_bytes()) {
                return print_error(format!("failed to write stdout: {e}"));
            }
            ExitCode::Success
        }
        Err(err) => {
            eprintln!("error: {err}");
            err.exit_code()
        }
    }
}

/// Split `rest` into the optional entity and the flag argv, then
/// assert via [`data_ops::assert_op`]. A leading non-flag token is
/// always the entity (the supersede form) — an entity reference
/// never starts with `-`, and flag values always follow their
/// flag, so the first token is either a flag or the entity. Same
/// dynamic-flag / `--help` handling as the old `add`/`set`.
async fn assert_cmd(concept: String, rest: Vec<String>) -> ExitCode {
    let (entity, argv) = match rest.split_first() {
        Some((first, tail)) if !first.starts_with('-') => {
            (Some(first.clone()), tail.to_vec())
        }
        _ => (None, rest),
    };
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return print_error(format!("could not determine current directory: {e}")),
    };
    let site = match site::TonkSite::discover_and_open(&cwd).await {
        Ok(s) => s,
        Err(err) => return print_error(err.to_string()),
    };

    match data_ops::assert_op(&site, &concept, entity.as_deref(), &argv).await {
        Ok(text) => {
            let mut stdout = std::io::stdout().lock();
            if let Err(e) = stdout.write_all(text.as_bytes()) {
                return print_error(format!("failed to write stdout: {e}"));
            }
            ExitCode::Success
        }
        Err(err) => {
            eprintln!("error: {err}");
            err.exit_code()
        }
    }
}

/// Retract a single field, or a whole instance, as rendered by
/// [`data_ops::retract`].
async fn retract_op(concept: String, entity: String, field: Option<String>) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return print_error(format!("could not determine current directory: {e}")),
    };
    let site = match site::TonkSite::discover_and_open(&cwd).await {
        Ok(s) => s,
        Err(err) => return print_error(err.to_string()),
    };

    match data_ops::retract(&site, &concept, &entity, field.as_deref()).await {
        Ok(text) => {
            let mut stdout = std::io::stdout().lock();
            if let Err(e) = stdout.write_all(text.as_bytes()) {
                return print_error(format!("failed to write stdout: {e}"));
            }
            ExitCode::Success
        }
        Err(err) => {
            eprintln!("error: {err}");
            err.exit_code()
        }
    }
}
```

(e) The module doc comment at the top of `bin/tonk.rs` and the `Cli` `after_help` keep their shape; no old verb names appear in either (verified by grep), so no change needed there.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `nix develop -c cargo test -p tonk-cli --test data_verbs`
Expected: all PASS, including the two new tests. Then the full crate: `nix develop -c cargo test -p tonk-cli` — the other integration suites (sync, share, …) don't touch these verbs and must stay green.

- [ ] **Step 6: Clippy**

Run: `nix develop -c cargo clippy -p tonk-cli --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
jj commit rust/tonk-cli/src/data_ops.rs rust/tonk-cli/src/bin/tonk.rs rust/tonk-cli/tests/data_verbs.rs -m "feat(cli): unified tonk assert plus retract and query, dialog-native verbs"
```

---

### Task 3: Lock cardinality-many behavior with tests

The spec's intended behavior: asserting on a many-cardinality field **appends** a value (mint or supersede); `retract --field` retracts **all** values. The analyzer's actual behavior through these builders is unverified — these tests lock it. **If the append test fails (the second assert supersedes instead of appending), STOP: do not paper over it. Report it to the controller as its own finding, per the spec.**

**Files:**
- Modify: `rust/tonk-cli/tests/common.rs` (new fixture decls)
- Modify: `rust/tonk-cli/src/data_ops/flags.rs` (many-cardinality help marker)
- Test: `rust/tonk-cli/tests/data_verbs.rs` (new module)

**Interfaces:**
- Consumes: Task 2's `assert_op`/`retract`/`query`; the file-scope `select_claims` helper Task 2 moved to the top of `data_verbs.rs`; `dialog_query::Cardinality` (variants `One`, `Many`); `ConceptFieldDescriptor::cardinality()`.
- Produces: `pub const NOTE_ATTRIBUTE_DECL: &str` and `pub const NOTE_CONCEPT_DECL: &str` in `tests/common.rs`; help strings for many-cardinality flags carry the suffix `" (cardinality many: each assert appends a value)"`.

- [ ] **Step 1: Add the fixtures to `tests/common.rs`**

Append after `CONCEPT_DECL`:

```rust
/// Attributes for the cardinality-many lock tests: a one-cardinality
/// `body` and a many-cardinality `tag`.
pub const NOTE_ATTRIBUTE_DECL: &str = r#"
attribute!: &note-body
  description: "note body"
  the:         xyz.tonk.note/body
  as:          text
  cardinality: one

attribute!: &note-tag
  description: "a tag on a note"
  the:         xyz.tonk.note/tag
  as:          text
  cardinality: many
"#;

/// A `note` concept referencing the attributes above — one required
/// one-cardinality field plus one required many-cardinality field.
pub const NOTE_CONCEPT_DECL: &str = r#"
concept!: &note
  description: "a tagged note"
  with:
    body: note-body
    tag:  note-tag
"#;
```

- [ ] **Step 2: Write the failing tests**

New module in `rust/tonk-cli/tests/data_verbs.rs` (import `NOTE_ATTRIBUTE_DECL, NOTE_CONCEPT_DECL` in the file's `use crate::common::{…}` line):

```rust
// Dialog semantics on a many-cardinality attribute: an assert
// APPENDS a value; it does not supersede. These tests lock that
// behavior through the typed-flag surface — if the analyzer turns
// out not to append, the failure here is a finding to surface, not
// to paper over (see the spec's cardinality-many section).
mod when_asserting_many_cardinality_fields {
    use super::*;

    #[dialog_common::test]
    async fn it_appends_a_value_instead_of_superseding() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(NOTE_ATTRIBUTE_DECL).await?;
        test.eval_inline(NOTE_CONCEPT_DECL).await?;
        // Anchor the mint through eval so the entity is addressable
        // by name across calls (the flag surface can't set anchors).
        test.eval_inline("note!: &n1\n  body: \"hello\"\n  tag: \"a\"\n")
            .await?;
        tonk_cli::data_ops::assert_op(
            &test.site,
            "note",
            Some("n1"),
            &["--tag".into(), "b".into()],
        )
        .await?;
        let tag_claims = select_claims(&test, "xyz.tonk.note/tag").await?;
        assert_eq!(
            tag_claims.len(),
            2,
            "asserting a second value on a many-cardinality field must append, got: {tag_claims:?}"
        );
        let body_claims = select_claims(&test, "xyz.tonk.note/body").await?;
        assert_eq!(
            body_claims.len(),
            1,
            "the untouched one-cardinality field must keep exactly one claim: {body_claims:?}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_retracts_every_value_of_a_many_field() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(NOTE_ATTRIBUTE_DECL).await?;
        test.eval_inline(NOTE_CONCEPT_DECL).await?;
        test.eval_inline("note!: &n2\n  body: \"hello\"\n  tag: \"a\"\n  tag: \"b\"\n")
            .await?;
        tonk_cli::data_ops::retract(&test.site, "note", "n2", Some("tag")).await?;
        let tag_claims = select_claims(&test, "xyz.tonk.note/tag").await?;
        assert!(
            tag_claims.is_empty(),
            "retract --field on a many field must retract every value: {tag_claims:?}"
        );
        let body_claims = select_claims(&test, "xyz.tonk.note/body").await?;
        assert_eq!(
            body_claims.len(),
            1,
            "the sibling field must survive: {body_claims:?}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_marks_many_fields_in_the_dynamic_help() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(NOTE_ATTRIBUTE_DECL).await?;
        test.eval_inline(NOTE_CONCEPT_DECL).await?;
        let help = tonk_cli::data_ops::assert_op(
            &test.site,
            "note",
            None,
            &["--help".into()],
        )
        .await?;
        assert!(
            help.contains("appends a value"),
            "many-cardinality fields should be marked in --help:\n{help}"
        );
        Ok(())
    }
}
```

Note on the `n2` seed: if the notation parser rejects a repeated `tag:` key in one block, seed the second value with a follow-up eval (`test.eval_inline("note!:\n  this: n2\n  tag: \"b\"\n")`) instead — the point of the fixture is two live tag claims, not the seeding syntax. Record which form worked in the task report.

- [ ] **Step 3: Run the tests to verify the help test fails**

Run: `nix develop -c cargo test -p tonk-cli --test data_verbs when_asserting_many_cardinality_fields`
Expected: `it_marks_many_fields_in_the_dynamic_help` FAILS (no marker yet). The two claim tests document unverified analyzer behavior — whatever they do on this run is the finding: if `it_appends_a_value_instead_of_superseding` fails, STOP and report (see the task preamble).

- [ ] **Step 4: Add the help marker to `flags.rs`**

In `rust/tonk-cli/src/data_ops/flags.rs`, change the import to `use dialog_query::{Cardinality, ConceptDescriptor};` and build the help string with the suffix:

```rust
    for (field, fd) in descriptor.with().iter() {
        let ty = fd
            .content_type()
            .map(|t| type_to_notation(&t))
            .unwrap_or_else(|| "value".into());
        let mut help = fd.description().to_string();
        if matches!(fd.cardinality(), Cardinality::Many) {
            help.push_str(" (cardinality many: each assert appends a value)");
        }
        cmd = cmd.arg(
            clap::Arg::new(field.to_string())
                .long(field.to_string())
                .value_name(ty.to_uppercase())
                .help(help)
                .required(all_required && !fd.is_optional()),
        );
    }
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `nix develop -c cargo test -p tonk-cli --test data_verbs`
Expected: all PASS (including the two claim-lock tests — if not, see the STOP rule).

- [ ] **Step 6: Clippy, then commit**

Run: `nix develop -c cargo clippy -p tonk-cli --all-targets -- -D warnings`

```bash
jj commit rust/tonk-cli/tests/common.rs rust/tonk-cli/tests/data_verbs.rs rust/tonk-cli/src/data_ops/flags.rs -m "test(cli): lock cardinality-many assert/retract semantics, mark many fields in help"
```

---

### Task 4: `schema <concept>` notation subset; drop `describe`

**Files:**
- Modify: `rust/tonk-cli/src/schema.rs` (new `render_one`)
- Modify: `rust/tonk-cli/src/data_ops.rs` (new `schema_subset`, delete `describe`)
- Modify: `rust/tonk-cli/src/bin/tonk.rs` (Schema gains optional positional; Describe variant removed)
- Test: `rust/tonk-cli/tests/data_verbs.rs`

**Interfaces:**
- Consumes: existing `enumerate_attributes`/`enumerate_concepts`/`render_attribute`/`render_concept` (all private to `schema.rs` — `render_one` lives beside them); `require_concept`; `ConceptDescriptor::with()` yielding field descriptors with `.the() -> impl Display`.
- Produces: `pub async fn schema::render_one(site: &TonkSite, name: &str) -> anyhow::Result<Option<String>>`; `pub async fn data_ops::schema_subset(site: &TonkSite, concept: &str) -> Result<String, DataOpError>`. `data_ops::describe` is deleted.

- [ ] **Step 1: Write the failing tests**

In `rust/tonk-cli/tests/data_verbs.rs`, replace `mod when_describing_a_concept` with (import `VIEW_DECL` in the `use crate::common::{…}` line):

```rust
mod when_rendering_a_concept_schema_subset {
    use super::*;
    use crate::common::VIEW_DECL;

    #[dialog_common::test]
    async fn it_emits_only_the_named_concepts_notation() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        // A second concept on the branch that must NOT appear.
        test.eval_inline(VIEW_DECL).await?;
        let out = tonk_cli::data_ops::schema_subset(&test.site, "task").await?;
        assert!(
            out.contains("concept!: &task"),
            "subset should carry the concept block:\n{out}"
        );
        assert!(
            out.contains("attribute!: &task-title"),
            "subset should carry the referenced attribute decls:\n{out}"
        );
        assert!(
            !out.contains("&view") && !out.contains("html-body"),
            "subset must not leak other concepts:\n{out}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_resubmits_cleanly_on_a_fresh_site() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        let out = tonk_cli::data_ops::schema_subset(&test.site, "task").await?;
        // Same format as bare `tonk schema`: the subset is a valid
        // notation document a fresh branch accepts wholesale.
        let fresh = TestSite::new().await?;
        fresh.eval_inline(&out).await?;
        let described = tonk_cli::data_ops::schema_subset(&fresh.site, "task").await?;
        assert!(
            described.contains("concept!: &task"),
            "re-submitted subset should reconstruct the concept:\n{described}"
        );
        Ok(())
    }
}
```

and change `mod when_the_concept_is_unknown`'s call from `data_ops::describe(&test.site, "widget")` to `data_ops::schema_subset(&test.site, "widget")` (assertion unchanged — the enumerating error comes from `require_concept` either way).

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop -c cargo test -p tonk-cli --test data_verbs when_rendering`
Expected: compile error — `schema_subset` doesn't exist.

- [ ] **Step 3: Implement**

(a) In `rust/tonk-cli/src/schema.rs`, below `render`:

```rust
/// Render one named concept's schema subset — the `attribute!:`
/// declarations it references followed by its `concept!:` block —
/// in the same re-submittable notation as [`render`]. Returns
/// `Ok(None)` when no user concept has that name.
pub async fn render_one(site: &TonkSite, name: &str) -> Result<Option<String>> {
    let attrs = enumerate_attributes(site).await?;
    let concepts = enumerate_concepts(site).await?;
    let Some(concept) = concepts.iter().find(|c| c.name == name) else {
        return Ok(None);
    };
    let uri_to_name: HashMap<String, String> = attrs
        .iter()
        .filter_map(|a| a.name.as_ref().map(|n| (a.the.clone(), n.clone())))
        .collect();
    let referenced: std::collections::HashSet<String> = concept
        .descriptor
        .with()
        .iter()
        .map(|(_, ad)| ad.the().to_string())
        .collect();
    let mut out = String::new();
    for attr in attrs.iter().filter(|a| referenced.contains(&a.the)) {
        render_attribute(&mut out, attr);
    }
    render_concept(&mut out, concept, &uri_to_name);
    Ok(Some(out))
}
```

(b) In `rust/tonk-cli/src/data_ops.rs`, delete `describe` and add:

```rust
/// Render one concept's schema subset — same notation as bare
/// `tonk schema`, filtered — or the enumerating [`DataOpError::NoConcept`].
/// The human field/type table this replaces lives in
/// `tonk assert <concept> --help`, where the flags are.
pub async fn schema_subset(site: &TonkSite, concept: &str) -> Result<String, DataOpError> {
    require_concept(site, concept).await?;
    match schema::render_one(site, concept).await {
        Ok(Some(text)) => Ok(text),
        Ok(None) => Err(DataOpError::Io(format!(
            "concept '{concept}' vanished between lookup and render"
        ))),
        Err(e) => Err(DataOpError::Io(e.to_string())),
    }
}
```

(If `type_to_notation` was only imported into `data_ops.rs` for `describe`, drop it from the `use crate::schema::…` line — clippy will flag it.)

(c) In `rust/tonk-cli/src/bin/tonk.rs`: delete the `Describe` variant, its `descriptor()` arm, its dispatch arm, and the `describe_op` fn. Change `Schema` to:

```rust
    /// Print the site's schema as a re-submittable notation
    /// document — every named attribute and concept, or just one
    /// concept's subset when `<CONCEPT>` is given. The human
    /// field/type view lives in `tonk assert <concept> --help`.
    #[command(
        after_help = "Examples:\n  tonk schema\n  tonk schema task\n  tonk schema > schema.notation"
    )]
    Schema {
        /// Optional concept name — emit only that concept's
        /// `concept!:` block plus the `attribute!:` declarations
        /// it references.
        #[arg(value_name = "CONCEPT")]
        concept: Option<String>,
    },
```

dispatch: `Command::Schema { concept } => print_schema(concept).await,` — `descriptor()` arm becomes `Command::Schema { .. } => ("schema", None),`. And `print_schema` becomes:

```rust
async fn print_schema(concept: Option<String>) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return print_error(format!("could not determine current directory: {e}")),
    };
    let site = match site::TonkSite::discover_and_open(&cwd).await {
        Ok(s) => s,
        Err(err) => return print_error(err.to_string()),
    };
    let rendered = match &concept {
        Some(name) => match data_ops::schema_subset(&site, name).await {
            Ok(text) => text,
            Err(err) => {
                eprintln!("error: {err}");
                return err.exit_code();
            }
        },
        None => match schema::render(&site).await {
            Ok(text) => text,
            Err(err) => return print_error(err.to_string()),
        },
    };
    let mut stdout = std::io::stdout().lock();
    if let Err(e) = stdout.write_all(rendered.as_bytes()) {
        return print_error(format!("failed to write stdout: {e}"));
    }
    ExitCode::Success
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `nix develop -c cargo test -p tonk-cli`
Expected: all PASS (full crate — schema/share suites must stay green).

- [ ] **Step 5: Clippy, then commit**

Run: `nix develop -c cargo clippy -p tonk-cli --all-targets -- -D warnings`

```bash
jj commit rust/tonk-cli/src/schema.rs rust/tonk-cli/src/data_ops.rs rust/tonk-cli/src/bin/tonk.rs rust/tonk-cli/tests/data_verbs.rs -m "feat(cli): tonk schema <concept> notation subset replaces describe"
```

---

### Task 5: Documentation — README, the tonk agent reference, superseded notes

**Files:**
- Modify: `rust/tonk-cli/README.md` (the data-verbs block, lines ~31–38)
- Rewrite: `.claude/commands/tonk.md`
- Modify: `docs/superpowers/plans/2026-07-08-data-verbs.md` (superseded note at top)
- Modify: `docs/superpowers/specs/2026-07-08-agent-ergonomic-cli-design.md` (superseded note at top)

**Interfaces:**
- Consumes: the shipped surface from Tasks 2–4 — verify each documented command against `target/debug/tonk … --help` output before writing it down (build with `nix develop -c cargo build -p tonk-cli` first).
- Produces: docs only.

- [ ] **Step 1: Rewrite the README data-verbs block**

In `rust/tonk-cli/README.md`, replace:

```
# Argument-based data verbs — a constrained front-end over `eval`.
tonk describe habit                       # fields, types, cardinality (schema-aware --help source)
tonk add habit --name "Run" --target "5k" # add an instance (typed flags from the branch schema)
tonk list habit                           # all instances (add --json for machine output)
tonk get habit <entity>                   # one instance
tonk set habit <entity> --target "10k"    # overwrite fields on an existing instance
tonk rm habit <entity> [--field target]   # retract one field, or the whole instance
```

with:

```
# Argument-based data verbs — a constrained front-end over `eval`.
# Dialog vocabulary: you assert claims and retract them. A retraction
# is itself an assertion invalidating an old claim, not a delete.
tonk schema habit                             # one concept's schema, as re-submittable notation
tonk assert habit --help                      # the concept's real flags (fields, types, required)
tonk assert habit --name "Run" --target "5k"  # mint a new instance (typed flags from the branch schema)
tonk assert habit <entity> --target "10k"     # assert superseding claims on an existing instance
tonk query habit                              # every instance (add --json for machine output)
tonk get habit <entity>                       # one instance
tonk retract habit <entity> --field target    # retract one field (a many field loses every value)
tonk retract habit <entity>                   # retract the whole instance
```

If surrounding README prose names the old verbs, update it to match (grep the file for `add`/`set `/`rm `/`list`/`describe` and fix any data-verb mentions — leave `remote add`/`set-upstream` alone).

- [ ] **Step 2: Rewrite `.claude/commands/tonk.md`**

The current file documents a wholly fictional CLI (`tonk login`, `space create`, `concept define`, `--json create/query/show/update/delete`). Replace the entire file with a reference for the real surface. Before writing, verify each command line against the built binary's `--help`. Content:

````markdown
# tonk CLI — Agent Reference

tonk is a headless CLI for reading and writing data and views in a local
`.tonk/` site (a dialog repository). Data lives as claims: you **assert**
claims and **retract** them — a retraction is itself an assertion that
invalidates an old claim, not a deletion.

Run commands from a directory at or below the one containing `.tonk/`.

## Orientation

```bash
tonk guide            # one-screen index of the agent reference
tonk schema           # every concept + attribute on the branch, as notation
tonk schema <concept> # one concept's subset, same format
tonk concepts         # name<TAB>description, one row per user concept
tonk status           # synced | ahead | behind | diverged | no-upstream
```

## Data verbs (schema-derived typed flags)

The flags for `assert` are built at runtime from the concept's own schema —
`tonk assert <concept> --help` shows the real fields, types, and which are
required. Errors enumerate the valid options.

```bash
tonk assert <concept> --<field> <value> …            # mint a new instance (all non-optional fields required)
tonk assert <concept> <entity> --<field> <value> …   # supersede fields on an existing instance
tonk query <concept> [--json]                        # every instance, every field bound
tonk get <concept> <entity> [--json]                 # one instance
tonk retract <concept> <entity> --field <f>          # retract one field (a many-cardinality field loses every value)
tonk retract <concept> <entity>                      # retract the whole instance
```

Notes:
- `<entity>` is a bookmark name or `did:key:…` URI. The supersede form
  requires the entity to already match the concept; a typo fails with
  "no <concept> instance at …" instead of minting a partial orphan.
- Asserting on a many-cardinality field appends a value.
- Exit codes: 0 success, 1 parse, 2 analyze, 3 commit, 4 I/O.

## Escape hatch: eval (asserted-notation documents)

Anything the verbs don't cover — defining concepts/attributes, rules,
views, multi-statement documents — goes through `tonk eval`:

```bash
tonk eval -c '<notation>'     # inline document (-c is required for inline!)
tonk eval ./doc.notation      # from a file
tonk eval - < doc.notation    # from stdin
tonk eval -c '…' --dry-run    # preview without committing
```

`tonk guide notation` documents the grammar; `tonk guide views` covers
`view!:` authoring. A bare positional is a FILE PATH, never inline text.

## Sync and sharing

```bash
tonk push | tonk pull                       # sync main with its upstream
tonk remote add <name> <url>                # register an access-service remote
tonk remote set-upstream <name>             # track <name>/main
tonk invite [--remote <name>]               # mint a paste-able invite URL (pushes first)
tonk join '<invite-url>'                    # claim an invite into a fresh .tonk/
tonk share concept <name>                   # launcher URL onto a live concept view
tonk share display <subject> --view <name>  # launcher URL onto a <tonk-display> render
tonk render <route>                         # headless HTML render (e.g. alice@person!card)
```

## Setup

```bash
tonk init            # create .tonk/ in the current directory
tonk identity        # show the local profile DID
tonk migrate         # convert a .carry/ site to .tonk/
```
````

- [ ] **Step 3: Add superseded notes to the two historical docs**

At the very top of `docs/superpowers/plans/2026-07-08-data-verbs.md` (below the H1) and `docs/superpowers/specs/2026-07-08-agent-ergonomic-cli-design.md` (below the Date/Status lines), insert:

```markdown
> **Superseded (verb surface):** the data verbs were renamed to
> dialog-native `assert`/`retract`/`query`, `add`+`set` merged into
> `assert`, and `describe` folded into `schema <concept>` — see
> `docs/superpowers/specs/2026-07-13-dialog-native-verbs-design.md`.
> Command names below are the old surface.
```

- [ ] **Step 4: Commit**

```bash
jj commit rust/tonk-cli/README.md .claude/commands/tonk.md docs/superpowers/plans/2026-07-08-data-verbs.md docs/superpowers/specs/2026-07-08-agent-ergonomic-cli-design.md -m "docs(cli): dialog-native verb docs; rewrite the tonk agent reference to the real surface"
```

---

### Task 6: Full gates + bench re-baseline

**Files:**
- Modify: `bench/README.md` (baseline table)

**Interfaces:**
- Consumes: everything landed in Tasks 1–5; the bench harness (`bench/bin/bench run <scenario>`, codex/gpt-5.5 episode runner).
- Produces: green workspace gates; post-rename bench numbers recorded next to the 2026-07-08 baselines.

- [ ] **Step 1: Full workspace gates**

Run, from the repo root:

```bash
nix develop -c cargo test -p tonk-cli
nix develop -c cargo clippy --workspace --all-targets --all-features -- -D warnings
nix develop -c cargo fmt --check
```

Expected: all green. (`--all-features` compiles integration tests other per-crate runs skip — this is the gate `nix flake check` enforces.)

- [ ] **Step 2: Bench re-baseline (episode spend — codex/gpt-5.5)**

Codex OAuth must be valid; if a run dies in seconds with `token_revoked`, ask the user to run `codex login` and retry — never record an auth-failed run.

```bash
nix develop -c bench/bin/bench run targeted-edit
nix develop -c bench/bin/bench run interview-build
```

Expected: both complete with a real `judge.outcome` in `bench/runs/<ts>-…/scores.json`. Read each run's `report.md`: the question is whether the episodes now use `assert`/`retract`/`query` (check `episode.jsonl` for the commands issued) and whether the outcome holds or improves against the 2026-07-08 baselines (targeted-edit 9/10, interview-build 3/10).

- [ ] **Step 3: Record the numbers**

In `bench/README.md`, add a dated row-set under the baselines section: scenario, outcome, top friction, and a note that this is the post-rename (dialog-native verbs) measurement. Keep the 07-08 rows for the before/after story.

- [ ] **Step 4: Commit**

```bash
jj commit bench/README.md -m "docs(bench): post-rename baselines for targeted-edit and interview-build"
```

---

## Self-review notes

- Spec coverage: verb mapping (T2, T4), entity backdoor + NoInstance + hint (T2), cardinality-many lock + help marker (T3), schema fold with notation-subset consistency (T4), copy rules (T2 g/h, T4, T5), doc blast radius incl. skill file + superseded notes (T5), bench re-baseline (T6), PR3 noun-first note (spec-only, no task needed).
- `cargo test -p tonk-cli` builds the binary, so lib renames and bin call sites always move in the same task (T2, T4) — every commit is green.
- Type consistency: `assert_op(site, concept, Option<&str>, &[String])`, `retract(site, concept, &str, Option<&str>)`, `query(site, concept, bool)`, `schema_subset(site, concept)`, builders `build_assert`/`build_supersede`/`build_retract` — names match across tasks.
