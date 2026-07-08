# Seed-Reaches-Remote Implementation Plan (CLI PR1)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `tonk invite --remote` pushes the local branch to its upstream before minting, so a joiner receives current repo state — including the stdlib seed `tonk init` committed before any upstream existed.

**Architecture:** Mirror the proven `share` path, which already does push-before-mint (`prepare_share` in `share.rs`: resolve remote → require upstream → pull-best-effort → push). Apply the same pull-then-push inside `invite::mint`, gated on an upstream being configured (a local-only invite with no upstream stays a no-op). Plus a one-line bench harness push and a cold-onboard re-baseline.

**Tech Stack:** Rust (`rust/tonk-cli`), clap CLI, the `sync`/`invite`/`share` modules; bash (`bench/bin/site.sh`); the bench harness.

**Spec:** `docs/superpowers/specs/2026-07-08-agent-ergonomic-cli-design.md` (§Adjacent fixes → "Seed reaches the remote"; §Sequencing PR1).

## Global Constraints

- VCS is jj (colocated). Commit with `jj commit <paths> -m "…"` — never `git add`/`git commit`, never touch bookmarks (the controller moves `feat/agent-build`). Conventional Commits, scope `cli` or `bench`.
- Repo test style: `#[dialog_common::test]`, test names `it_does_x`, grouped by behavior in `mod when_…` blocks (see `rust/tonk-cli/tests/sync.rs`). Shared helpers via `tests/common.rs`. No `mod.rs`.
- Lint gate: `nix develop -c cargo clippy --all -D warnings` (native, not wasm) must pass.
- No emojis anywhere.
- The push mirrors `prepare_share` exactly: best-effort `sync::pull` (warn on failure, continue), then hard `sync::push` (propagate the error). It runs only when the branch has an upstream.
- Do NOT change `eval`'s behavior or `invite::claim`; the change is confined to the mint path.

---

### Task 1: `tonk invite` pushes to the upstream before minting

**Files:**
- Modify: `rust/tonk-cli/src/invite.rs` (the `mint` fn, around line 106)
- Test: `rust/tonk-cli/tests/sync.rs` (new `mod when_minting_an_invite` block; reuse `wire_sibling_upstream`/`upstream_revision`)

**Interfaces:**
- Consumes: `sync::pull(&TonkSite)`, `sync::push(&TonkSite)` (both `pub async fn … -> Result<SyncOutcome, SyncError>`); `site.branch().await?.handle().upstream()` (returns `Option<_>`); the existing `mint(site, base_url, remote_url) -> Result<InviteOutcome, InviteError>`.
- Produces: `mint` performs push-before-mint when an upstream exists; signature unchanged.

- [ ] **Step 1: Write the failing test**

Add to `rust/tonk-cli/tests/sync.rs` (after the existing modules). Model the harness on the file's existing `wire_sibling_upstream` + `upstream_revision` helpers and the `it_auto_pushes_the_commit_to_the_upstream` test. `ATTRIBUTE_DECL` is the shared seed constant already imported.

```rust
mod when_minting_an_invite {
    use super::*;
    use tonk_cli::invite;

    #[dialog_common::test]
    async fn it_pushes_local_state_to_the_upstream_before_minting() -> Result<()> {
        let test = TestSite::new().await?;
        wire_sibling_upstream(&test).await?;
        // Commit something locally that has not been pushed, mirroring the
        // stdlib seed sitting unpushed on a freshly-init'd repo.
        test.eval_inline(ATTRIBUTE_DECL).await?;
        assert!(
            upstream_revision(&test).await?.is_none(),
            "upstream starts empty — the local commit has not been pushed yet"
        );

        // Minting a local-only invite (no embedded remote URL) must still
        // push, because the branch has an upstream.
        invite::mint(&test.site, None, None).await?;

        assert!(
            upstream_revision(&test).await?.is_some(),
            "mint must push the unpushed local state to the upstream"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_is_a_noop_push_when_no_upstream_is_configured() -> Result<()> {
        // No upstream wired: mint must still succeed (local-only invite),
        // not error trying to push.
        let test = TestSite::new().await?;
        let outcome = invite::mint(&test.site, None, None).await?;
        assert!(!outcome.url.is_empty(), "a local-only invite still mints a URL");
        Ok(())
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `nix develop -c cargo test -p tonk-cli --test sync when_minting_an_invite -- --nocapture`
Expected: `it_pushes_local_state_to_the_upstream_before_minting` FAILS on the final assert (`upstream_revision` still `None` — mint does not push today). The no-op test may already pass.

- [ ] **Step 3: Add the push-before-mint to `invite::mint`**

In `rust/tonk-cli/src/invite.rs`, at the top of `mint` (before any keypair generation / delegation work), add the guarded pull-then-push, mirroring `share::prepare_share`. Add `use crate::sync;` to the module imports if not present.

```rust
pub async fn mint(
    site: &TonkSite,
    base_url: Option<&str>,
    remote_url: Option<&str>,
) -> Result<InviteOutcome, InviteError> {
    // Push local state to the upstream before minting, so a joiner
    // receives current repo state — including the stdlib seed that
    // `tonk init` committed before any upstream existed. Mirrors
    // `share`'s push-before-mint. No-op when the branch has no upstream
    // (a local-only invite). Pull-before-push reconciles a possibly
    // advanced upstream, best-effort; the push error is authoritative.
    let has_upstream = {
        let session = site
            .branch()
            .await
            .map_err(|e| InviteError::Io(format!("acquire branch: {e}")))?;
        session.handle().upstream().is_some()
    };
    if has_upstream {
        if let Err(e) = sync::pull(site).await {
            eprintln!("warning: pull before invite failed: {e}");
        }
        sync::push(site)
            .await
            .map_err(|e| InviteError::Io(format!("push before invite failed: {e}")))?;
    }

    // … existing mint body unchanged …
}
```

Confirm `InviteError::Io(String)` exists (it is used elsewhere in this file — the module doc and error type already reference it). If the branch-session borrow conflicts with later `site` uses, the scoped block above drops the session before the mint body runs.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `nix develop -c cargo test -p tonk-cli --test sync when_minting_an_invite`
Expected: both tests PASS.

- [ ] **Step 5: Full crate tests + clippy**

Run: `nix develop -c cargo test -p tonk-cli` then `nix develop -c cargo clippy --all -D warnings`
Expected: all green, no warnings. (The `share` tests must still pass — the change is additive and confined to `invite::mint`.)

- [ ] **Step 6: Commit**

```bash
jj commit rust/tonk-cli/src/invite.rs rust/tonk-cli/tests/sync.rs -m "fix(cli): tonk invite pushes local state to upstream before minting"
```

---

### Task 2: Bench harness pushes after set-upstream

**Files:**
- Modify: `bench/bin/site.sh` (the `setup` fn, after `set-upstream`)

**Interfaces:**
- Consumes: the release `tonk` binary at `$TONK`.
- Produces: after `site.sh setup`, the origin's upstream carries the seed even before any invite is minted.

- [ ] **Step 1: Add the push**

In `bench/bin/site.sh`'s `setup()`, immediately after the `set-upstream` line and before `status`, add a push so the freshly-seeded origin publishes its stdlib to the remote (belt-and-braces alongside Task 1; also makes the origin's remote correct if anything inspects it before an invite is minted):

```bash
  "$TONK" remote set-upstream origin
  # Publish the init-seeded stdlib to the remote now, so a joiner (or an
  # inspector) sees current state even before an invite is minted. `tonk
  # init` commits the seed before the upstream exists, so nothing has
  # pushed it yet.
  "$TONK" push
  "$TONK" status
```

- [ ] **Step 2: Verify the origin is no longer ahead after setup**

Run a scripted cold-onboard setup and check status. From the repo root:

```bash
nix develop -c bash -c '
  export ROOT="$PWD" RUN_DIR="$PWD/bench/runs/dev-seedpush" BENCH_PORT=8796 BENCH_URL=http://127.0.0.1:8796
  mkdir -p "$RUN_DIR"
  bench/bin/stack.sh start
  bench/bin/site.sh setup
  ( cd "$RUN_DIR/site" && "$ROOT/target/release/tonk" status )
  bench/bin/stack.sh stop
'
rm -rf "$PWD/bench/runs/dev-seedpush"
```

Expected: the final `tonk status` prints `synced` (not `ahead`). If `push` fails (e.g. the access service rejects), capture the error — it must be resolved, since it is the same push Task 1 relies on.

- [ ] **Step 3: Commit**

```bash
jj commit bench/bin/site.sh -m "fix(bench): push seed to the remote after set-upstream so joiners aren't barren"
```

---

### Task 3: Re-baseline cold-onboard (verification, not code)

**Files:**
- Modify: `bench/README.md` (baseline table) — record the clean number.

**Interfaces:**
- Consumes: Tasks 1–2 (invite now pushes; origin publishes the seed).
- Produces: a cold-onboard baseline no longer confounded by the barren join, and durable evidence the fix worked (the joined branch carries `tonk:view`).

- [ ] **Step 1: Confirm codex auth, then run a real cold-onboard episode**

The codex OAuth session must be valid (it was re-authed this session; if a run fails 3s with a `token_revoked` error, ask the user to run `codex login` and retry — do not record an auth-failed run as a baseline).

Run: `nix develop -c bench/bin/bench run cold-onboard`
Expected: completes; `bench/runs/<ts>-cold-onboard/scores.json` has a real `judge.outcome`.

- [ ] **Step 2: Verify the joined branch now carries the stdlib**

```bash
RD="$(ls -dt bench/runs/*-cold-onboard | head -1)"
( cd "$RD/agent" && env TONK_NO_SYNC=1 /Users/jackdouglas/tonk/tonk/target/release/tonk schema 2>/dev/null | grep -c "xyz.tonk.view" )
```

Expected: a non-zero count (the joined agent branch now has the `tonk:view` attributes — the barren-join artifact is gone). Contrast with the pre-fix run `bench/runs/20260708-093235-1-cold-onboard/agent`, which had none.

- [ ] **Step 3: Record the clean baseline**

In `bench/README.md`, update the codex/gpt-5.5 baseline row for cold-onboard with the new outcome and a note that the barren-join confound is fixed (`tonk invite` now pushes the seed). Keep the prior confounded number in a parenthetical for the before/after story.

- [ ] **Step 4: Commit**

```bash
jj commit bench/README.md -m "docs(bench): clean cold-onboard baseline after seed-reaches-remote fix"
```
