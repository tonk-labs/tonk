# Data Verbs Implementation Plan (CLI PR2)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add schema-aware, argument-based data verbs to tonk-cli — `tonk add/set/get/list/rm` over existing concepts, plus `tonk describe` — as a thin front-end over the existing eval pipeline, so agents manipulate data with fixed `--flag` names read from `--help` instead of authoring the notation DSL.

**Architecture:** Each verb builds an asserted-notation document in-process and runs it through the existing `tonk_cli::eval::run_against_site` (analyze→commit) — no new write path. The mutating verbs (`add`/`set`) get schema-aware typed flags by capturing raw args in the clap derive layer, reading the target concept's attributes off the branch, building a `clap::Command` with one `Arg` per attribute at runtime, and parsing the captured args against it (so `tonk add habit --help` renders the real flags and clap's own errors enumerate valid flags). Reads (`get`/`list`) build query docs and reuse `output::render` for human/`--json` output. `eval` stays the escape hatch.

**Tech Stack:** Rust, clap 4 (builder API alongside the existing derive), `tonk_evaluator` via `eval::run_against_site`, `dialog_query::{ConceptDescriptor, Type, Cardinality}`.

**Spec:** `docs/superpowers/specs/2026-07-08-agent-ergonomic-cli-design.md` (§Verb surface → Data; §Sequencing phase 2).

## Global Constraints

- VCS is jj (colocated). Commit path-scoped with `jj commit <paths> -m "…"` — never `git add`/`git commit`, never touch bookmarks (the controller moves `feat/agent-build`). Conventional Commits, scope `cli`. No emojis.
- Repo test style: `#[dialog_common::test]`, test names `it_does_x`, grouped in `mod when_…` blocks. Integration tests use `crate::common::TestSite` (`rust/tonk-cli/tests/common.rs`: `TestSite::new()`, `.eval_inline(doc)`, and the `ATTRIBUTE_DECL` / `CONCEPT_DECL` seed constants). No `mod.rs`; use `foo.rs` + `foo/`.
- Lint gate: `nix develop -c cargo clippy -p tonk-cli --all-targets -- -D warnings` must pass (note the `--` separator).
- Additive only: do NOT change `eval`, `invite`, or existing verbs. `eval` remains the escape hatch.
- The build uses the dialog-db locked rev `1c9bc9c` (`Cargo.lock`). The `ConceptDescriptor` API: `descriptor.with().iter()` yields `(&str field_name, &ConceptFieldDescriptor)` sorted by name; per field `.the()` (attribute selector; `.the().to_string()` = `domain/name` URI), `.content_type() -> Option<Type>`, `.cardinality() -> Cardinality`, `.description() -> &str`, `.is_optional() -> bool`. `descriptor.description() -> Option<&str>`.
- `Type` (= `dialog_artifacts::ValueDataType`) → notation string: `String`→`Text`, `Entity`→`Entity`, `UnsignedInt`→`UnsignedInteger`, `SignedInt`→`SignedInteger`, `Float`→`Float`, `Boolean`→`Boolean`, `Symbol`→`Symbol`, `Bytes`→`Bytes`, `Record`→`Record`. Derive the string via `serde_json::to_string(&ty)` trimmed of quotes (the existing private `type_to_notation` at `schema.rs:377`).
- Value rendering into notation, by field `Type`: `Text`/`String` → double-quoted+escaped (reuse `quote_string`, `output.rs`/`schema.rs:384`) — ALWAYS quote (a bare value that parses as a symbol/bool is misread); numerics (`UnsignedInt`/`SignedInt`/`Float`) → bare literal; `Boolean` → bare `true`/`false`; `Entity`/`Symbol` → bare (a name or a URI, emitted verbatim, no quotes). Untyped (`None`) → quote as text.
- Entity addressing (`get`/`set`/`rm` `<entity>`): pass the user string straight into notation `this:` — the notation layer resolves a bare bookmark name via the name table and a `did:key:…`/`id:…` URI directly. No Name-concept round-trip needed.

---

### Task 1: Expose the concept schema-read API

**Files:**
- Modify: `rust/tonk-cli/src/schema.rs` (make `ConceptInfo` + its fields `pub`, add a `pub async fn find_concept`, make `type_to_notation` `pub(crate)`)
- Test: `rust/tonk-cli/tests/schema_read.rs` (new; add `mod common;` and the `autotests`/`[[test]]` wiring only if the crate's Cargo.toml requires explicit test entries — check `rust/tonk-cli/Cargo.toml` first, mirror how `tests/render.rs` is registered)

**Interfaces:**
- Produces: `pub struct ConceptInfo { pub name: String, pub description: Option<String>, pub descriptor: dialog_query::ConceptDescriptor }`; `pub async fn find_concept(site: &TonkSite, name: &str) -> anyhow::Result<Option<ConceptInfo>>` (returns the named user concept or `None`); `pub(crate) fn type_to_notation(ty: &dialog_query::Type) -> String`.
- Consumes: existing private `enumerate_concepts` (`schema.rs:239`).

- [ ] **Step 1: Write the failing test**

Create `rust/tonk-cli/tests/schema_read.rs`:

```rust
mod common;

use anyhow::Result;
use crate::common::{CONCEPT_DECL, TestSite};

mod when_reading_a_concepts_schema {
    use super::*;

    #[dialog_common::test]
    async fn it_returns_fields_types_and_cardinality_for_a_named_concept() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(CONCEPT_DECL).await?; // seeds the `task` concept
        let info = tonk_cli::schema::find_concept(&test.site, "task")
            .await?
            .expect("task concept should be found");
        assert_eq!(info.name, "task");
        let fields: Vec<&str> = info.descriptor.with().iter().map(|(f, _)| f).collect();
        assert!(fields.contains(&"title"), "task should have a title field, got {fields:?}");
        Ok(())
    }

    #[dialog_common::test]
    async fn it_returns_none_for_an_unknown_concept() -> Result<()> {
        let test = TestSite::new().await?;
        assert!(tonk_cli::schema::find_concept(&test.site, "nope").await?.is_none());
        Ok(())
    }
}
```

(First open `rust/tonk-cli/tests/common.rs` and confirm the exact name of the concept-seed constant — the plan assumes `CONCEPT_DECL` seeding a `task` concept with a `title` field. If the constant or field differs, use the real one and adjust the asserted field name.)

- [ ] **Step 2: Run the test, verify it fails**

Run: `nix develop -c cargo test -p tonk-cli --test schema_read`
Expected: FAILS to compile — `find_concept` / `pub ConceptInfo` don't exist yet.

- [ ] **Step 3: Expose the API in schema.rs**

Make `ConceptInfo` and its fields `pub` (it's currently a private struct near `enumerate_concepts`). Change `fn type_to_notation` to `pub(crate) fn type_to_notation`. Add:

```rust
/// Find a single user-defined concept by its bookmark name,
/// returning its full descriptor (fields, types, cardinalities,
/// descriptions) or `None`. Built-in concepts are excluded, matching
/// `enumerate_concepts`.
pub async fn find_concept(site: &TonkSite, name: &str) -> Result<Option<ConceptInfo>> {
    Ok(enumerate_concepts(site)
        .await?
        .into_iter()
        .find(|c| c.name == name))
}
```

- [ ] **Step 4: Run the tests, verify they pass**

Run: `nix develop -c cargo test -p tonk-cli --test schema_read`
Expected: both PASS.

- [ ] **Step 5: Clippy + commit**

Run: `nix develop -c cargo clippy -p tonk-cli --all-targets -- -D warnings`

```bash
jj commit rust/tonk-cli/src/schema.rs rust/tonk-cli/tests/schema_read.rs rust/tonk-cli/Cargo.toml -m "feat(cli): expose find_concept schema-read API for data verbs"
```
(Include Cargo.toml only if you added a `[[test]]` entry.)

---

### Task 2: The `data` module — value rendering + notation builders

**Files:**
- Create: `rust/tonk-cli/src/data.rs`
- Modify: `rust/tonk-cli/src/lib.rs` (add `pub mod data;`)
- Test: unit tests inline in `data.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `dialog_query::{ConceptDescriptor, Type}`, `crate::schema::type_to_notation`, `crate::output`'s `quote_string` (if not `pub(crate)`, lift a local copy — a 6-line string escaper; do NOT make output internals public just for this).
- Produces (pure, no IO):
  - `pub fn render_value(ty: Option<Type>, raw: &str) -> Result<String, DataError>` — raw user string → notation value per the rendering rules.
  - `pub fn build_add(descriptor: &ConceptDescriptor, concept: &str, fields: &[(String, String)]) -> Result<String, DataError>` → `<concept>!: { … }` doc.
  - `pub fn build_set(descriptor: &ConceptDescriptor, concept: &str, entity: &str, fields: &[(String, String)]) -> Result<String, DataError>` → `<concept>!: { this: <entity>, … }`.
  - `pub fn build_rm(concept: &str, entity: &str, field: Option<&str>) -> String` → `field: _` or `..: _` retract.
  - `pub enum DataError { UnknownField{concept,field,valid:Vec<String>}, BadValue{field,ty:String,raw:String} }` with `Display` that ENUMERATES (e.g. `unknown field 'x' on task; valid fields: title, done`).
- `fields` is a `Vec<(field_name, raw_value)>` the caller extracted from clap matches (Task 4). `render_value` looks up each field's `Type` from the descriptor; `build_*` return `DataError::UnknownField` for a field not in the descriptor.

- [ ] **Step 1: Write the failing tests**

In `rust/tonk-cli/src/data.rs`, add pure unit tests. Construct a descriptor by parsing a known concept doc through the analyzer is heavy for a unit test — instead test `render_value` directly (it takes `Option<Type>`), and test the `build_*` string shape with a small hand-built descriptor helper OR by asserting on the doc a full round-trip produces (defer full-descriptor tests to Task 4's integration test). Minimum unit tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use dialog_query::Type;

    #[test]
    fn it_quotes_text_values() {
        assert_eq!(render_value(Some(Type::String), "hi there").unwrap(), "\"hi there\"");
    }
    #[test]
    fn it_renders_numeric_values_bare() {
        assert_eq!(render_value(Some(Type::UnsignedInt), "42").unwrap(), "42");
    }
    #[test]
    fn it_renders_boolean_bare() {
        assert_eq!(render_value(Some(Type::Boolean), "true").unwrap(), "true");
    }
    #[test]
    fn it_renders_entity_values_bare() {
        assert_eq!(render_value(Some(Type::Entity), "run").unwrap(), "run");
        assert_eq!(render_value(Some(Type::Entity), "did:key:z6Mk").unwrap(), "did:key:z6Mk");
    }
    #[test]
    fn it_rejects_a_non_numeric_for_a_numeric_field() {
        assert!(render_value(Some(Type::UnsignedInt), "notanumber").is_err());
    }
    #[test]
    fn it_builds_an_rm_field_retraction() {
        assert_eq!(
            build_rm("task", "t1", Some("done")),
            "task!:\n  this: t1\n  done: _\n"
        );
    }
    #[test]
    fn it_builds_an_rm_whole_entity_retraction() {
        assert_eq!(build_rm("task", "t1", None), "task!:\n  this: t1\n  ..: _\n");
    }
}
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `nix develop -c cargo test -p tonk-cli --lib data`
Expected: FAILS to compile (module/functions absent).

- [ ] **Step 3: Implement `data.rs`**

```rust
//! Notation builders for the argument-based data verbs. Each verb
//! collects (field, raw-value) pairs from clap, renders them into an
//! asserted-notation document per the field's schema type, and hands
//! the document to `eval::run_against_site` — so the verbs are a
//! constrained front-end over the same analyze→commit pipeline as
//! `tonk eval`, not a second write path.

use dialog_query::{ConceptDescriptor, Type};

use crate::schema::type_to_notation;

#[derive(Debug, thiserror::Error)]
pub enum DataError {
    #[error("unknown field '{field}' on {concept}; valid fields: {}", valid.join(", "))]
    UnknownField { concept: String, field: String, valid: Vec<String> },
    #[error("value '{raw}' is not a valid {ty} for field '{field}'")]
    BadValue { field: String, ty: String, raw: String },
}

/// Render one raw CLI value into its notation form given the field's
/// declared type. Text is always quoted (a bare value that parses as a
/// symbol/bool would be misread); numerics/bools are bare literals
/// (validated); entities/symbols are emitted verbatim (a bare name or a
/// URI). An untyped field (`None`) is quoted as text.
pub fn render_value(ty: Option<Type>, raw: &str) -> Result<String, DataError> {
    let bad = |ty: &str| DataError::BadValue { field: String::new(), ty: ty.into(), raw: raw.into() };
    match ty {
        Some(Type::UnsignedInt) => { raw.parse::<u64>().map_err(|_| bad("UnsignedInteger"))?; Ok(raw.to_string()) }
        Some(Type::SignedInt)   => { raw.parse::<i64>().map_err(|_| bad("SignedInteger"))?; Ok(raw.to_string()) }
        Some(Type::Float)       => { raw.parse::<f64>().map_err(|_| bad("Float"))?; Ok(raw.to_string()) }
        Some(Type::Boolean)     => { raw.parse::<bool>().map_err(|_| bad("Boolean"))?; Ok(raw.to_string()) }
        Some(Type::Entity) | Some(Type::Symbol) => Ok(raw.to_string()),
        _ => Ok(quote_string(raw)), // String/Bytes/Record/None → quoted text
    }
}

/// Double-quote and escape a string for notation (mirrors the emitter
/// in `output.rs`/`schema.rs`; kept local to avoid widening their API).
fn quote_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn valid_fields(descriptor: &ConceptDescriptor) -> Vec<String> {
    descriptor.with().iter().map(|(f, _)| f.to_string()).collect()
}

fn render_pairs(
    descriptor: &ConceptDescriptor,
    concept: &str,
    fields: &[(String, String)],
) -> Result<Vec<String>, DataError> {
    let mut lines = Vec::with_capacity(fields.len());
    for (field, raw) in fields {
        let Some((_, fd)) = descriptor.with().iter().find(|(f, _)| f == field) else {
            return Err(DataError::UnknownField {
                concept: concept.to_string(),
                field: field.clone(),
                valid: valid_fields(descriptor),
            });
        };
        let value = render_value(fd.content_type(), raw).map_err(|e| match e {
            DataError::BadValue { ty, raw, .. } => DataError::BadValue { field: field.clone(), ty, raw },
            other => other,
        })?;
        lines.push(format!("  {field}: {value}"));
    }
    Ok(lines)
}

pub fn build_add(descriptor: &ConceptDescriptor, concept: &str, fields: &[(String, String)]) -> Result<String, DataError> {
    let body = render_pairs(descriptor, concept, fields)?.join("\n");
    Ok(format!("{concept}!:\n{body}\n"))
}

pub fn build_set(descriptor: &ConceptDescriptor, concept: &str, entity: &str, fields: &[(String, String)]) -> Result<String, DataError> {
    let body = render_pairs(descriptor, concept, fields)?.join("\n");
    Ok(format!("{concept}!:\n  this: {entity}\n{body}\n"))
}

pub fn build_rm(concept: &str, entity: &str, field: Option<&str>) -> String {
    match field {
        Some(f) => format!("{concept}!:\n  this: {entity}\n  {f}: _\n"),
        None => format!("{concept}!:\n  this: {entity}\n  ..: _\n"),
    }
}
```

Add `pub mod data;` to `rust/tonk-cli/src/lib.rs`. Note `type_to_notation` is imported but only used once errors reference type names — if clippy flags it unused, drop the import (the `Type` match handles rendering). Keep the import only if a task below needs it; otherwise remove to satisfy `-D warnings`.

- [ ] **Step 4: Run tests, verify pass; clippy**

Run: `nix develop -c cargo test -p tonk-cli --lib data` then `nix develop -c cargo clippy -p tonk-cli --all-targets -- -D warnings`
Expected: all pass, no warnings.

- [ ] **Step 5: Commit**

```bash
jj commit rust/tonk-cli/src/data.rs rust/tonk-cli/src/lib.rs -m "feat(cli): notation builders and value rendering for data verbs"
```

---

### Task 3: `tonk describe <concept>`

**Files:**
- Modify: `rust/tonk-cli/src/bin/tonk.rs` (new `Describe` variant + handler + `descriptor()` entry)
- Test: `rust/tonk-cli/tests/data_verbs.rs` (new; `mod common;`)

**Interfaces:**
- Consumes: `schema::find_concept` (Task 1), `schema::type_to_notation` (Task 1).
- Produces: `tonk describe <concept>` prints each field as `<name>  <TYPE>  <cardinality>  — <description>`, plus the concept description. Exit `Success`; unknown concept → an enumerating error (list known concept names via `schema::list_concepts`) and `ExitCode::IoError`.

- [ ] **Step 1: Write the failing test**

Create `rust/tonk-cli/tests/data_verbs.rs`:

```rust
mod common;

use anyhow::Result;
use crate::common::{CONCEPT_DECL, TestSite};

// The verbs are exercised through the library handlers, not the binary,
// to avoid spawning a subprocess. Each handler returns its rendered
// stdout + an ExitCode. (Task 3 introduces `tonk_cli::data_ops`.)
mod when_describing_a_concept {
    use super::*;

    #[dialog_common::test]
    async fn it_lists_fields_with_types() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(CONCEPT_DECL).await?;
        let out = tonk_cli::data_ops::describe(&test.site, "task").await?;
        assert!(out.contains("title"), "describe should list the title field:\n{out}");
        assert!(out.contains("Text"), "describe should show the field type:\n{out}");
        Ok(())
    }
}
```

Design note this locks in: verb bodies live in a testable library module `tonk_cli::data_ops` (returning `Result<String, EvalError>` or a small `DataOpError`), and the thin `bin/tonk.rs` handlers call them and map to `ExitCode`. This keeps the binary a parser→call shim (matching `eval`/`invite`) and makes every verb integration-testable without a subprocess.

- [ ] **Step 2: Run test, verify it fails**

Run: `nix develop -c cargo test -p tonk-cli --test data_verbs`
Expected: FAILS to compile (`data_ops` absent).

- [ ] **Step 3: Implement `data_ops::describe`**

Create `rust/tonk-cli/src/data_ops.rs` (add `pub mod data_ops;` to `lib.rs`):

```rust
//! Library handlers for the data verbs — the testable core the thin
//! `bin/tonk.rs` handlers call. Each returns rendered stdout; the
//! binary maps errors to exit codes.

use anyhow::Result;

use crate::schema::{self, type_to_notation};
use crate::site::TonkSite;

/// Render a concept's schema as a human-readable field list.
pub async fn describe(site: &TonkSite, concept: &str) -> Result<String> {
    let info = schema::find_concept(site, concept).await?.ok_or_else(|| {
        anyhow::anyhow!("no concept named '{concept}'")
    })?;
    let mut out = String::new();
    if let Some(desc) = info.descriptor.description() {
        out.push_str(desc);
        out.push_str("\n\n");
    }
    for (field, fd) in info.descriptor.with().iter() {
        let ty = fd.content_type().map(|t| type_to_notation(&t)).unwrap_or_else(|| "any".into());
        let card = format!("{:?}", fd.cardinality()).to_lowercase();
        let req = if fd.is_optional() { "" } else { " (required)" };
        out.push_str(&format!("  --{field} <{ty}> [{card}]{req}  {}\n", fd.description()));
    }
    Ok(out)
}
```

Wire the binary: add `Describe { #[arg(value_name="CONCEPT")] concept: String }` to `enum Command` (`bin/tonk.rs`), a match arm `Command::Describe { concept } => describe_op(concept).await`, an async `describe_op` that calls `data_ops::describe`, prints the string, and (on the unknown-concept error) prints a message listing `schema::list_concepts` names to stderr and returns `ExitCode::IoError`. Add `Command::Describe { .. } => ("describe", None)` to the `descriptor()` telemetry map.

- [ ] **Step 4: Run test, verify pass; clippy**

Run: `nix develop -c cargo test -p tonk-cli --test data_verbs` then the clippy gate.
Expected: pass, clean.

- [ ] **Step 5: Commit**

```bash
jj commit rust/tonk-cli/src/data_ops.rs rust/tonk-cli/src/lib.rs rust/tonk-cli/src/bin/tonk.rs rust/tonk-cli/tests/data_verbs.rs -m "feat(cli): tonk describe <concept>"
```

---

### Task 4: `tonk add <concept>` with dynamic schema-aware flags

**Files:**
- Modify: `rust/tonk-cli/src/data_ops.rs` (add `add`), `rust/tonk-cli/src/bin/tonk.rs` (variant + dynamic-clap wiring), `rust/tonk-cli/tests/data_verbs.rs`
- Create: `rust/tonk-cli/src/data_ops/flags.rs` (the reusable dynamic-`Command` builder) — or keep it inline in `data_ops.rs` if small; prefer a `flags.rs` since `set` reuses it.

**Interfaces:**
- Consumes: `schema::find_concept`, `data::{build_add, DataError}`, `eval::{run_against_site, Source, Options}`.
- Produces:
  - `pub fn parse_field_flags(descriptor: &ConceptDescriptor, argv: &[String], all_required: bool) -> Result<Vec<(String,String)>, clap::Error>` (in `flags.rs`) — builds a `clap::Command` with one `Arg` per field and parses `argv`; returns the (field, value) pairs the user supplied. `--help` in `argv` surfaces as `Err` with `kind()==DisplayHelp` whose `to_string()` is the rendered help.
  - `pub async fn add(site, concept: &str, argv: &[String]) -> AddOutcome` where the handler resolves the descriptor, calls `parse_field_flags(.., all_required=true)`, `build_add`, and `run_against_site`, then returns the created entity URI (read from the committed response's matches for `concept`) plus a success line.
- Global flags: `--json`/`--dry-run` are NOT in the trailing capture. Put the trailing capture in a derive variant `Add { concept: String, #[arg(trailing_var_arg=true, allow_hyphen_values=true)] rest: Vec<String> }`. `tonk add habit --help` routes `--help` into `rest` (dynamic help); `tonk add --help` hits clap's static subcommand help.

- [ ] **Step 1: Write the failing test**

Add to `tests/data_verbs.rs`:

```rust
mod when_adding_an_instance {
    use super::*;

    #[dialog_common::test]
    async fn it_commits_an_instance_from_typed_flags() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(CONCEPT_DECL).await?;
        // task has a `title` (Text) field.
        let argv = vec!["--title".to_string(), "Write the plan".to_string()];
        tonk_cli::data_ops::add(&test.site, "task", &argv).await?;
        // Verify it landed: list should now show the title.
        let out = tonk_cli::data_ops::list(&test.site, "task", false).await?;
        assert!(out.contains("Write the plan"), "added instance should appear in list:\n{out}");
        Ok(())
    }

    #[dialog_common::test]
    async fn it_errors_enumerating_valid_flags_on_unknown_field() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(CONCEPT_DECL).await?;
        let argv = vec!["--nope".to_string(), "x".to_string()];
        let err = tonk_cli::data_ops::add(&test.site, "task", &argv).await.unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("title"), "error should enumerate valid flags:\n{msg}");
        Ok(())
    }
}
```

(This test references `data_ops::list` from Task 6 — write Task 6's `list` first if executing strictly TDD, or stub `list` in this task and flesh it out in Task 6. Recommended: implement a minimal `list` here since `add` needs a read-back to verify, and enrich it in Task 6. Note the dependency in the report.)

- [ ] **Step 2: Run test, verify it fails**

Run: `nix develop -c cargo test -p tonk-cli --test data_verbs when_adding`
Expected: FAILS to compile (`add`/`list` absent).

- [ ] **Step 3: Implement `flags.rs` + `add`**

`rust/tonk-cli/src/data_ops/flags.rs`:

```rust
//! Build a clap Command from a concept's schema so `tonk add/set
//! <concept>` gets real typed `--flags`, `--help`, and enumerating
//! errors — all driven by the branch schema, not hand-rolled.

use dialog_query::ConceptDescriptor;

use crate::schema::type_to_notation;

/// Parse schema-derived `--field value` flags out of `argv`. With
/// `all_required`, every field is a required arg (used by `add`);
/// otherwise all are optional (used by `set`). Returns the (field,
/// value) pairs actually supplied. A `--help` in `argv` returns
/// `Err(e)` with `e.kind() == clap::error::ErrorKind::DisplayHelp`.
pub fn parse_field_flags(
    descriptor: &ConceptDescriptor,
    concept: &str,
    argv: &[String],
    all_required: bool,
) -> Result<Vec<(String, String)>, clap::Error> {
    let mut cmd = clap::Command::new(format!("tonk … {concept}")).no_binary_name(true);
    if let Some(about) = descriptor.description() {
        cmd = cmd.about(about.to_string());
    }
    let field_names: Vec<String> = descriptor.with().iter().map(|(f, _)| f.to_string()).collect();
    for (field, fd) in descriptor.with().iter() {
        let ty = fd.content_type().map(|t| type_to_notation(&t)).unwrap_or_else(|| "value".into());
        cmd = cmd.arg(
            clap::Arg::new(field.to_string())
                .long(field.to_string())
                .value_name(ty.to_uppercase())
                .help(fd.description().to_string())
                .required(all_required && !fd.is_optional()),
        );
    }
    let matches = cmd.try_get_matches_from(argv)?;
    Ok(field_names
        .into_iter()
        .filter_map(|f| matches.get_one::<String>(&f).map(|v| (f, v.clone())))
        .collect())
}
```

In `data_ops.rs` add `pub mod flags;` and:

```rust
use crate::data::{build_add, build_set, build_rm};
use crate::eval::{self, Options, Source};

pub async fn add(site: &TonkSite, concept: &str, argv: &[String]) -> Result<String> {
    let info = schema::find_concept(site, concept).await?
        .ok_or_else(|| anyhow::anyhow!("no concept named '{concept}'"))?;
    let pairs = match flags::parse_field_flags(&info.descriptor, concept, argv, true) {
        Ok(p) => p,
        Err(e) => {
            // DisplayHelp is a successful help render, not an error.
            print!("{e}");
            if e.kind() == clap::error::ErrorKind::DisplayHelp { return Ok(String::new()); }
            anyhow::bail!("invalid flags for {concept}");
        }
    };
    let doc = build_add(&info.descriptor, concept, &pairs)?;
    eval::run_against_site(site, Source::Inline(doc), Options::default()).await?;
    Ok(format!("added {concept}\n"))
}
```

Wire the binary: `Add { concept: String, #[arg(trailing_var_arg = true, allow_hyphen_values = true)] rest: Vec<String> }`, match arm calls an `add_op(concept, rest)` that invokes `data_ops::add` and prints the outcome; add the `descriptor()` entry `Command::Add { .. } => ("add", None)`. Handle the `DisplayHelp` path so `--help` exits `Success`.

(The `build_set`/`build_rm` imports are used by Task 5; add them there if clippy flags unused here.)

- [ ] **Step 4: Run tests, verify pass; verify --help renders flags manually**

Run: `nix develop -c cargo test -p tonk-cli --test data_verbs`
Then a manual check of the dynamic help against a real scratch site:
```bash
T=$(mktemp -d); cd "$T"; /Users/jackdouglas/tonk/tonk/target/release/tonk init >/dev/null
# (build first: nix develop -c cargo build --release -p tonk-cli)
env TONK_NO_SYNC=1 /Users/jackdouglas/tonk/tonk/target/release/tonk eval /Users/jackdouglas/tonk/tonk/bench/scenarios/targeted-edit/seed.notation >/dev/null 2>&1
env TONK_NO_SYNC=1 /Users/jackdouglas/tonk/tonk/target/release/tonk add habit --help
cd - >/dev/null; rm -rf "$T"
```
Expected: the help lists `--name` and `--target` with their types. (Requires a release build with this task's code; run the cargo build first.)

- [ ] **Step 5: Clippy + commit**

```bash
jj commit rust/tonk-cli/src/data_ops.rs rust/tonk-cli/src/data_ops/flags.rs rust/tonk-cli/src/bin/tonk.rs rust/tonk-cli/tests/data_verbs.rs -m "feat(cli): tonk add <concept> with schema-aware typed flags"
```

---

### Task 5: `tonk set <entity>` and `tonk rm <entity>`

**Files:**
- Modify: `rust/tonk-cli/src/data_ops.rs`, `rust/tonk-cli/src/bin/tonk.rs`, `rust/tonk-cli/tests/data_verbs.rs`

**Interfaces:**
- Consumes: `flags::parse_field_flags(.., all_required=false)`, `data::{build_set, build_rm}`, `eval::run_against_site`, `schema::find_concept`.
- Produces:
  - `pub async fn set(site, concept: &str, entity: &str, argv: &[String]) -> Result<String>` — the entity is addressed by bare name or URI (passed to `this:`); flags optional (a subset overwrite).
  - `pub async fn rm(site, concept: &str, entity: &str, field: Option<&str>) -> Result<String>`.
- Binary: `Set { concept: String, entity: String, #[arg(trailing_var_arg=true, allow_hyphen_values=true)] rest: Vec<String> }` and `Rm { concept: String, entity: String, #[arg(long)] field: Option<String> }`.

Note the addressing decision (from the mechanics report): `set`/`rm` take BOTH `<concept>` and `<entity>` — the concept names which schema to validate flags against and which head to assert; the entity is the `this:`. (An alternative single-arg `set <entity>` would require reverse-resolving the concept from the entity — deferred; v1 takes both.)

- [ ] **Step 1: Write the failing tests**

```rust
mod when_setting_and_removing {
    use super::*;

    #[dialog_common::test]
    async fn it_overwrites_a_field_on_a_named_entity() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(CONCEPT_DECL).await?;
        // Seed a named task `t1` with an anchor so it is addressable.
        test.eval_inline("task!: &t1\n  title: \"old\"\n").await?;
        tonk_cli::data_ops::set(&test.site, "task", "t1", &["--title".into(), "new".into()]).await?;
        let out = tonk_cli::data_ops::get(&test.site, "task", "t1", false).await?;
        assert!(out.contains("new") && !out.contains("old"), "set should overwrite:\n{out}");
        Ok(())
    }

    #[dialog_common::test]
    async fn it_retracts_a_single_field() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(CONCEPT_DECL).await?;
        test.eval_inline("task!: &t2\n  title: \"x\"\n").await?;
        tonk_cli::data_ops::rm(&test.site, "task", "t2", Some("title")).await?;
        // After retracting its only declared field, the concept query no
        // longer matches it (concept queries require every field present).
        let out = tonk_cli::data_ops::list(&test.site, "task", false).await?;
        assert!(!out.contains("t2"), "retracted field should drop the row from the concept query:\n{out}");
        Ok(())
    }
}
```

(Uses `get`/`list` from Task 6; implement minimal versions here or ensure Task 6 lands first. Confirm the `task!: &t1 …` anchored-seed form and the `task` field name against the real `CONCEPT_DECL`.)

- [ ] **Step 2: Run, verify fail.** `nix develop -c cargo test -p tonk-cli --test data_verbs when_setting` → compile failure.

- [ ] **Step 3: Implement `set`/`rm`**

```rust
pub async fn set(site: &TonkSite, concept: &str, entity: &str, argv: &[String]) -> Result<String> {
    let info = schema::find_concept(site, concept).await?
        .ok_or_else(|| anyhow::anyhow!("no concept named '{concept}'"))?;
    let pairs = match flags::parse_field_flags(&info.descriptor, concept, argv, false) {
        Ok(p) => p,
        Err(e) => { print!("{e}");
            if e.kind() == clap::error::ErrorKind::DisplayHelp { return Ok(String::new()); }
            anyhow::bail!("invalid flags for {concept}"); }
    };
    if pairs.is_empty() { anyhow::bail!("set needs at least one --field to change"); }
    let doc = build_set(&info.descriptor, concept, entity, &pairs)?;
    eval::run_against_site(site, Source::Inline(doc), Options::default()).await?;
    Ok(format!("updated {entity}\n"))
}

pub async fn rm(site: &TonkSite, concept: &str, entity: &str, field: Option<&str>) -> Result<String> {
    // Validate the field belongs to the concept (enumerating error).
    if let Some(f) = field {
        let info = schema::find_concept(site, concept).await?
            .ok_or_else(|| anyhow::anyhow!("no concept named '{concept}'"))?;
        let valid: Vec<String> = info.descriptor.with().iter().map(|(n,_)| n.to_string()).collect();
        if !valid.contains(&f.to_string()) {
            anyhow::bail!("unknown field '{f}' on {concept}; valid fields: {}", valid.join(", "));
        }
    }
    let doc = build_rm(concept, entity, field);
    eval::run_against_site(site, Source::Inline(doc), Options::default()).await?;
    Ok(match field { Some(f) => format!("removed {f} from {entity}\n"), None => format!("removed {entity}\n") })
}
```

Wire the two binary variants + `descriptor()` entries (`"set"`, `"rm"`).

- [ ] **Step 4: Run tests, verify pass; clippy.**

- [ ] **Step 5: Commit**

```bash
jj commit rust/tonk-cli/src/data_ops.rs rust/tonk-cli/src/bin/tonk.rs rust/tonk-cli/tests/data_verbs.rs -m "feat(cli): tonk set and rm over existing instances"
```

---

### Task 6: `tonk get`/`tonk list` + `--json`

**Files:**
- Modify: `rust/tonk-cli/src/data_ops.rs`, `rust/tonk-cli/src/bin/tonk.rs`, `rust/tonk-cli/tests/data_verbs.rs`

**Interfaces:**
- Consumes: `schema::find_concept` (for the field list to bind), `eval::run_against_site` with `Options { format, quiet:false, dry_run:false }`, `output::Format`.
- Produces:
  - `pub async fn list(site, concept: &str, json: bool) -> Result<String>` — builds `<concept>:\n  this: ?e\n  <f>: ?<f>\n…` and renders via `run_against_site` (a no-`!` doc commits nothing).
  - `pub async fn get(site, concept: &str, entity: &str, json: bool) -> Result<String>` — same doc with `this: <entity>` as a constant.
- Binary: `List { concept: String, #[arg(long)] json: bool }`, `Get { concept: String, entity: String, #[arg(long)] json: bool }`.
- If Tasks 4/5 already added minimal `list`/`get`, this task enriches them (bind all fields, `--json` via `Format::Json`).

- [ ] **Step 1: Write the failing tests**

```rust
mod when_reading_instances {
    use super::*;

    #[dialog_common::test]
    async fn it_lists_all_instances_of_a_concept() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(CONCEPT_DECL).await?;
        test.eval_inline("task!:\n  title: \"alpha\"\ntask!:\n  title: \"beta\"\n").await?;
        let out = tonk_cli::data_ops::list(&test.site, "task", false).await?;
        assert!(out.contains("alpha") && out.contains("beta"), "list should show both:\n{out}");
        Ok(())
    }

    #[dialog_common::test]
    async fn it_emits_json_when_requested() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(CONCEPT_DECL).await?;
        test.eval_inline("task!:\n  title: \"gamma\"\n").await?;
        let out = tonk_cli::data_ops::list(&test.site, "task", true).await?;
        assert!(out.trim_start().starts_with('{') || out.trim_start().starts_with('['), "json output:\n{out}");
        Ok(())
    }
}
```

- [ ] **Step 2: Run, verify fail.**

- [ ] **Step 3: Implement `get`/`list`**

```rust
use crate::output::Format;

fn query_doc(descriptor: &dialog_query::ConceptDescriptor, concept: &str, entity: Option<&str>) -> String {
    let mut doc = format!("{concept}:\n");
    match entity {
        Some(e) => doc.push_str(&format!("  this: {e}\n")),
        None => doc.push_str("  this: ?e\n"),
    }
    for (field, _) in descriptor.with().iter() {
        doc.push_str(&format!("  {field}: ?{field}\n"));
    }
    doc
}

async fn run_read(site: &TonkSite, doc: String, json: bool) -> Result<String> {
    let options = Options { format: if json { Format::Json } else { Format::Notation }, quiet: false, dry_run: false };
    let outcome = eval::run_against_site(site, Source::Inline(doc), options).await?;
    Ok(outcome.stdout)
}

pub async fn list(site: &TonkSite, concept: &str, json: bool) -> Result<String> {
    let info = schema::find_concept(site, concept).await?
        .ok_or_else(|| anyhow::anyhow!("no concept named '{concept}'"))?;
    run_read(site, query_doc(&info.descriptor, concept, None), json).await
}

pub async fn get(site: &TonkSite, concept: &str, entity: &str, json: bool) -> Result<String> {
    let info = schema::find_concept(site, concept).await?
        .ok_or_else(|| anyhow::anyhow!("no concept named '{concept}'"))?;
    run_read(site, query_doc(&info.descriptor, concept, Some(entity)), json).await
}
```

Wire the `List`/`Get` binary variants + `descriptor()` entries. Confirm `output::Format` variant names (`Format::Json`, `Format::Notation`) against `output.rs`.

- [ ] **Step 4: Run tests, verify pass; clippy.**

- [ ] **Step 5: Commit**

```bash
jj commit rust/tonk-cli/src/data_ops.rs rust/tonk-cli/src/bin/tonk.rs rust/tonk-cli/tests/data_verbs.rs -m "feat(cli): tonk get and list with --json"
```

---

### Task 7: Enumerating-concept errors + guide/README + full verify

**Files:**
- Modify: `rust/tonk-cli/src/data_ops.rs` (unknown-concept errors enumerate known concepts), `rust/tonk-cli/src/bin/tonk.rs` (help examples), `rust/tonk-cli/README.md` (document the verbs), `rust/tonk-cli/tests/data_verbs.rs`

**Interfaces:**
- Consumes: `schema::list_concepts`.
- Produces: every "no concept named 'x'" path lists the known concept names; the README's Usage block shows the verbs.

- [ ] **Step 1: Write the failing test**

```rust
mod when_the_concept_is_unknown {
    use super::*;

    #[dialog_common::test]
    async fn it_enumerates_known_concepts() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(CONCEPT_DECL).await?; // seeds `task`
        let err = tonk_cli::data_ops::list(&test.site, "widget", false).await.unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("task"), "unknown-concept error should list known concepts:\n{msg}");
        Ok(())
    }
}
```

- [ ] **Step 2: Run, verify fail** (current error says only "no concept named 'widget'").

- [ ] **Step 3: Enrich the error**

Replace the shared `find_concept(...).ok_or_else(...)` sites with a helper:

```rust
async fn require_concept(site: &TonkSite, concept: &str) -> Result<schema::ConceptInfo> {
    match schema::find_concept(site, concept).await? {
        Some(info) => Ok(info),
        None => {
            let known: Vec<String> = schema::list_concepts(site).await?
                .into_iter().map(|c| c.name).collect();
            anyhow::bail!("no concept named '{concept}'; known concepts: {}", known.join(", "))
        }
    }
}
```

Use `require_concept` in `add`/`set`/`rm`/`get`/`list`/`describe`.

- [ ] **Step 4: Run test, verify pass.**

- [ ] **Step 5: Document + full suite + clippy**

Add a "Data verbs" block to `rust/tonk-cli/README.md`'s Usage section:
```
# Argument-based data verbs (a constrained front-end over `eval`).
tonk describe habit                 # fields, types, cardinality
tonk add habit --name "Run" --target "5k"
tonk list habit                     # all instances (--json for machine output)
tonk get habit <entity>
tonk set habit <entity> --target "10k"
tonk rm habit <entity> [--field target]
```

Run the full suite + clippy:
`nix develop -c cargo test -p tonk-cli` then `nix develop -c cargo clippy -p tonk-cli --all-targets -- -D warnings`
Expected: all green, no warnings.

- [ ] **Step 6: Commit**

```bash
jj commit rust/tonk-cli/src/data_ops.rs rust/tonk-cli/src/bin/tonk.rs rust/tonk-cli/README.md rust/tonk-cli/tests/data_verbs.rs -m "feat(cli): enumerating unknown-concept errors and data-verb docs"
```

---

## Bench delta (after the plan lands, controller-run)

Rebuild the release binary and re-run `targeted-edit` on the bench: the one-line rename that scored 9/10 but "read two guides" should now be a single `tonk set habit <entity> --name "Inbox Zero — daily"` (or the agent may still choose eval — the point is the verb exists and `describe`/`--help` surface it). Record the delta against the eval-only baseline in `bench/README.md`. Not a code task; the controller runs it.
