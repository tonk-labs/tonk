# CLI Invite Remote Origin Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `tonk invite` build its link on the resolved remote's own origin, so the link, the embedded `remote=`, and the shortcut service are always one deployment — and remove the unused `tonk share` that duplicated the remote-resolution logic.

**Architecture:** Two stacked PRs onto `staging`. PR 1 deletes `tonk share` outright (source, CLI surface, tests, docs) and ports its one real caller, `bench/bin/shots.sh`, onto `tonk push` plus a direct entity lookup. PR 2 adds a pure `invite::base_url_for_remote` helper and a `remote::resolve` remote-picker, wires both into `mint_invite` in `bin/tonk.rs`, and retargets `DEFAULT_BASE_URL` to `https://tonk.spot/join` for the no-remote fallback.

**Tech Stack:** Rust, clap 4 derive API, `url::Url`, `#[dialog_common::test]`, `nix develop` dev shell.

**Spec:** `docs/superpowers/specs/2026-07-22-cli-invite-remote-origin-design.md`

## Global Constraints

- Base both PRs on `origin/staging`, not `main`. The tonk-labs/tonk default branch is `staging`.
- Every test uses `#[dialog_common::test]`, never `#[test]` or `#[tokio::test]`.
- Every new `#[cfg(test)] mod tests` block includes the wasm configure guard:
  ```rust
  #[cfg(target_arch = "wasm32")]
  use wasm_bindgen_test::wasm_bindgen_test_configure;
  #[cfg(target_arch = "wasm32")]
  wasm_bindgen_test_configure!(run_in_browser);
  ```
- Test names are `it_does_x`, grouped in `mod when_<situation>` blocks.
- No emojis in code, commits, or output.
- Conventional Commits: `type(scope): subject`, imperative mood, lowercase, no trailing period.
- Do not name "Phase N" or reference this plan or the RFC in source comments or tests. Code stands on its own.
- The lint gate is `nix flake check` — workspace `clippy --all-targets --all-features` plus `cargo fmt --check`. `--all-features` compiles the integration tests, so a per-crate no-features clippy can be green while the gate fails.

---

# PR 1 — Remove `tonk share`

Branch: `refactor/drop-cli-share`, cut from the current `fix/cli-invite` HEAD (which is `origin/staging` plus the design spec commit `4c4ed1ec9`).

### Task 1: Delete the share module and its CLI surface

**Files:**
- Delete: `rust/tonk-cli/src/share.rs` (618 lines)
- Delete: `rust/tonk-cli/SHARE.md`
- Modify: `rust/tonk-cli/src/lib.rs:45`
- Modify: `rust/tonk-cli/src/bin/tonk.rs` (six regions, listed per step)
- Modify: `rust/tonk-cli/tests/site.rs` (three test mods, lines 851-1469)

**Interfaces:**
- Consumes: nothing.
- Produces: nothing. `tonk_cli::share` and every `Share*` type stop existing. `crate::invite`, `crate::remote`, `crate::schema`, `crate::views`, `crate::sync`, and `crate::site` are untouched — `share.rs` only consumed them.

- [ ] **Step 1: Cut the branch**

```bash
cd /Users/jackdouglas/tonk/tonk-invite
git switch -c refactor/drop-cli-share
```

- [ ] **Step 2: Delete the module and its doc**

```bash
git rm rust/tonk-cli/src/share.rs rust/tonk-cli/SHARE.md
```

- [ ] **Step 3: Drop the module declaration**

In `rust/tonk-cli/src/lib.rs`, delete line 45:

```rust
pub mod share;
```

The surrounding `pub mod` list is alphabetical; removing the line leaves `pub mod schema;` followed by `pub mod site;`.

- [ ] **Step 4: Drop the `Share` variant from `Command`**

In `rust/tonk-cli/src/bin/tonk.rs`, delete lines 220-224:

```rust
    /// Push, then mint a launcher URL onto a live view
    Share {
        #[command(subcommand)]
        command: ShareCommand,
    },
```

Keep line 219, the `// -- collab ---` section banner — it heads the whole collab group (`Invite`, `Join`, `Remote`), not just `Share`. After the edit the banner sits directly above `/// Mint an invite URL granting access to this repo`.

- [ ] **Step 5: Delete the `ShareCommand` enum**

In `rust/tonk-cli/src/bin/tonk.rs`, delete lines 384-480 — from `#[derive(Subcommand, Debug)]` above `enum ShareCommand {` through its closing `}`. The next item, `#[derive(Subcommand, Debug)] enum RemoteCommand {`, stays.

- [ ] **Step 6: Drop the telemetry arm**

In `rust/tonk-cli/src/bin/tonk.rs`, delete lines 754-761:

```rust
        Command::Share { command } => (
            "share",
            Some(match command {
                ShareCommand::Concept { .. } => "concept",
                ShareCommand::View { .. } => "view",
                ShareCommand::Display { .. } => "display",
            }),
        ),
```

- [ ] **Step 7: Drop the dispatch arm**

In `rust/tonk-cli/src/bin/tonk.rs`, delete line 856:

```rust
        Command::Share { command } => share_op(command, spot.as_deref()).await,
```

- [ ] **Step 8: Delete `share_op` and the three printers**

In `rust/tonk-cli/src/bin/tonk.rs`, delete lines 1404-1526 — `async fn share_op` through the end of `fn print_share_display_outcome`. The next item, `async fn mint_invite`, stays.

- [ ] **Step 9: Drop the share import**

In `rust/tonk-cli/src/bin/tonk.rs`, delete line 24:

```rust
use tonk_cli::share::{self, ShareDisplayOutcome, ShareOptions, ShareOutcome, ShareViewOutcome};
```

- [ ] **Step 10: Delete the share test modules**

In `rust/tonk-cli/tests/site.rs`, delete three consecutive modules — lines 851-1469 inclusive:

- `mod when_sharing_a_view {` (851-1001)
- `mod when_sharing_a_concept {` (1003-1241)
- `mod when_sharing_a_display {` (1243-1469)

`mod when_listing_views {` (766-849) stays above; `mod when_migrating_from_carry {` (1471) stays below.

- [ ] **Step 11: Verify the crate builds with no share left**

```bash
cd /Users/jackdouglas/tonk/tonk-invite
cargo build -p tonk-cli 2>&1 | tail -20
grep -rn "share::\|ShareCommand\|ShareOptions\|share_op\|print_share" rust/tonk-cli/src rust/tonk-cli/tests
```

Expected: build succeeds; the grep prints nothing.

If the build reports newly-unused imports in `bin/tonk.rs` (for example a `views` or `schema` import that only `share_op` used), delete those import lines too and rebuild.

- [ ] **Step 12: Run the affected test binary**

```bash
cargo test -p tonk-cli --features integration-tests --test site 2>&1 | tail -20
```

Expected: PASS, with the three `when_sharing_*` modules gone from the output.

- [ ] **Step 13: Confirm the CLI no longer offers `share`**

```bash
cargo run -p tonk-cli --bin tonk -- --help 2>&1 | grep -c share
```

Expected: `0`.

- [ ] **Step 14: Commit**

```bash
git add -A rust/tonk-cli
git commit -m "refactor(cli): remove tonk share

Unused. It was also the second caller of the remote-resolution
heuristic and the only producer of the name=/then= launcher URL,
so removing it first keeps the invite change small.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Purge share from the docs and agent guides

The `guide-*.md` files ship to agents through `tonk guide`, so a dangling `tonk share` there is a live footgun, not just stale prose.

**Files:**
- Modify: `rust/tonk-cli/src/bin/tonk.rs:33` (the `Cli` `after_help` banner)
- Modify: `rust/tonk-cli/README.md:65-67`
- Modify: `rust/tonk-cli/src/guide-index.md:65`
- Modify: `rust/tonk-cli/src/guide-views.md:61,204-206`
- Modify: `rust/tonk-cli/src/guide-events.md:422-468`
- Modify: `rust/tonk-cli/src/guide-workspace.md:82-83`
- Modify: `rust/tonk-cli/src/schema.rs:50`
- Modify: `rust/tonk-cli/src/views.rs:13,76,200`
- Modify: `.claude/commands/tonk.md:87-88`

**Interfaces:**
- Consumes: nothing.
- Produces: nothing. Prose only.

Every replacement below points the reader at `tonk invite`, which now carries the remote (PR 2) and is the only way to hand someone a link.

- [ ] **Step 1: The `--help` banner**

Task 1 deleted the subcommand but left the top-level help text advertising it. In `rust/tonk-cli/src/bin/tonk.rs`, the `Cli` struct's `after_help` string (line 33) names `share` twice:

```
The loop: orient, define concepts, assert facts, give them a view, share.
...
  collab   share · invite · join · push · pull · remote
```

Change the `collab` row to `invite · join · push · pull · remote`, and rewrite the opening sentence so its last beat is a verb the CLI still has. `tonk invite` is what a human reaches for now.

This is a string literal inside `#[command(...)]`, all on one line with `\n` escapes — edit it in place, keep it a single line, and keep the two-space column alignment of the rows around it.

- [ ] **Step 2: README**

In `rust/tonk-cli/README.md`, delete lines 65-67:

```
tonk share concept person
tonk share view my-page
tonk share display alice --view person-card
```

Replace with:

```
tonk invite
```

Adjust the surrounding prose so it reads as "mint an invite to the repo" rather than "share one view". Read the enclosing section first — do not leave a heading describing three flavours above a single command.

- [ ] **Step 3: guide-index.md**

Line 65 currently reads:

```
6. Share a live view: `tonk share display <entity> --view <name>`.
```

Replace with:

```
6. Hand the repo to someone: `tonk invite`.
```

- [ ] **Step 4: guide-views.md**

Line 61:

```
Share a display: `tonk share display <entity> --view <view-name>`
```

Replace with:

```
Hand the repo to someone: `tonk invite` (they land on the space and
navigate to the view themselves)
```

Lines 204-206 describe `tonk share view` versus `tonk share display`. Delete the whole paragraph — the distinction it draws no longer exists in the CLI. Read the surrounding section and make sure the remaining prose still flows.

- [ ] **Step 5: guide-events.md**

Lines 422-468 contain the largest block: an intro at 422, a fenced example at 425, a second example at 446, and a three-bullet comparison at 460-468. Replace the whole span with:

```
Once the view works locally, hand the repo to a collaborator with
`tonk invite`. They join the space and open the view there — events
fire in the live shell, not in a standalone page.
```

Read lines 400-480 before editing so the replacement lands in the right narrative position.

- [ ] **Step 6: guide-workspace.md**

Lines 82-83:

```
`tonk share display <workspace-entity>` (carousel of its sheets) or
`tonk share display alice --view person-card` for one sheet.
```

Replace with:

```
`tonk invite` — the recipient joins the space and opens the workspace
there.
```

- [ ] **Step 7: Source doc comments**

`rust/tonk-cli/src/schema.rs:50` — change `for `tonk concepts` to print and for `tonk share concept`` so it names only `tonk concepts`.

`rust/tonk-cli/src/views.rs` — three mentions:
- line 13: the module doc says `tonk share view` calls back into it. `tonk view ls` is now the only caller; reword to say so.
- line 76: "Used by `tonk share view` to refuse minting a…" — reword to describe what the function checks, without naming a caller.
- line 200: "`tonk share view` to resolve a positional name argument" — same treatment.

Read each doc comment in full before rewriting. Do not leave a sentence whose subject was the deleted command.

- [ ] **Step 8: `.claude/commands/tonk.md`**

Lines 87-88:

```
tonk share concept <name>                   # launcher URL onto a live concept view
tonk share display <subject> --view <name>  # launcher URL onto a <tonk-display> render
```

Replace with:

```
tonk invite                                 # invite link to this repo
```

If `tonk invite` is already listed elsewhere in that file, delete these two lines instead of replacing them.

- [ ] **Step 9: Verify nothing under `rust/` references the command**

```bash
cd /Users/jackdouglas/tonk/tonk-invite
grep -rn "tonk share" --include="*.md" --include="*.rs" --include="*.toml" --include="*.yaml" rust/ .claude/
```

Expected: no output. `bench/` still references it — Task 3 owns that, because bench genuinely executes the command and needs a working replacement, not a reworded sentence.

- [ ] **Step 10: Run the full native suite**

```bash
nix develop -c test:native:debug 2>&1 | tail -30
```

Expected: PASS.

- [ ] **Step 11: Lint gate**

```bash
nix flake check 2>&1 | tail -30
```

Expected: clean.

- [ ] **Step 12: Commit**

```bash
git add -A rust/ .claude/
git commit -m "docs(cli): drop tonk share from the guides and readmes

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Port the bench screenshot pipeline off `tonk share`

`bench/bin/shots.sh` executes `tonk share display` for real, so Task 1's deletion breaks the `display:<view-name>` checkpoint in six of the seven scenarios (`from-scratch`, `targeted-edit`, `smoke`, `wiki-conversion`, `artifact-conversion`, and whichever others carry one).

It does not want the launcher URL: stdout goes to `"$share_stderr.stdout"` and is deleted unread. The call buys exactly two things:

1. a push of the local repo to its upstream
2. the `subject: <name> (<entity>)` line on stderr, parsed to get the view's entity URI

Both have direct replacements. `tonk push` covers (1). For (2), `shots.sh` already runs `tonk eval --no-sync --format json -c 'view:'` in its very next step and filters those rows by `.this == $view_entity` — the entity it went to `share display` to obtain. Resolving the row by the view's own anchor name instead removes the need for the entity entirely.

**Files:**
- Modify: `bench/bin/shots.sh` (the `resolve_display` function, roughly lines 19-80)
- Modify: `bench/README.md:159`

**Interfaces:**
- Consumes: the `tonk` binary at `$TONK` (`$ROOT/target/release/tonk`), which after Task 1 has no `share` subcommand. `tonk push`, `tonk eval --no-sync --format json`, and `tonk view ls` all remain.
- Produces: nothing other tasks consume. `resolve_display` keeps its contract — given a view name it echoes a navigable URL suffix, or returns non-zero and the caller records the checkpoint as missing.

- [ ] **Step 1: Read the function and its caller in full**

```bash
cd /Users/jackdouglas/tonk/tonk-invite
cat bench/bin/shots.sh
```

Understand the whole file before editing. Note in particular: what `resolve_display` echoes on success, how the caller consumes it, and the existing fallback to `<view-name>!tonk:view` when the model lookup fails. Preserve all three.

- [ ] **Step 2: Confirm how a view's anchor name is queryable**

```bash
cd /Users/jackdouglas/tonk/tonk-invite
cargo build --release -p tonk-cli 2>&1 | tail -3
export TONK_SPOTS_STATE="$(mktemp -d)"
export TONK_SPOT=bench-probe
./target/release/tonk spot new bench-probe 2>&1 | tail -2
./target/release/tonk eval --no-sync --format json -c 'view:' | head -40
./target/release/tonk view ls 2>&1 | head -10
```

The probe spot is empty, so the interesting output is the *shape* of the JSON and whether `view:` rows carry a name field. If they do not, `tonk view ls` (tab-separated `name<TAB>entity`) is the lookup — use whichever actually resolves a name to an entity. Do not guess: run the commands and read the output.

If neither surfaces a name for a `view` row, stop and report — the port needs a CLI affordance that does not exist, and that is a plan gap, not something to work around in shell.

- [ ] **Step 3: Rewrite `resolve_display`**

Replace the `tonk share display` invocation with:

- `cd "$site" && "$TONK" push` for the push, keeping the existing failure handling shape (log to stderr, `return 1`, never fatal)
- the name→entity lookup you confirmed in Step 2, feeding the same `view_entity` variable the rest of the function already uses

Leave Steps 2 and 3 of the function's own documented strategy (model URI lookup, URL construction, the `<view-name>!tonk:view` fallback) untouched. Update the function's header comment so it describes what the code now does — it currently opens by naming `tonk share display`.

- [ ] **Step 4: Verify the script parses and the command is gone**

```bash
cd /Users/jackdouglas/tonk/tonk-invite
bash -n bench/bin/shots.sh && echo "syntax OK"
grep -n "share" bench/bin/shots.sh
```

Expected: `syntax OK`, and the grep prints nothing (or only the unrelated `$share_stderr` variable if you kept the name — rename it, since it no longer refers to sharing).

- [ ] **Step 5: Update `bench/README.md`**

Line 159 reads:

```
- `display:<view-name>` → resolved at capture time via `tonk share display`;
```

Reword to name the mechanism the script now uses. Read the two lines that follow — they describe the model query and URL construction and stay accurate.

- [ ] **Step 6: Run a scenario end to end**

```bash
cd /Users/jackdouglas/tonk/tonk-invite
ls bench/scenarios/
```

Run the `smoke` scenario — it is the smallest with a `display:` checkpoint. Find the entry point (`bench/README.md` documents it) and run it. Confirm `display:notes` produces a screenshot in the run's `shots/` directory rather than landing in the missing list.

If the bench harness cannot run in this environment (missing browser, missing API key, network sandbox), say so explicitly in your report rather than claiming the port works. A `bash -n` parse check is not evidence that a pipeline runs.

- [ ] **Step 7: Verify nothing anywhere references the command**

```bash
cd /Users/jackdouglas/tonk/tonk-invite
grep -rn "tonk share" --include="*.md" --include="*.rs" --include="*.sh" --include="*.toml" --include="*.yaml" . | grep -v target | grep -v docs/superpowers | grep -v bench/testdata
```

Expected: no output. `bench/testdata/codex-episode.jsonl` is a frozen recording of a past CLI session and stays as-is.

- [ ] **Step 8: Commit and open PR 1**

```bash
git add bench/
git commit -m "refactor(bench): resolve display checkpoints without tonk share

shots.sh called tonk share display only for its push side effect and
the view entity it echoed on stderr. Push directly and look the
entity up through the query the script already runs.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
git push -u origin refactor/drop-cli-share
gh pr create --base staging --title "refactor(cli): remove tonk share" --body "$(cat <<'EOF'
`tonk share` is unused. It was also the second caller of the
remote-resolution heuristic and the only producer of the
`name=`/`then=` launcher URL, so removing it clears the way for the
invite base-URL fix that follows.

Deletes `share.rs`, the `Share`/`ShareCommand` CLI surface, the three
`when_sharing_*` test modules, `SHARE.md`, and every guide reference.
Tonk-ui's consumer side is untouched: invites still land on `/join`,
nothing in the CLI emits `name=`/`then=` any more.

`bench/bin/shots.sh` was the one real caller — it used `share display`
for its push side effect and the view entity echoed on stderr, and
discarded the launcher URL. It now pushes directly and resolves the
entity through the query it already ran.

Spec: `docs/superpowers/specs/2026-07-22-cli-invite-remote-origin-design.md`

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

# PR 2 — Invite follows the remote

Branch: `fix/cli-invite`, rebased onto `refactor/drop-cli-share`.

### Task 4: `invite::base_url_for_remote`

**Files:**
- Modify: `rust/tonk-cli/src/invite.rs` (add the helper after `mint`, add a `#[cfg(test)] mod tests` at the end)

**Interfaces:**
- Consumes: `InviteError` (already defined at `invite.rs:84`), `url::Url` (already imported at `invite.rs:26`).
- Produces: `pub fn base_url_for_remote(endpoint: &str) -> Result<String, InviteError>`. Task 6 calls it with a `RemoteRecord::endpoint`, which is a `String`, not a `Url`.

- [ ] **Step 1: Switch to the PR 2 branch**

This is the first task of PR 2. Move onto its branch, stacked on PR 1's:

```bash
cd /Users/jackdouglas/tonk/tonk-invite
git switch fix/cli-invite
git rebase refactor/drop-cli-share
```

- [ ] **Step 2: Write the failing tests**

Append to `rust/tonk-cli/src/invite.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    mod when_deriving_a_base_url_from_a_remote {
        use super::*;

        #[dialog_common::test]
        fn it_replaces_the_access_service_path_with_join() {
            let base = base_url_for_remote("https://staging.tonk.xyz/ucan/").unwrap();
            assert_eq!(base, "https://staging.tonk.xyz/join");
        }

        #[dialog_common::test]
        fn it_handles_an_endpoint_with_no_path() {
            let base = base_url_for_remote("https://staging.tonk.xyz").unwrap();
            assert_eq!(base, "https://staging.tonk.xyz/join");
        }

        #[dialog_common::test]
        fn it_keeps_the_port_so_local_services_resolve() {
            let base = base_url_for_remote("http://127.0.0.1:8787/ucan/").unwrap();
            assert_eq!(base, "http://127.0.0.1:8787/join");
        }

        #[dialog_common::test]
        fn it_rejects_an_endpoint_that_is_not_a_url() {
            assert!(base_url_for_remote("not a url").is_err());
        }
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

```bash
cd /Users/jackdouglas/tonk/tonk-invite
cargo test -p tonk-cli --lib when_deriving_a_base_url_from_a_remote 2>&1 | tail -20
```

Expected: FAIL to compile — `cannot find function 'base_url_for_remote' in this scope`.

- [ ] **Step 4: Write the implementation**

In `rust/tonk-cli/src/invite.rs`, immediately after `mint` (which ends at line 197) and before the `claim` doc comment:

```rust
/// Derive the invite base URL from a remote's endpoint.
///
/// The invite has to live on the remote's own origin. That origin is
/// the deployment actually serving the repo, and — because the
/// shortcut service is same-origin by construction — the only one
/// whose `PUT /@` can answer. This is the CLI's stand-in for the
/// worker's `location.origin`, which the browser mint path reads
/// straight off its own scope.
///
/// # Errors
///
/// Returns an error if `endpoint` doesn't parse, or has no origin to
/// hang `/join` off (a `data:` or `mailto:` URL, say).
pub fn base_url_for_remote(endpoint: &str) -> Result<String, InviteError> {
    let parsed = Url::parse(endpoint).map_err(|e| {
        InviteError::Io(format!("remote endpoint '{endpoint}' is not a valid URL: {e}"))
    })?;
    parsed.join("/join").map(String::from).map_err(|e| {
        InviteError::Io(format!(
            "remote endpoint '{endpoint}' has no usable origin: {e}"
        ))
    })
}
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo test -p tonk-cli --lib when_deriving_a_base_url_from_a_remote 2>&1 | tail -20
```

Expected: PASS, 4 tests.

- [ ] **Step 6: Commit**

```bash
git add rust/tonk-cli/src/invite.rs
git commit -m "feat(cli): derive an invite base URL from a remote endpoint

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: `remote::resolve`

**Files:**
- Modify: `rust/tonk-cli/src/remote.rs` (add `AmbiguousRemote` to `RemoteError` around line 82, add `resolve` after `find` at line 295)
- Test: `rust/tonk-cli/tests/site.rs` (new module after `mod when_managing_remotes`, which closes at line 115)

**Interfaces:**
- Consumes: `list` and `find` (`remote.rs:242` and `remote.rs:288`), `RemoteRecord` (`remote.rs:40`, whose `endpoint` field is a `String`), `TonkSite`.
- Produces: `pub async fn resolve(site: &TonkSite, explicit: Option<&str>) -> Result<Option<RemoteRecord>, RemoteError>` and the new `RemoteError::AmbiguousRemote(String)` variant. Task 6 calls both.

- [ ] **Step 1: Write the failing tests**

In `rust/tonk-cli/tests/site.rs`, insert a new module directly after `mod when_managing_remotes { … }` closes at line 115:

```rust
mod when_resolving_a_remote {
    use anyhow::Result;
    use tonk_cli::remote::{self, RemoteError};

    use crate::common;

    const ENDPOINT: &str = "https://access.example.test/ucan/";
    const OTHER: &str = "https://other.example.test/ucan/";

    #[dialog_common::test]
    async fn it_resolves_nothing_when_no_remote_is_registered() -> Result<()> {
        let test = common::TestSite::new().await?;
        assert!(remote::resolve(&test.site, None).await?.is_none());
        Ok(())
    }

    #[dialog_common::test]
    async fn it_resolves_the_only_registered_remote() -> Result<()> {
        let test = common::TestSite::new().await?;
        remote::add(&test.site, "origin", ENDPOINT, None).await?;

        let resolved = remote::resolve(&test.site, None).await?;
        assert_eq!(resolved.expect("a lone remote resolves").endpoint, ENDPOINT);
        Ok(())
    }

    #[dialog_common::test]
    async fn it_resolves_the_named_remote_when_several_exist() -> Result<()> {
        let test = common::TestSite::new().await?;
        remote::add(&test.site, "origin", ENDPOINT, None).await?;
        remote::add(&test.site, "backup", OTHER, None).await?;

        let resolved = remote::resolve(&test.site, Some("backup")).await?;
        assert_eq!(resolved.expect("named remote resolves").endpoint, OTHER);
        Ok(())
    }

    #[dialog_common::test]
    async fn it_refuses_to_guess_between_several_remotes() -> Result<()> {
        let test = common::TestSite::new().await?;
        remote::add(&test.site, "origin", ENDPOINT, None).await?;
        remote::add(&test.site, "backup", OTHER, None).await?;

        match remote::resolve(&test.site, None).await {
            Err(RemoteError::AmbiguousRemote(names)) => {
                assert!(names.contains("origin"), "names both: {names}");
                assert!(names.contains("backup"), "names both: {names}");
            }
            other => panic!("expected AmbiguousRemote, got: {other:?}"),
        }
        Ok(())
    }

    #[dialog_common::test]
    async fn it_errors_on_a_name_that_is_not_registered() -> Result<()> {
        let test = common::TestSite::new().await?;
        remote::add(&test.site, "origin", ENDPOINT, None).await?;

        match remote::resolve(&test.site, Some("missing")).await {
            Err(RemoteError::UnknownRemote(name)) => assert_eq!(name, "missing"),
            other => panic!("expected UnknownRemote, got: {other:?}"),
        }
        Ok(())
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p tonk-cli --features integration-tests --test site when_resolving_a_remote 2>&1 | tail -20
```

Expected: FAIL to compile — `cannot find function 'resolve' in module 'remote'` and `no variant named 'AmbiguousRemote'`.

- [ ] **Step 3: Add the error variant**

In `rust/tonk-cli/src/remote.rs`, inside `enum RemoteError` (which starts at line 78), after the `UnknownRemote` variant that ends at line 82:

```rust
    /// Several remotes are registered and the caller named none, so
    /// there is no unambiguous choice to make on their behalf.
    #[error("several remotes are registered ({0}); name one with `--remote <NAME>`")]
    AmbiguousRemote(String),
```

`RemoteError::exit_code` (line 90) returns `ExitCode::IoError` for every variant, so it needs no change.

- [ ] **Step 4: Implement `resolve`**

In `rust/tonk-cli/src/remote.rs`, after `find` (which ends at line 295) and before the `// Helpers` banner at line 297:

```rust
/// Pick the remote a command should act on.
///
/// `explicit` names one outright. Otherwise this follows `tonk push`'s
/// implicit-when-unambiguous rule: a lone registered remote is the
/// obvious choice, no remotes at all means there is nothing to act on
/// (`None`, not an error — a local-only repo is a legitimate thing to
/// invite someone to), and several is a question only the caller can
/// answer.
pub async fn resolve(
    site: &TonkSite,
    explicit: Option<&str>,
) -> Result<Option<RemoteRecord>, RemoteError> {
    if let Some(name) = explicit {
        let record = find(site, name)
            .await?
            .ok_or_else(|| RemoteError::UnknownRemote(name.to_owned()))?;
        return Ok(Some(record));
    }

    let mut remotes = list(site).await?;
    match remotes.len() {
        0 => Ok(None),
        1 => Ok(Some(remotes.remove(0))),
        _ => Err(RemoteError::AmbiguousRemote(
            remotes
                .iter()
                .map(|record| record.name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        )),
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo test -p tonk-cli --features integration-tests --test site when_resolving_a_remote 2>&1 | tail -20
```

Expected: PASS, 5 tests.

- [ ] **Step 6: Commit**

```bash
git add rust/tonk-cli/src/remote.rs rust/tonk-cli/tests/site.rs
git commit -m "feat(cli): resolve a remote implicitly when unambiguous

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: Wire `tonk invite` to the resolved remote

**Files:**
- Modify: `rust/tonk-cli/src/bin/tonk.rs:226-243` (the `Invite` variant)
- Modify: `rust/tonk-cli/src/bin/tonk.rs:850-851` (the dispatch arm)
- Modify: `rust/tonk-cli/src/bin/tonk.rs` (`mint_invite`, at line 1537 before PR 1's deletions shift it upward — find it by name)

**Interfaces:**
- Consumes: `invite::base_url_for_remote` (Task 4), `remote::resolve` (Task 5), `invite::DEFAULT_BASE_URL` (re-exported at `invite.rs:37`), `invite::mint`, `invite::shorten`, the existing `print_error` and `open_selected` helpers in `bin/tonk.rs`.
- Produces: nothing further tasks consume. This is the behaviour change.

`bin/tonk.rs` has no test harness — it is verified end-to-end in Task 8 and by hand here.

- [ ] **Step 1: Make `--base-url` optional and add `--no-remote`**

Replace the `Invite` variant at `rust/tonk-cli/src/bin/tonk.rs:226-243` with:

```rust
    /// Mint an invite URL granting access to this repo
    ///
    /// Mints a UCAN delegation chain over the local repo. The
    /// default form is audience-open: anyone holding the URL can
    /// claim by redelegating from the embedded ephemeral key.
    ///
    /// The link is built on the remote's own origin, so the
    /// recipient lands on the deployment that actually serves the
    /// repo — and that origin's shortcut service can shorten it.
    #[command(
        after_help = "Examples:\n  tonk invite\n  tonk invite --remote prod\n  tonk invite --no-remote"
    )]
    Invite {
        /// Override the URL prefix the invite is built against.
        /// Defaults to `/join` on the resolved remote's origin, or
        /// to the canonical base when the repo has no remote.
        #[arg(long, value_name = "URL")]
        base_url: Option<String>,

        /// Embed a registered remote's URL in the invite so
        /// the claimer auto-configures the same access service
        /// after redeeming. Argument is the remote's local
        /// name (as registered with `tonk remote add`).
        /// Defaults to the only registered remote when there
        /// is exactly one.
        #[arg(long, value_name = "NAME", conflicts_with = "no_remote")]
        remote: Option<String>,

        /// Mint a local-only invite carrying no `remote=`, even
        /// when remotes are registered. The recipient joins with
        /// no upstream and wires one by hand.
        #[arg(long)]
        no_remote: bool,
    },
```

- [ ] **Step 2: Update the dispatch arm**

At `rust/tonk-cli/src/bin/tonk.rs:850-851`, replace:

```rust
        Command::Invite { base_url, remote } => {
            mint_invite(base_url, remote, spot.as_deref()).await
        }
```

with:

```rust
        Command::Invite {
            base_url,
            remote,
            no_remote,
        } => mint_invite(base_url, remote, no_remote, spot.as_deref()).await,
```

The telemetry arm at line 744 is `Command::Invite { .. } => ("invite", None)` and needs no change.

- [ ] **Step 3: Rewrite `mint_invite`**

Replace the whole of `async fn mint_invite` with:

```rust
async fn mint_invite(
    base_url: Option<String>,
    remote_name: Option<String>,
    no_remote: bool,
    spot: Option<&str>,
) -> ExitCode {
    let (_, site) = match open_selected(spot).await {
        Ok(opened) => opened,
        Err(code) => return code,
    };

    // Resolve the remote first: it decides both what gets embedded as
    // `remote=` and, unless `--base-url` overrides, which origin the
    // link points at. Those two have to stay in step — a link on one
    // deployment carrying a remote on another can't be shortened (the
    // shortcut service is same-origin) and drops the recipient on a
    // deployment that isn't serving the repo.
    let remote_record = if no_remote {
        None
    } else {
        match remote::resolve(&site, remote_name.as_deref()).await {
            Ok(resolved) => resolved,
            Err(err) => return print_error(err.to_string()),
        }
    };

    let base_url = match (base_url, &remote_record) {
        (Some(explicit), _) => explicit,
        (None, Some(record)) => match invite::base_url_for_remote(&record.endpoint) {
            Ok(derived) => derived,
            Err(err) => return print_error(err.to_string()),
        },
        (None, None) => invite::DEFAULT_BASE_URL.to_owned(),
    };

    let remote_url = remote_record.map(|record| record.endpoint);

    match invite::mint(&site, Some(&base_url), remote_url.as_deref()).await {
        Ok(mut outcome) => {
            // Shorten against the link's own origin; the long URL is
            // fully functional, so an unreachable shortcut service
            // (offline, dev base) degrades with a warning.
            match invite::shorten(&outcome.url).await {
                Ok(short) => outcome.url = short,
                Err(err) => eprintln!("warning: could not shorten the invite URL: {err}"),
            }
            print_invite_outcome(&outcome);
            ExitCode::Success
        }
        Err(err) => {
            eprintln!("error: {err}");
            err.exit_code()
        }
    }
}
```

- [ ] **Step 4: Build and check imports**

```bash
cd /Users/jackdouglas/tonk/tonk-invite
cargo build -p tonk-cli 2>&1 | tail -20
```

Expected: success. `remote` and `invite` are already imported in `bin/tonk.rs`; if `remote::find` was the only prior use and is now unused, the build stays clean because `remote` is imported as a module, not by item.

- [ ] **Step 5: Verify by hand against a scratch spot with no remote**

```bash
export TONK_SPOTS_STATE="$(mktemp -d)"
export TONK_SPOT=probe-noremote
cargo run -q -p tonk-cli --bin tonk -- spot new probe-noremote 2>&1 | tail -2
cargo run -q -p tonk-cli --bin tonk -- invite 2>&1 | tail -4
```

Expected: the link starts with `https://tonk.spot/join?access=` after Task 7 (before Task 7 it is still `https://hub.tonk.xyz/join?access=`), and stderr carries the shorten warning because neither host serves `PUT /@` yet.

- [ ] **Step 6: Verify by hand against a scratch spot with a staging remote**

```bash
export TONK_SPOTS_STATE="$(mktemp -d)"
export TONK_SPOT=probe-staging
cargo run -q -p tonk-cli --bin tonk -- spot new probe-staging 2>&1 | tail -2
cargo run -q -p tonk-cli --bin tonk -- remote add origin https://staging.tonk.xyz/ucan/ 2>&1 | tail -2
cargo run -q -p tonk-cli --bin tonk -- invite 2>&1 | tail -4
```

Expected: a short link of the form `https://staging.tonk.xyz/@/<base58>#<seed>`, and **no** shorten warning. This is the whole point of the change — if the warning is still there, stop and diagnose before continuing.

Note this makes a real `PUT` to the staging shortcut service, storing a delegation for a throwaway local repo with no data on it.

- [ ] **Step 7: Verify the ambiguity guard**

```bash
cargo run -q -p tonk-cli --bin tonk -- remote add backup https://hub.tonk.xyz/ucan/ 2>&1 | tail -2
cargo run -q -p tonk-cli --bin tonk -- invite 2>&1 | tail -3
cargo run -q -p tonk-cli --bin tonk -- invite --remote origin 2>&1 | tail -3
cargo run -q -p tonk-cli --bin tonk -- invite --no-remote 2>&1 | tail -3
```

Expected: the bare `invite` errors naming both remotes; `--remote origin` mints a short `staging.tonk.xyz` link; `--no-remote` mints against the fallback base with a shorten warning.

- [ ] **Step 8: Commit**

```bash
git add rust/tonk-cli/src/bin/tonk.rs
git commit -m "fix(cli): build invite links on the remote's origin

tonk invite mints against a hardcoded base independent of the remote
it embeds, so the link and the repo can land on different deployments
and shortening PUTs to an origin with no shortcut service. Resolve the
remote first and derive the base from its origin, matching what the
worker gets for free from its own scope.

Also resolves a lone remote implicitly, so a bare tonk invite carries
an upstream instead of stranding the joiner without one. --no-remote
opts back out.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 7: Retarget `DEFAULT_BASE_URL`

**Files:**
- Modify: `rust/tonk-invite/src/lib.rs:40-44`
- Modify: `rust/tonk-invite/README.md:35`

**Interfaces:**
- Consumes: nothing.
- Produces: `DEFAULT_BASE_URL` becomes `"https://tonk.spot/join"`. Task 6's `(None, None)` arm reads it. No test asserts its host — the `tonk-invite` unit tests at `lib.rs:488,512,531,681` use it opaquely, and the hardcoded `hub.tonk.xyz` strings at `lib.rs:464,472,506` and throughout `shortcut.rs` are literals unrelated to the constant.

- [ ] **Step 1: Change the constant and its doc**

In `rust/tonk-invite/src/lib.rs`, replace lines 40-44:

```rust
/// Canonical base URL for tonk invite links. Callers serializing an
/// [`Invite`] can pass this to [`Invite::to_url`] to mint a link rooted at
/// hub.tonk.xyz. Changing this value is a breaking change for any outstanding
/// invite URLs that embed it.
pub const DEFAULT_BASE_URL: &str = "https://hub.tonk.xyz/join";
```

with:

```rust
/// Canonical base URL for tonk invite links, and the fallback for a repo
/// with no remote to take an origin from. Callers serializing an [`Invite`]
/// pass this to [`Invite::to_url`] to mint a link rooted at tonk.spot.
///
/// Changing it does not invalidate outstanding invites: the base is not a
/// lookup key, so a link already minted against another host keeps
/// redeeming for as long as that host stays up.
pub const DEFAULT_BASE_URL: &str = "https://tonk.spot/join";
```

- [ ] **Step 2: Update the crate README**

`rust/tonk-invite/README.md:35` reads:

```
[`DEFAULT_BASE_URL`] (`https://hub.tonk.xyz/join`) is the canonical base for
```

Change `https://hub.tonk.xyz/join` to `https://tonk.spot/join`. Read the following lines and fix any prose that names hub.tonk.xyz as the deployment.

- [ ] **Step 3: Run the tonk-invite tests**

```bash
cargo test -p tonk-invite 2>&1 | tail -20
```

Expected: PASS. If a test asserts on `hub.tonk.xyz` *through* the constant, it is asserting the wrong thing — fix the test to derive its expectation from `DEFAULT_BASE_URL` rather than hardcoding a host.

- [ ] **Step 4: Commit**

```bash
git add rust/tonk-invite/src/lib.rs rust/tonk-invite/README.md
git commit -m "fix(invite): point the default base URL at tonk.spot

hub.tonk.xyz is the old name for the production deployment. The base
is only a URL prefix, not a lookup key, so links already minted
against it keep redeeming.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 8: End-to-end regression test

The regression this whole change exists to prevent: minting with no explicit `--base-url` must produce a link on the remote's origin, and that link must shorten.

**Files:**
- Modify: `rust/tonk-cli/tests/site.rs`, `mod when_shortening_an_invite` (lines 117-168)

**Interfaces:**
- Consumes: `invite::base_url_for_remote` (Task 4), `remote::resolve` (Task 5), `remote::add`, `invite::mint`, `invite::shorten`, `invite::claim`, `AccessServiceAddress` and `common::TestSite` (both already used by the existing test in this module).
- Produces: nothing.

The existing `it_shortens_and_claims_a_minted_invite` passes an explicit `base` and stays as-is — it covers the shortcut round trip. The new test covers the derivation.

- [ ] **Step 1: Write the failing test**

In `rust/tonk-cli/tests/site.rs`, inside `mod when_shortening_an_invite`, after `it_shortens_and_claims_a_minted_invite` closes at line 167:

```rust
    /// The regression: with no explicit base, the link must land on
    /// the remote's own origin — the only origin whose same-origin
    /// shortcut service can answer, and the deployment actually
    /// serving the repo.
    #[dialog_common::test]
    async fn it_derives_the_base_from_the_remote_and_shortens(
        env: AccessServiceAddress,
    ) -> Result<()> {
        let endpoint = env.access_service_url.as_str();
        let inviter = common::TestSite::new().await?;
        remote::add(&inviter.site, "origin", endpoint, None).await?;

        let resolved = remote::resolve(&inviter.site, None)
            .await?
            .expect("the lone remote resolves");
        let base = invite::base_url_for_remote(&resolved.endpoint)?;
        assert_eq!(base, format!("{endpoint}/join"));

        let outcome = invite::mint(&inviter.site, Some(&base), Some(&resolved.endpoint)).await?;
        let short = invite::shorten(&outcome.url).await?;
        assert!(
            short.starts_with(&format!("{endpoint}/@/")),
            "short link sits on the remote's origin: {short}"
        );
        Ok(())
    }
```

The `AccessServiceAddress` harness serves an origin with no path, so `base_url_for_remote` yields `{endpoint}/join` exactly. If the harness ever grows a path component, assert on the origin rather than the whole string.

- [ ] **Step 2: Run it to verify it fails on an unrebased tree**

```bash
cargo test -p tonk-cli --features integration-tests --test site it_derives_the_base_from_the_remote 2>&1 | tail -20
```

Expected: PASS, because Tasks 4 and 5 already landed. If it fails, the failure is real — diagnose before continuing. (This test is written after its implementation deliberately: it exercises the wiring end to end rather than driving a new unit into existence.)

- [ ] **Step 3: Run the whole site suite**

```bash
cargo test -p tonk-cli --features integration-tests --test site 2>&1 | tail -20
```

Expected: PASS.

- [ ] **Step 4: Run the full native suite**

```bash
nix develop -c test:native:debug 2>&1 | tail -30
```

Expected: PASS.

- [ ] **Step 5: Lint gate**

```bash
nix flake check 2>&1 | tail -30
```

Expected: clean. `--all-features` compiles the integration tests, so this is the run that catches an unused import behind the `integration-tests` feature.

- [ ] **Step 6: Commit and open PR 2**

````bash
git add rust/tonk-cli/tests/site.rs
git commit -m "test(cli): cover deriving an invite base from the remote

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
git push -u origin fix/cli-invite

# The body carries backticks and fenced shell output, so it goes
# through a quoted heredoc (no expansion) into a file. PR 1's number
# is read off its branch and spliced in afterwards.
PR1=$(gh pr list --head refactor/drop-cli-share --json number --jq '.[0].number')

cat > /tmp/pr2-body.md <<'EOF'
Stacked on #PR1_NUMBER — rebase onto staging once that merges.

`tonk invite` minted against a hardcoded base independent of the
remote it embedded, so the link and the repo could land on different
deployments:

```
$ tonk invite
warning: could not shorten the invite URL: shortcut PUT returned HTTP 404 Not Found
https://hub.tonk.xyz/join?access=...

$ tonk remote list
origin  https://staging.tonk.xyz/ucan/  did:key:z6Mkjd4...
```

Shortening PUTs to the link's own origin — the shortcut service is
same-origin by construction — so a prod link with a staging remote
can never be shortened, and drops the recipient on a deployment that
isn't serving the repo. The web UI never hits this because it reads
its own worker scope.

- `invite::base_url_for_remote` derives `/join` on the remote's origin
- `remote::resolve` picks the remote implicitly when unambiguous, so a
  bare `tonk invite` carries an upstream instead of stranding the
  joiner; `--no-remote` opts out
- `--base-url` becomes optional and overrides the derivation
- `DEFAULT_BASE_URL` moves to `https://tonk.spot/join` for the
  no-remote fallback

Prod won't actually shorten until `tonk.spot` is redeployed with the
current worker — it 404s on `PUT /@` today, so
`run_worker_first = ["/@", ...]` never took effect there. Staging works.

Spec: `docs/superpowers/specs/2026-07-22-cli-invite-remote-origin-design.md`

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF

sed -i '' "s/#PR1_NUMBER/#${PR1}/" /tmp/pr2-body.md
gh pr create --base refactor/drop-cli-share \
  --title "fix(cli): build invite links on the remote's origin" \
  --body-file /tmp/pr2-body.md
````

---

### Task 9: Make the invite surface honest about where the data goes

Task 6's review found that `invite::mint` pushes to the branch's **upstream**, not to the remote the caller resolved (`rust/tonk-cli/src/invite.rs:126-140`). Reproduced:

```
$ tonk invite --remote backup          # backup = http://127.0.0.1:9/other/
error: push before invite failed: ... url (http://127.0.0.1:9/ucan/)
```

The user asked for `backup`; the push went to `origin`, because `origin` is what `main` tracks. So the link is built on backup's origin and embeds backup's endpoint, while the repo state ships somewhere else — the recipient lands on a deployment that never received the data. That is this plan's own bug class, one leg over.

It is inherited: push-before-mint predates Task 6, and in the single-remote case the resolved remote *is* the upstream, so they coincide. But implicit resolution makes multi-remote `--remote` a more plausible path than it was.

The decision is to **warn, not re-route**. Pushing to the resolved remote would be correct by construction, but `sync::push` is upstream-bound and a push-to-named-remote path does not exist yet — too large for this branch. A warning that names both is honest and cannot make anything worse.

This task also cleans up the invite surface's prose, which Task 6 left stale.

**Files:**
- Modify: `rust/tonk-cli/src/bin/tonk.rs` (`mint_invite`, and the `--no-remote` help text on the `Invite` variant)
- Modify: `rust/tonk-cli/README.md:63-64`
- Modify: `rust/tonk-cli/SYNC.md:59`
- Modify: `.claude/commands/tonk.md:85`
- Test: `rust/tonk-cli/tests/site.rs`

**Interfaces:**
- Consumes: `remote::resolve` and `RemoteRecord` (Task 5), `remote::list`, `site.branch()` → `session.handle().upstream()`, the existing `mint_invite` in `bin/tonk.rs`.
- Produces: nothing later tasks consume. This is the last code task.

- [ ] **Step 1: Find out what the upstream can tell you**

Before writing anything, establish how to decide "the resolved remote is not the upstream". Read `rust/tonk-cli/src/sync.rs` (`push`, and how it reaches the upstream), `rust/tonk-cli/src/remote.rs` (`set_upstream`, `upstream_configured`, `list`), and `rust/tonk-schema/src/tracking_branch.rs`.

The question to answer: given a `TonkSite`, can you recover the *name* (or the endpoint) of the remote its `main` branch tracks? `session.handle().upstream()` returns the upstream branch handle — see what identifies it.

Three outcomes, in order of preference:

1. The upstream's remote name or endpoint is reachable. Compare it against the resolved `RemoteRecord` directly. Best — it is exact.
2. Only the upstream's subject DID or branch identity is reachable. Match it against `remote::list()` rows to recover the name. Still exact.
3. Nothing usable is reachable. Fall back to a coarser condition: warn whenever the user passed `--remote` explicitly *and* more than one remote is registered, since that is precisely when the two can diverge. Weaker, but never wrong about the risk.

Write down which one you took and why. Do not add a new public function to `remote.rs` unless option 1 or 2 needs it; if it does, give it a doc comment in the style of its neighbours.

- [ ] **Step 2: Write the failing test**

In `rust/tonk-cli/tests/site.rs`, add a module after `mod when_resolving_a_remote` (which closes around line 174 — read to confirm).

The test registers two remotes, sets the upstream to the first, and asserts that whatever you built in Step 1 reports a mismatch when asked about the second and no mismatch when asked about the first. Shape it to whatever you actually built — a helper returning `Option<String>`, a bool, whatever fits. Use `#[dialog_common::test]`, name the module `mod when_the_invite_remote_is_not_the_upstream`, and name the tests `it_does_x`.

The existing `mod when_managing_remotes` shows how to register a remote (`remote::add(&test.site, "origin", ENDPOINT, None)`) and set an upstream (`remote::set_upstream(&test.site, "origin")`) against `common::TestSite`.

If Step 1 landed on outcome 3, the condition is pure argument inspection with no site state, so test it as a unit test in `bin/tonk.rs`'s crate instead — say so and put it where it can actually run.

- [ ] **Step 3: Run it and watch it fail**

```bash
cd /Users/jackdouglas/tonk/tonk-invite
cargo test -p tonk-cli --features integration-tests --test site when_the_invite_remote_is_not_the_upstream -- --test-threads=1 2>&1 | tail -20
```

Expected: FAIL to compile, naming whatever you have not written yet.

- [ ] **Step 4: Implement the warning**

In `mint_invite` in `rust/tonk-cli/src/bin/tonk.rs`, after the remote is resolved and before `invite::mint` is called, emit a warning to stderr when the resolved remote is not the branch's upstream. Name both, and say what actually happens — the wording matters more than the mechanism:

```
warning: the invite embeds remote 'backup' but the repo pushes to 'origin';
         the recipient may join a deployment that has not received this data
```

Match the file's existing warning style (`eprintln!("warning: ...")` — see the shorten degradation a few lines below). Keep it a warning: minting must still succeed. A user with a deliberate split setup should not be blocked.

Do not warn when there is no upstream at all — `invite::mint` skips the push entirely in that case, so there is nothing to diverge from.

- [ ] **Step 5: Run the test and the suite**

```bash
cargo test -p tonk-cli --features integration-tests --test site when_the_invite_remote_is_not_the_upstream -- --test-threads=1 2>&1 | tail -20
cargo test -p tonk-cli --features integration-tests --test site -- --test-threads=1 2>&1 | tail -5
```

Expected: the new tests pass; the suite is at its prior count plus yours.

- [ ] **Step 6: Verify the warning by hand**

```bash
export TONK_SPOTS_STATE="$(mktemp -d)"
export TONK_SPOT=probe-mismatch
cargo run -q -p tonk-cli --bin tonk -- spot new probe-mismatch 2>&1 | tail -2
cargo run -q -p tonk-cli --bin tonk -- remote add origin http://127.0.0.1:9/ucan/ 2>&1 | tail -2
cargo run -q -p tonk-cli --bin tonk -- remote add backup http://127.0.0.1:9/other/ 2>&1 | tail -2
cargo run -q -p tonk-cli --bin tonk -- invite --remote backup 2>&1 | tail -6
```

Both remotes point at a dead port on purpose: the push fails either way, and what you are checking is that the mismatch warning appears *before* the push error. Confirm `--remote origin` produces no such warning.

- [ ] **Step 7: Fix the `--no-remote` help text**

Its current help says "Mint a local-only invite carrying no `remote=`". The *invite* is local-only; the mint still pulls and pushes to the upstream when one is configured. Reword so a reader does not take "local-only" to mean "no network".

- [ ] **Step 8: Update the invite docs**

Three files describe `tonk invite` as it behaved before Task 6, and none mention `--no-remote`:

- `rust/tonk-cli/README.md:63-64` — frames `--remote prod` as "also embeds a registered remote", when embedding is now the default and the flag is the disambiguator.
- `rust/tonk-cli/SYNC.md:59`
- `.claude/commands/tonk.md:85`

Read each in context. State the behaviour as it now is: a bare `tonk invite` resolves the repo's remote, builds the link on that remote's origin, and embeds it; `--remote <NAME>` picks one when several are registered; `--no-remote` mints without one. House style — lead with the answer, plain words, vary sentence length, no filler, match the surrounding voice.

- [ ] **Step 9: Lint gate**

```bash
cargo fmt --check 2>&1 | tail -5
cargo clippy --workspace --all-targets --all-features -- -D warnings 2>&1 | tail -10
```

Both must be clean. `nix flake check` cannot run on this machine (macOS 27 beta libffi breakage); these are the equivalent direct invocations, with args matching the flake's own check derivation.

- [ ] **Step 10: Commit**

```bash
git add rust/tonk-cli/ .claude/commands/tonk.md
git commit -m "fix(cli): warn when the invite remote is not the upstream

invite::mint pushes to the branch's upstream, not to the remote the
link embeds. With several remotes registered, --remote could build a
link on one deployment while the data shipped to another, leaving the
recipient on a deployment that never received it. Name both and say so.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Deploy follow-up (not code)

Neither PR makes production shorten. `https://tonk.spot/@` returns 404 because the deployed worker predates `run_worker_first = ["/@", "/@/*", "/ucan", "/ucan/*"]` in `wrangler.toml`. Redeploying the current worker to the production environment fixes it. Verify with:

```bash
curl -s -X PUT https://tonk.spot/@ --data-binary "/join?access=probe" -w " [%{http_code}]\n"
```

Expected after the deploy: a base58 hash and `[200]`, matching `staging.tonk.xyz` today.
