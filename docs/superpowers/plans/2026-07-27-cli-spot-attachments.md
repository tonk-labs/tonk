# CLI Spot Directory Attachments Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a shell tab or an agent bind a directory to a tonk spot once, so every later command run from that directory resolves to it without `--spot` or `TONK_SPOT`.

**Architecture:** `spots.json` gains a top-level `attachments` map from canonicalized absolute directory to spot name. A new resolution tier sits between `TONK_SPOT` and the registry's `current`: walk the cwd and its ancestors, first hit wins. The directory is only ever a key into the registry — it never locates site data and never creates anything.

**Tech Stack:** Rust, `clap` (derive), `serde`/`serde_json`, `thiserror`, `tempfile`, `#[dialog_common::test]`.

**Spec:** `docs/superpowers/specs/2026-07-27-cli-spot-attachments-design.md`

## Global Constraints

- All work is in `rust/tonk-cli/`. Branch: `feat/cli-sessions`. PRs on this repo target `staging`, not `main`.
- New tests use `#[dialog_common::test]` (matching `tests/spot.rs` and `tests/cli_spot.rs`), never bare `#[test]` or `#[tokio::test]`. Leave the existing `#[test]` attributes in `src/spot.rs` alone — converting them is unrelated churn. `tonk-cli` does not build for wasm, so no `wasm_bindgen_test_configure!` line.
- Test names read `it_does_x` and group under behaviour-named `mod` blocks. This is the existing style in both files.
- No new dependencies.
- Never mutate process-global state (cwd, env) from a test. `SpotStore::resolve` takes the cwd as a parameter for exactly this reason; integration tests set `Command::current_dir` on the spawned binary instead.
- Do not reference "Task N", "the plan", or "the spec" in code or comments. Comments explain why, in the voice of the surrounding file.
- Verify with, from the repo root inside `nix develop`:
  - unit: `cargo test -p tonk-cli --lib spot::`
  - ops: `cargo test -p tonk-cli --test spot`
  - CLI: `cargo test -p tonk-cli --test cli_spot`
  - lint gate before the final commit: `cargo clippy --workspace --all-targets --all-features` and `cargo fmt --all`

---

### Task 1: Attachment storage and ops

Adds the `attachments` field plus `attach` / `detach` / prune-on-`remove`. Resolution is untouched here — nothing reads the map yet, which keeps this task independently reviewable.

**Files:**
- Modify: `rust/tonk-cli/src/spot.rs`

**Interfaces:**
- Consumes: nothing (first task).
- Produces:
  - `Registry.attachments: BTreeMap<PathBuf, String>` (public field, `#[serde(default)]`)
  - `fn canonical(path: &Path) -> PathBuf` (private)
  - `fn attached(registry: &Registry, cwd: &Path) -> Option<(PathBuf, String)>` (private)
  - `pub struct AttachOutcome { pub directory: PathBuf, pub name: String, pub previous: Option<String> }`
  - `pub struct DetachOutcome { pub directory: PathBuf, pub name: String }`
  - `pub fn attach(store: &SpotStore, name: &str, directory: &Path) -> Result<AttachOutcome, SpotError>`
  - `pub fn detach(store: &SpotStore, directory: &Path) -> Result<DetachOutcome, SpotError>`
  - `SpotError::NotAttached { directory: PathBuf, ancestor: Option<(PathBuf, String)> }`
  - `RemoveOutcome.detached: Vec<PathBuf>`

- [ ] **Step 1: Write the failing tests**

Add a new `mod attaching` inside the existing `#[cfg(test)] mod tests` block in `src/spot.rs`, after `mod resolving`:

```rust
    mod attaching {
        use super::*;

        #[dialog_common::test]
        fn it_attaches_a_directory_and_reports_the_previous_binding() {
            let (_tmp, store) = store();
            store
                .save(&registry_with(&[("a", "/s/a"), ("b", "/s/b")], None))
                .expect("save");

            let first = attach(&store, "a", Path::new("/proj")).expect("attach");
            assert_eq!(first.name, "a");
            assert_eq!(first.previous, None);
            assert_eq!(first.directory, PathBuf::from("/proj"));

            let second = attach(&store, "b", Path::new("/proj")).expect("re-attach");
            assert_eq!(second.previous.as_deref(), Some("a"));
            assert_eq!(
                store
                    .load()
                    .expect("load")
                    .attachments
                    .get(Path::new("/proj")),
                Some(&"b".to_owned())
            );
        }

        #[dialog_common::test]
        fn it_refuses_to_attach_an_unknown_spot() {
            let (_tmp, store) = store();
            store
                .save(&registry_with(&[("a", "/s/a")], None))
                .expect("save");

            let err = attach(&store, "nope", Path::new("/proj")).expect_err("unknown");
            assert!(matches!(err, SpotError::Unknown { .. }), "{err}");
            assert!(
                store.load().expect("load").attachments.is_empty(),
                "a failed attach must not write"
            );
        }

        #[dialog_common::test]
        fn it_detaches_only_an_exact_match_and_names_the_ancestor() {
            let (_tmp, store) = store();
            store
                .save(&registry_with(&[("a", "/s/a")], None))
                .expect("save");
            attach(&store, "a", Path::new("/proj")).expect("attach");

            let err = detach(&store, Path::new("/proj/sub")).expect_err("not attached here");
            assert!(err.to_string().contains("/proj is attached to a"), "{err}");
            assert!(
                !store.load().expect("load").attachments.is_empty(),
                "a subdirectory detach must not unbind the parent"
            );

            let outcome = detach(&store, Path::new("/proj")).expect("detach");
            assert_eq!(outcome.name, "a");
            assert!(store.load().expect("load").attachments.is_empty());
        }

        #[dialog_common::test]
        fn it_prunes_attachments_when_the_spot_is_removed() {
            let (_tmp, store) = store();
            store
                .save(&registry_with(&[("a", "/s/a"), ("b", "/s/b")], None))
                .expect("save");
            attach(&store, "a", Path::new("/proj")).expect("attach a");
            attach(&store, "b", Path::new("/other")).expect("attach b");

            let outcome = remove(&store, "a", false).expect("remove");
            assert_eq!(outcome.detached, vec![PathBuf::from("/proj")]);

            let attachments = store.load().expect("load").attachments;
            assert_eq!(attachments.get(Path::new("/other")), Some(&"b".to_owned()));
            assert!(attachments.get(Path::new("/proj")).is_none());
        }
    }
```

Note on the fake paths: `canonical()` falls back to the path as given when `canonicalize()` fails, so `/proj` and `/proj/sub` behave consistently on both the write and the lookup side without needing real directories.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p tonk-cli --lib spot::tests::attaching`
Expected: FAIL to compile — `cannot find function 'attach' in this scope`.

- [ ] **Step 3: Add the registry field**

In `src/spot.rs`, add to `struct Registry` after the `current` field:

```rust
    /// Directories bound to a spot, keyed by canonicalized absolute
    /// path. Consulted between `TONK_SPOT` and `current`, so a
    /// session that works out of a directory holds its own spot
    /// without repeating itself on every invocation. A top-level map
    /// rather than a list inside each entry: a key cannot repeat, so
    /// "one directory, one spot" is structural rather than enforced.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attachments: BTreeMap<PathBuf, String>,
```

Also extend the module header's layout comment, changing the `spots.json` line to:

```rust
//!   spots.json      registry: name → site path, plus `current` and
//!                   directory attachments
```

- [ ] **Step 4: Add the path helpers**

Add below `validate_name`:

```rust
/// Canonicalize a path for use as an attachment key, falling back to
/// the path as given when the filesystem refuses (most often: the
/// directory has been deleted). A key that cannot be canonicalized
/// simply never matches a canonicalized cwd, which is the right
/// outcome for a directory that no longer exists — the attachment
/// tier is skipped and resolution falls through.
fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// The nearest attachment at or above `cwd`: start at the directory
/// itself and climb to the root, taking the first hit, so a nested
/// directory overrides its parent.
fn attached(registry: &Registry, cwd: &Path) -> Option<(PathBuf, String)> {
    if registry.attachments.is_empty() {
        return None;
    }
    let cwd = canonical(cwd);
    cwd.ancestors().find_map(|dir| {
        registry
            .attachments
            .get_key_value(dir)
            .map(|(path, name)| (path.clone(), name.clone()))
    })
}
```

- [ ] **Step 5: Add the `NotAttached` error**

Add to `enum SpotError`, after the `Unknown` variant:

```rust
    /// `spot detach` against a directory with no attachment of its
    /// own. Matching is exact on purpose — unbinding a whole project
    /// because someone typed `detach` three levels down inside it is
    /// not a recoverable surprise — so the ancestor that *is*
    /// attached goes in the message instead.
    #[error("no attachment at {directory}{}", detach_hint(.ancestor))]
    NotAttached {
        /// The directory that was asked about.
        directory: PathBuf,
        /// The nearest attached ancestor, for the error hint.
        ancestor: Option<(PathBuf, String)>,
    },
```

And beside `unknown_hint`:

```rust
/// Hint suffix for [`SpotError::NotAttached`]: name the ancestor
/// that is attached, so the fix is a `cd` away.
fn detach_hint(ancestor: &Option<(PathBuf, String)>) -> String {
    match ancestor {
        Some((directory, name)) => {
            format!("; {} is attached to {name}", directory.display())
        }
        None => String::new(),
    }
}
```

- [ ] **Step 6: Add the attach and detach ops**

Add after `select`:

```rust
/// Outcome of [`attach`].
#[derive(Debug, Clone)]
pub struct AttachOutcome {
    /// The canonicalized directory now bound.
    pub directory: PathBuf,
    /// The spot it resolves to.
    pub name: String,
    /// The spot it was bound to before, when it was already attached.
    pub previous: Option<String>,
}

/// Outcome of [`detach`].
#[derive(Debug, Clone)]
pub struct DetachOutcome {
    /// The directory that is no longer bound.
    pub directory: PathBuf,
    /// The spot it used to resolve to.
    pub name: String,
}

/// Bind `directory` to `name`, leaving the global `current` alone.
/// Re-attaching an already-bound directory overwrites and reports
/// what it replaced: unlike `spot new`, nothing is destroyed, so
/// there is no reason to demand a detach first.
pub fn attach(
    store: &SpotStore,
    name: &str,
    directory: &Path,
) -> Result<AttachOutcome, SpotError> {
    let mut registry = store.load()?;
    if !registry.spots.contains_key(name) {
        return Err(SpotError::Unknown {
            name: name.to_owned(),
            available: registry.spots.keys().cloned().collect(),
        });
    }
    let directory = canonical(directory);
    let previous = registry
        .attachments
        .insert(directory.clone(), name.to_owned());
    store.save(&registry)?;
    Ok(AttachOutcome {
        directory,
        name: name.to_owned(),
        previous,
    })
}

/// Unbind `directory`. Exact match only — see
/// [`SpotError::NotAttached`].
pub fn detach(store: &SpotStore, directory: &Path) -> Result<DetachOutcome, SpotError> {
    let mut registry = store.load()?;
    let key = canonical(directory);
    let Some(name) = registry.attachments.remove(&key) else {
        return Err(SpotError::NotAttached {
            directory: key.clone(),
            // The exact lookup just missed, so any hit here is a
            // strict ancestor.
            ancestor: attached(&registry, &key),
        });
    };
    store.save(&registry)?;
    Ok(DetachOutcome {
        directory: key,
        name,
    })
}
```

- [ ] **Step 7: Prune attachments in `remove`**

In `RemoveOutcome`, add:

```rust
    /// Directories that were attached to this spot and are no longer
    /// bound to anything.
    pub detached: Vec<PathBuf>,
```

In `remove`, after the `current` clear and before `store.save(&registry)?`:

```rust
    // An attachment naming an unregistered spot would resolve to a
    // bare "unknown spot" on the next command, so drop them with the
    // entry — the same cascade `current` gets.
    let detached: Vec<PathBuf> = registry
        .attachments
        .iter()
        .filter(|(_, spot)| spot.as_str() == name)
        .map(|(directory, _)| directory.clone())
        .collect();
    for directory in &detached {
        registry.attachments.remove(directory);
    }
```

and add `detached` to the returned `RemoveOutcome`.

- [ ] **Step 8: Run the tests to verify they pass**

Run: `cargo test -p tonk-cli --lib spot::tests::attaching`
Expected: PASS, 4 tests.

- [ ] **Step 9: Fix the one existing consumer of `RemoveOutcome`**

`RemoveOutcome` gained a field, so `tests/spot.rs` and `src/bin/tonk.rs` still compile (neither constructs one), but confirm:

Run: `cargo test -p tonk-cli --test spot`
Expected: PASS, unchanged.

- [ ] **Step 10: Commit**

```bash
git add rust/tonk-cli/src/spot.rs
git commit -m "feat(tonk-cli): store directory attachments in the spot registry"
```

---

### Task 2: The resolution tier

**Files:**
- Modify: `rust/tonk-cli/src/spot.rs`
- Modify: `rust/tonk-cli/src/bin/tonk.rs:1050,2095` (call sites only — the CLI surface comes in Task 3)
- Modify: `rust/tonk-cli/tests/spot.rs:25,115`

**Interfaces:**
- Consumes: `attached()`, `Registry.attachments` from Task 1.
- Produces:
  - `Source::Attached(PathBuf)`; `Source` now derives `Clone` instead of `Copy`
  - `SpotStore::resolve(&self, flag: Option<&str>, env: Option<&str>, cwd: Option<&Path>) -> Result<Resolved, SpotError>`
  - `spot::listing(store: &SpotStore, flag: Option<&str>, env: Option<&str>, cwd: Option<&Path>) -> Result<Listing, SpotError>`
  - `Listing.attachments: Vec<(PathBuf, String)>`

- [ ] **Step 1: Write the failing tests**

Add to the existing `mod resolving` in `src/spot.rs`:

```rust
        /// A registry with two spots, `b` selected globally, and
        /// `/proj` attached to `a`.
        fn attached_registry() -> Registry {
            let mut registry = registry_with(&[("a", "/s/a"), ("b", "/s/b")], Some("b"));
            registry
                .attachments
                .insert(PathBuf::from("/proj"), "a".to_owned());
            registry
        }

        #[dialog_common::test]
        fn it_prefers_an_attachment_over_the_global_selection() {
            let (_tmp, store) = store();
            store.save(&attached_registry()).expect("save");

            let resolved = store
                .resolve(None, None, Some(Path::new("/proj/sub/deep")))
                .expect("attached");
            assert_eq!(resolved.name, "a");
            assert_eq!(resolved.source, Source::Attached(PathBuf::from("/proj")));
        }

        #[dialog_common::test]
        fn it_takes_the_deepest_attachment() {
            let (_tmp, store) = store();
            let mut registry = attached_registry();
            registry
                .attachments
                .insert(PathBuf::from("/proj/sub"), "b".to_owned());
            store.save(&registry).expect("save");

            let resolved = store
                .resolve(None, None, Some(Path::new("/proj/sub/deep")))
                .expect("attached");
            assert_eq!(resolved.name, "b");
            assert_eq!(
                resolved.source,
                Source::Attached(PathBuf::from("/proj/sub"))
            );
        }

        #[dialog_common::test]
        fn it_falls_back_to_the_global_selection_outside_any_attachment() {
            let (_tmp, store) = store();
            store.save(&attached_registry()).expect("save");

            let resolved = store
                .resolve(None, None, Some(Path::new("/elsewhere")))
                .expect("global");
            assert_eq!(resolved.name, "b");
            assert_eq!(resolved.source, Source::Global);
        }

        #[dialog_common::test]
        fn it_prefers_the_environment_over_an_attachment() {
            let (_tmp, store) = store();
            store.save(&attached_registry()).expect("save");

            let resolved = store
                .resolve(None, Some("b"), Some(Path::new("/proj")))
                .expect("env");
            assert_eq!(resolved.name, "b");
            assert_eq!(resolved.source, Source::Env);
        }

        #[dialog_common::test]
        fn it_prefers_the_flag_over_an_attachment() {
            let (_tmp, store) = store();
            store.save(&attached_registry()).expect("save");

            let resolved = store
                .resolve(Some("b"), None, Some(Path::new("/proj")))
                .expect("flag");
            assert_eq!(resolved.name, "b");
            assert_eq!(resolved.source, Source::Flag);
        }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p tonk-cli --lib spot::tests::resolving`
Expected: FAIL to compile — `this method takes 2 arguments but 3 arguments were supplied`.

- [ ] **Step 3: Add the `Attached` source variant**

In `src/spot.rs`, change the `Source` derive and add the variant:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// `--spot` flag.
    Flag,
    /// [`SPOT_ENV`] environment variable.
    Env,
    /// An attachment on the cwd or one of its ancestors. Carries the
    /// attached directory, so output can say *which* one answered.
    Attached(PathBuf),
    /// The registry's `current` field.
    Global,
}
```

`Copy` is gone because the variant carries a `PathBuf`. Update `Display`:

```rust
impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Source::Flag => f.write_str("flag"),
            Source::Env => f.write_str("env"),
            Source::Attached(directory) => write!(f, "attached {}", directory.display()),
            Source::Global => f.write_str("global"),
        }
    }
}
```

- [ ] **Step 4: Insert the tier in `resolve`**

Replace the signature and the selection chain in `SpotStore::resolve`:

```rust
    /// Resolve the spot a command should operate on.
    ///
    /// Strict precedence: `flag` (`--spot`) > `env` ([`SPOT_ENV`],
    /// already read and empty-filtered by the caller) > a directory
    /// attachment at or above `cwd` > the registry's `current`.
    ///
    /// `cwd` is passed in rather than read here so nothing depends on
    /// process-global state, and it is only ever a key into the
    /// registry: the directory never locates site data.
    ///
    /// `SPOT_ENV` outranks attachments deliberately. A harness that
    /// pinned a spot for the process must not be overridden by
    /// whatever directory it happened to launch in.
    pub fn resolve(
        &self,
        flag: Option<&str>,
        env: Option<&str>,
        cwd: Option<&Path>,
    ) -> Result<Resolved, SpotError> {
        let registry = self.load()?;
        let (name, source) = if let Some(name) = flag {
            (name.to_owned(), Source::Flag)
        } else if let Some(name) = env {
            (name.to_owned(), Source::Env)
        } else if let Some((directory, name)) = cwd.and_then(|cwd| attached(&registry, cwd)) {
            (name, Source::Attached(directory))
        } else if let Some(name) = registry.current.clone() {
            (name, Source::Global)
        } else if registry.spots.is_empty() {
            return Err(SpotError::NothingRegistered);
        } else {
            return Err(SpotError::NoSelection);
        };
        match registry.spots.get(&name) {
            Some(entry) => Ok(Resolved {
                name,
                site: entry.site.clone(),
                source,
            }),
            None => Err(SpotError::Unknown {
                name,
                available: registry.spots.keys().cloned().collect(),
            }),
        }
    }
```

Also update the module header's selection paragraph:

```rust
//! Selection resolves `--spot` > `TONK_SPOT` > a directory
//! attachment (the nearest attached ancestor of the cwd) > the
//! registry's `current`. The flag and env forms are per-invocation /
//! per-process, so concurrent sessions pinning their own spot can
//! never mix regardless of who rewrites the shared `current`;
//! attachments give a session that lives in a directory the same
//! isolation without repeating itself every invocation. A directory
//! is only ever a key into the registry — it never locates data.
```

And the `NoSelection` message:

```rust
    #[error(
        "no spot selected; run `tonk use <name>`, add --here to bind this \
         directory, pass --spot, or set TONK_SPOT"
    )]
    NoSelection,
```

- [ ] **Step 5: Extend `listing`**

```rust
pub fn listing(
    store: &SpotStore,
    flag: Option<&str>,
    env: Option<&str>,
    cwd: Option<&Path>,
) -> Result<Listing, SpotError> {
    let registry = store.load()?;
    let rows = registry
        .spots
        .iter()
        .map(|(name, entry)| (name.clone(), entry.site.clone()))
        .collect();
    let attachments = registry
        .attachments
        .iter()
        .map(|(directory, name)| (directory.clone(), name.clone()))
        .collect();
    Ok(Listing {
        rows,
        attachments,
        current: store.resolve(flag, env, cwd).ok(),
    })
}
```

and add the field to `struct Listing`:

```rust
    /// `(directory, spot)` per attachment, in path order.
    pub attachments: Vec<(PathBuf, String)>,
```

- [ ] **Step 6: Update the existing call sites**

Four places pass the new argument:

- `src/bin/tonk.rs:1050` — `tonk_cli::spot::listing(&store, flag, env.as_deref(), None)` for now; Task 3 replaces `None` with the real cwd.
- `src/bin/tonk.rs:2095` — `store.resolve(flag, env.as_deref(), None)` for now; same.
- `tests/spot.rs:25` — `store.resolve(None, None, None)?`
- `tests/spot.rs:115` — `spot::listing(&store, None, None, None)?`

Also in `src/spot.rs`'s existing `it_prefers_flag_over_env_over_global`, add the third argument to all three calls, and split the tuple assertions so a mismatch names which half failed (`Source` no longer being `Copy` does not break them — the tuple moves a disjoint field — but the split reads better next to the new tests):

```rust
            let flag = store.resolve(Some("a"), Some("b"), None).expect("flag");
            assert_eq!(flag.name, "a");
            assert_eq!(flag.source, Source::Flag);

            let env = store.resolve(None, Some("b"), None).expect("env");
            assert_eq!(env.name, "b");
            assert_eq!(env.source, Source::Env);

            let global = store.resolve(None, None, None).expect("global");
            assert_eq!(global.name, "c");
            assert_eq!(global.source, Source::Global);
            assert_eq!(global.site, PathBuf::from("/s/c"));
```

The remaining `resolve` calls in `mod resolving` (`it_errors_when_nothing_is_selected`, `it_hints_spot_new_when_the_registry_is_empty`, `it_errors_on_an_unknown_name_listing_available`) each take a trailing `None`.

One more borrow fix in `src/bin/tonk.rs:1069` — `resolved` is a `&Resolved` there, so the moved-out `source` must become a borrow:

```rust
                            source = &resolved.source,
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p tonk-cli --lib spot:: && cargo test -p tonk-cli --test spot`
Expected: PASS both.

- [ ] **Step 8: Commit**

```bash
git add rust/tonk-cli/src/spot.rs rust/tonk-cli/src/bin/tonk.rs rust/tonk-cli/tests/spot.rs
git commit -m "feat(tonk-cli): resolve a spot from a directory attachment"
```

---

### Task 3: CLI surface

**Files:**
- Modify: `rust/tonk-cli/src/bin/tonk.rs`
- Modify: `rust/tonk-cli/tests/cli_spot.rs`

**Interfaces:**
- Consumes: `spot::attach`, `spot::detach`, `spot::AttachOutcome`, `spot::DetachOutcome`, `Listing.attachments`, `RemoveOutcome.detached`, `SpotStore::resolve(flag, env, cwd)` from Tasks 1–2.
- Produces: `tonk use <name> --here`, `tonk spot detach [PATH]`, an `attached:` block in `tonk spot list`, and `fn working_directory() -> Option<PathBuf>` in `bin/tonk.rs`.

- [ ] **Step 1: Write the failing tests**

Add to `tests/cli_spot.rs`. First a cwd-aware runner beside the existing `run`:

```rust
/// Same isolation as [`tonk_cmd`], run *from* `cwd`. The attachment
/// tier keys off the working directory, so these have to control it
/// rather than inherit the test runner's.
fn run_in(state_dir: &Path, cwd: &Path, args: &[&str], extra_env: &[(&str, &str)]) -> Output {
    tonk_cmd(state_dir, args, extra_env)
        .current_dir(cwd)
        .output()
        .expect("tonk binary runs")
}
```

Then the behaviour module, at the end of the file:

```rust
mod when_a_directory_is_attached {
    use super::*;

    /// Two registered spots with `b` selected globally, plus a real
    /// `work/nested/` tree to attach and run from. Site paths need
    /// not hold a repo: resolution and its error text are what is
    /// under test, not site opening.
    fn fixture(state: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
        let a = state.join("site-a");
        let b = state.join("site-b");
        std::fs::create_dir_all(&a).expect("mkdir a");
        std::fs::create_dir_all(&b).expect("mkdir b");
        write_registry(state, &[("a", &a), ("b", &b)], Some("b"));

        let work = state.join("work");
        let nested = work.join("nested");
        std::fs::create_dir_all(&nested).expect("mkdir work/nested");
        (work, nested)
    }

    /// The CLI stores canonicalized paths, and on macOS a tempdir
    /// under `/var/...` canonicalizes to `/private/var/...`, so
    /// assertions on printed paths have to canonicalize too.
    fn shown(path: &Path) -> String {
        path.canonicalize()
            .expect("canonicalize")
            .display()
            .to_string()
    }

    fn attach(state: &Path, cwd: &Path, name: &str) {
        let output = run_in(state, cwd, &["use", name, "--here"], &[]);
        assert!(output.status.success(), "{}", stderr_of(&output));
    }

    #[dialog_common::test]
    fn it_resolves_the_attachment_from_a_subdirectory() {
        let state = tempfile::tempdir().expect("tempdir");
        let (work, nested) = fixture(state.path());
        attach(state.path(), &work, "a");

        let output = run_in(state.path(), &nested, &["status"], &[]);
        assert!(!output.status.success());
        let stderr = stderr_of(&output);
        assert!(stderr.contains("spot 'a' (via attached"), "{stderr}");
        assert!(stderr.contains(&shown(&work)), "{stderr}");
    }

    #[dialog_common::test]
    fn it_leaves_the_global_selection_alone() {
        let state = tempfile::tempdir().expect("tempdir");
        let (work, _nested) = fixture(state.path());
        attach(state.path(), &work, "a");

        let elsewhere = state.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).expect("mkdir elsewhere");
        let output = run_in(state.path(), &elsewhere, &["spot", "list"], &[]);
        let stdout = stdout_of(&output);
        assert!(stdout.contains("current: b (global)"), "{stdout}");
    }

    #[dialog_common::test]
    fn it_prefers_tonk_spot_over_an_attachment() {
        let state = tempfile::tempdir().expect("tempdir");
        let (work, nested) = fixture(state.path());
        attach(state.path(), &work, "a");

        let output = run_in(state.path(), &nested, &["status"], &[("TONK_SPOT", "b")]);
        assert!(!output.status.success());
        let stderr = stderr_of(&output);
        assert!(stderr.contains("spot 'b' (via env"), "{stderr}");
    }

    #[dialog_common::test]
    fn it_takes_the_deepest_attachment() {
        let state = tempfile::tempdir().expect("tempdir");
        let (work, nested) = fixture(state.path());
        attach(state.path(), &work, "a");
        attach(state.path(), &nested, "b");

        let output = run_in(state.path(), &nested, &["status"], &[]);
        assert!(!output.status.success());
        let stderr = stderr_of(&output);
        assert!(stderr.contains("spot 'b' (via attached"), "{stderr}");
    }

    #[dialog_common::test]
    fn it_lists_attachments() {
        let state = tempfile::tempdir().expect("tempdir");
        let (work, _nested) = fixture(state.path());
        attach(state.path(), &work, "a");

        let output = run_in(state.path(), state.path(), &["spot", "list"], &[]);
        let stdout = stdout_of(&output);
        assert!(stdout.contains("attached:"), "{stdout}");
        assert!(stdout.contains(&shown(&work)), "{stdout}");
    }

    #[dialog_common::test]
    fn it_refuses_to_detach_from_a_subdirectory() {
        let state = tempfile::tempdir().expect("tempdir");
        let (work, nested) = fixture(state.path());
        attach(state.path(), &work, "a");

        let refused = run_in(state.path(), &nested, &["spot", "detach"], &[]);
        assert!(!refused.status.success());
        let stderr = stderr_of(&refused);
        assert!(stderr.contains("is attached to a"), "{stderr}");

        let detached = run_in(state.path(), &work, &["spot", "detach"], &[]);
        assert!(detached.status.success(), "{}", stderr_of(&detached));

        let output = run_in(state.path(), &nested, &["status"], &[]);
        let stderr = stderr_of(&output);
        assert!(stderr.contains("spot 'b' (via global"), "{stderr}");
    }

    #[dialog_common::test]
    fn it_reports_the_previous_binding_on_reattach() {
        let state = tempfile::tempdir().expect("tempdir");
        let (work, _nested) = fixture(state.path());
        attach(state.path(), &work, "a");

        let output = run_in(state.path(), &work, &["use", "b", "--here"], &[]);
        assert!(output.status.success(), "{}", stderr_of(&output));
        let stdout = stdout_of(&output);
        assert!(stdout.contains("to b (was a)"), "{stdout}");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p tonk-cli --test cli_spot when_a_directory_is_attached`
Expected: FAIL — `unexpected argument '--here' found`.

- [ ] **Step 3: Add the cwd helper and wire resolution to it**

In `src/bin/tonk.rs`, add beside `open_selected`:

```rust
/// The process's working directory, used only as a key into the
/// attachment map. A cwd the OS refuses to report (deleted out from
/// under the process) is not fatal — resolution just skips the
/// attachment tier and falls through to the global selection.
fn working_directory() -> Option<PathBuf> {
    std::env::current_dir().ok()
}
```

In `open_selected`, replace the `resolve` call:

```rust
    let cwd = working_directory();
    let resolved = match store.resolve(flag, env.as_deref(), cwd.as_deref()) {
```

- [ ] **Step 4: Add `--here` to `tonk use`**

Change the `Use` variant in `enum Command`:

```rust
    /// Select the current spot (used by every command from anywhere)
    ///
    /// Without `--here` the selection is global to this machine.
    /// Concurrent sessions (agents, CI) should bind their directory
    /// with `--here`, or pin per-process with --spot / TONK_SPOT,
    /// rather than relying on the global selection.
    #[command(after_help = "Examples:\n  tonk use garden\n  tonk use garden --here")]
    Use {
        /// A registered spot name (see `tonk spot list`).
        #[arg(value_name = "NAME")]
        name: String,
        /// Bind this directory to the spot instead of changing the
        /// machine-global selection. Commands run from here or any
        /// subdirectory resolve to it, so sessions in different
        /// directories never clobber each other. Unbind with
        /// `tonk spot detach`.
        #[arg(long)]
        here: bool,
    },
```

Update the dispatch arm:

```rust
        Command::Use { name, here } => use_op(name, here).await,
```

and `use_op`:

```rust
/// `tonk use` — set the global current spot, or bind this directory
/// to one with `--here`.
async fn use_op(name: String, here: bool) -> ExitCode {
    let store = match tonk_cli::spot::SpotStore::open() {
        Ok(store) => store,
        Err(err) => return print_error(err.to_string()),
    };
    if here {
        let Some(cwd) = working_directory() else {
            return print_error("could not read the current directory".to_owned());
        };
        return match tonk_cli::spot::attach(&store, &name, &cwd) {
            Ok(outcome) => {
                let was = match &outcome.previous {
                    Some(previous) => format!(" (was {previous})"),
                    None => String::new(),
                };
                println!(
                    "attached {directory} to {name}{was}",
                    directory = outcome.directory.display(),
                    name = outcome.name,
                );
                ExitCode::Success
            }
            Err(err) => print_error(err.to_string()),
        };
    }
    match tonk_cli::spot::select(&store, &name) {
        Ok(resolved) => {
            println!(
                "current spot: {name} ({site})",
                name = resolved.name,
                site = resolved.site.display(),
            );
            ExitCode::Success
        }
        Err(err) => print_error(err.to_string()),
    }
}
```

- [ ] **Step 5: Add `tonk spot detach`**

Add to `enum SpotCommand`:

```rust
    /// Unbind a directory from its spot (see `tonk use --here`)
    ///
    /// Matches exactly: run from the directory that was attached,
    /// not a subdirectory of it.
    #[command(
        after_help = "Examples:\n  tonk spot detach\n  tonk spot detach ~/old-project"
    )]
    Detach {
        /// Directory to unbind. Default: the current directory. Pass
        /// a path to clear an entry whose directory no longer exists.
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,
    },
```

Add the arm to `spot_op`:

```rust
        SpotCommand::Detach { path } => {
            let directory = match path.or_else(working_directory) {
                Some(directory) => directory,
                None => return print_error("could not read the current directory".to_owned()),
            };
            match tonk_cli::spot::detach(&store, &directory) {
                Ok(outcome) => {
                    println!(
                        "detached {directory} from {name}",
                        directory = outcome.directory.display(),
                        name = outcome.name,
                    );
                    ExitCode::Success
                }
                Err(err) => print_error(err.to_string()),
            }
        }
```

And to `descriptor`, inside the `Command::Spot` match:

```rust
                SpotCommand::Detach { .. } => "detach",
```

Also report whether `use` bound a directory (a static string, never a value):

```rust
        Command::Use { here, .. } => ("use", here.then_some("here")),
```

- [ ] **Step 6: Show attachments in `spot list` and detachments in `spot rm`**

In `spot_op`'s `SpotCommand::List` arm, pass the cwd:

```rust
            let cwd = working_directory();
            match tonk_cli::spot::listing(&store, flag, env.as_deref(), cwd.as_deref()) {
```

and after the `current:` line, before `ExitCode::Success`:

```rust
                    if !listing.attachments.is_empty() {
                        println!();
                        println!("attached:");
                        for (directory, name) in &listing.attachments {
                            println!("  {directory}\t{name}", directory = directory.display());
                        }
                    }
```

In the `SpotCommand::Rm` arm, after the site line:

```rust
                for directory in &outcome.detached {
                    println!("detached {}", directory.display());
                }
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p tonk-cli --test cli_spot`
Expected: PASS — the seven new tests plus every pre-existing one.

- [ ] **Step 8: Run the whole crate's tests**

Run: `cargo test -p tonk-cli`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add rust/tonk-cli/src/bin/tonk.rs rust/tonk-cli/tests/cli_spot.rs
git commit -m "feat(tonk-cli): bind a directory to a spot with tonk use --here"
```

---

### Task 4: Documentation

The resolution order is stated in five places outside the code, all of which are now wrong. `bench/README.md` is the sharpest: it currently asserts the CLI "*never* consults the cwd".

**Files:**
- Modify: `rust/tonk-cli/README.md:91-93`
- Modify: `rust/tonk-cli/src/guide-index.md:30-37`
- Modify: `README.md:54`
- Modify: `bench/README.md:146-150`
- Modify: `.claude/commands/tonk.md:8-11`

**Interfaces:**
- Consumes: the CLI surface from Task 3. No code changes.
- Produces: nothing consumed by later tasks.

- [ ] **Step 1: Update the CLI README**

Replace the sentence at `rust/tonk-cli/README.md:91-92`:

```markdown
Commands resolve which spot to use as `--spot` > `TONK_SPOT` > a directory
attachment > the `tonk use` selection, then open its site. An attachment is a
directory bound to a spot with `tonk use <name> --here`; the nearest attached
ancestor of the working directory wins, so parallel sessions in separate
directories each hold their own spot without passing a flag. The directory is
only a key into the registry — nothing about a site is stored there, and
`tonk spot detach` unbinds it. `spots.json` is plain JSON, so any application
can read the registry without going through the CLI.
```

- [ ] **Step 2: Update the agent guide**

Replace the `## Spots` section at `rust/tonk-cli/src/guide-index.md:30-37`:

```markdown
## Spots

Commands run against the selected *spot* (a named fact store), not
the cwd. Resolution order: `--spot <name>` > `TONK_SPOT` env > a
directory attachment > `tonk use <name>` selection. In automation,
pin the spot per-process (`TONK_SPOT=x tonk ...` or `--spot x`) —
never rely on bare `tonk use`, which is shared global state another
session can change. An agent that works out of one directory can
instead bind it once with `tonk use <name> --here` and drop the flag
afterwards; `tonk spot detach` unbinds it. `tonk spot list` shows
what's registered, what's attached, and what is current.
```

- [ ] **Step 3: Update the root README**

At `README.md:54`, replace `selected with \`tonk use\`, \`--spot\`, or \`TONK_SPOT\`` with:

```markdown
selected with `tonk use`, `--spot`, `TONK_SPOT`, or a directory bound to it by `tonk use <name> --here`
```

- [ ] **Step 4: Update the bench README**

Replace the paragraph at `bench/README.md:146-150`:

```markdown
**Spot pinning** — the CLI resolves a spot by `--spot`, then `TONK_SPOT`, then
a directory attachment (`tonk use <name> --here`), then the `tonk use`
selection. `cd`-ing into the site directory does nothing on its own — only an
explicit attachment makes a directory mean anything — and an unpinned `tonk`
call succeeds against whatever spot the developer happens to have selected
globally, silently, against the wrong repo. `TONK_SPOT` outranks attachments
precisely so the harness stays authoritative over whatever the developer has
bound locally. `run.sh` therefore exports both:
```

- [ ] **Step 5: Update the tonk skill**

Replace `.claude/commands/tonk.md:8-11`:

```markdown
Commands run from anywhere, against whichever spot is selected —
resolution is `--spot` > `TONK_SPOT` > a directory attached with
`tonk use <name> --here` > `tonk use`. Automation (agents, CI) should
set `TONK_SPOT` or pass `--spot` rather than relying on the global
`tonk use` selection; an agent working out of a fixed directory can
attach it once instead.
```

- [ ] **Step 6: Update the `--spot` help text**

In `src/bin/tonk.rs`, the global flag's doc comment:

```rust
    /// Operate on this spot instead of the selected one.
    /// Precedence: --spot > TONK_SPOT > directory attachment >
    /// `tonk use` selection.
    #[arg(long, global = true, value_name = "NAME")]
    spot: Option<String>,
```

- [ ] **Step 7: Verify the docs match the binary**

Run: `cargo run -p tonk-cli --bin tonk -- use --help` and `cargo run -p tonk-cli --bin tonk -- spot detach --help`
Expected: both render, and the text matches what the READMEs claim.

Run: `cargo test -p tonk-cli`
Expected: PASS (the guide text is compiled into the binary via `guide-index.md`, so a broken edit shows up in `tests/telemetry.rs`'s guide run).

- [ ] **Step 8: Run the lint gate**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets --all-features`
Expected: no warnings. `--all-features` matters — it compiles the integration tests, so a per-crate run can be green while the gate is not.

- [ ] **Step 9: Commit**

```bash
git add README.md bench/README.md .claude/commands/tonk.md rust/tonk-cli/README.md rust/tonk-cli/src/guide-index.md rust/tonk-cli/src/bin/tonk.rs
git commit -m "docs(tonk-cli): document the directory attachment tier"
```
