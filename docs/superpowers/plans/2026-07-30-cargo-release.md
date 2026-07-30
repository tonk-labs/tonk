# cargo-release Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the version bump and the `v*` tag one causal chain rather than two acts a human can forget to connect, and point npm's `stable` dist-tag at the release commit `stable` holds.

**Architecture:** A root `release.toml` overrides seven cargo-release defaults that would misfire on a 34-crate workspace behind branch rulesets. `cargo-release` joins the flake devshell — the devshell only, not the shared build inputs. A new workflow turns a merged version bump into a `v<version>` tag and starts the publish. A second new workflow re-points the npm `stable` dist-tag on pushes to `stable` — a metadata move, never a publish.

**Tech Stack:** cargo-release 1.1.2 (nixpkgs), GitHub Actions, npm CLI, nix flakes.

Spec: `docs/superpowers/specs/2026-07-30-cargo-release-design.md`.

## Global Constraints

- **Nothing can push a commit to `staging`.** Two active rulesets target the default branch: `pull_request` + `non_fast_forward` with no bypass actor, and `deletion` + `non_fast_forward` + `required_signatures` + `required_status_checks`. `allow_merge_commit` is false, so a PR merge rewrites the SHA. A local `cargo release --execute` cannot push, and a locally created tag would point at a commit that never lands. The rulesets cover branches, not tags, so CI can push a tag.
- Tag name must be exactly `v{{version}}` — `cli-npm.yml` triggers on the `v*` glob and would never fire on cargo-release's default `tonk-cli-v0.6.4`.
- Commit message must be exactly `chore: release {{version}}` — matches the existing convention (lowercase "release", no crate name).
- `publish = false` is mandatory. The default `true` targets all 34 crates and none are on crates.io.
- `push = false` and `tag = false`. CI owns the tag; the push is a PR.
- `allow-branch = ["staging", "release/*"]`. Version bumps happen on the way to `staging`; `stable` is only ever fast-forwarded.
- The tag predicate is a version **change**, never the absence of a tag.
- Do not modify `.github/workflows/cli-npm.yml`. It is the publish contract; the new workflows feed it.
- No changelog generation. No crates.io publishing. No new platform targets.
- No emojis in code, commits, or output.

**Note on verification style:** this change is configuration and CI, not executable logic — the only Rust in it is one string. Verification is therefore dry-runs, `actionlint`, a local simulation of the tag predicate over real commits, and registry reads against real state. Do not manufacture unit tests for a TOML file; the checks below are the real ones.

**Note on verification ordering:** cargo-release's dry run refuses to run against a dirty tree ("uncommitted changes detected"). Every dry-run step therefore comes *after* the commit step in its task, not before. Ordering them the other way produces a spurious error that looks like a config problem.

---

### Task 1: release.toml and the devshell

**Files:**
- Create: `release.toml` (repo root)
- Modify: `flake.nix` — `devShellBuildInputs`, not `commonBuildInputs`

**Interfaces:**
- Consumes: nothing.
- Produces: the `cargo release <level> --execute` command, available in `nix develop`, emitting exactly one commit `chore: release <version>` on the current branch and nothing else.

- [ ] **Step 1: Create `release.toml`**

Settings and their justifications are the table in the spec's `release.toml` section. The file carries them as comments, plus a header explaining that a release is two acts because the ruleset makes it two acts.

- [ ] **Step 2: Add cargo-release to the devshell**

In `flake.nix`, add `pkgs.cargo-release` to `devShellBuildInputs`.

**Not `commonBuildInputs`.** `flake.nix` passes `commonBuildInputs` to `nix/rust.nix` as `buildInputs`, which land as `nativeBuildInputs` on `buildDepsOnly` and all 34 crate derivations. A release tool there changes every crate hash, forces a cold cachix rebuild on both runners, and puts cargo-release in every build sandbox. `devShellBuildInputs` is `commonBuildInputs ++ [...]`, so `nix develop` still resolves it.

`cargo-nextest` is already in `commonBuildInputs`. That is pre-existing; leave it.

- [ ] **Step 3: Commit**

Commit before verifying. The dry runs in Steps 4-7 need a clean tree: cargo-release checks for uncommitted changes *before* it reaches the branch guard, so on a dirty tree it aborts with `uncommitted changes detected` and never prints the release plan you are trying to inspect.

```bash
git add release.toml flake.nix
git commit -m "build(release): let CI cut the release tag

cargo-release cannot push to staging. Two active rulesets cover the
default branch and require a pull request with no bypass actor, and
merge commits are disabled, so a PR merge rewrites the SHA and a
locally created tag could never point at a commit that lands. The bump
stays local; CI owns the tag.

The remaining settings each override a default that misfires on a
34-crate shared-version workspace: without them a release is 34
commits, 34 tags named tonk-cli-v*, and 34 attempted crates.io
publishes."
```

Task 2 adds the tag workflow to *this* commit rather than a second one — see Task 2, Step 4.

- [ ] **Step 4: Verify the tool resolves in the devshell**

Run: `nix develop --command cargo release --version`

Note the positional: cargo plugins are invoked as `cargo release`, so `cargo-release --version` is not the form to document even though the binary answers to it.

Expected: `cargo-release 1.1.2` or later.

- [ ] **Step 5: Verify the config is parsed as intended**

Run: `nix develop --command cargo release config`

Expected to include:

```
allow-branch = ["staging", "release/*"]
publish = false
push = false
tag = false
consolidate-commits = true
pre-release-commit-message = "chore: release {{version}}"
tag-name = "v{{version}}"
tag-message = "chore: release {{version}}"
```

- [ ] **Step 6: Verify the branch guard rejects an unrelated branch**

From a branch matching neither `staging` nor `release/*`:

`nix develop --command cargo release patch --workspace --no-confirm`

Expected: `error: cannot release from branch '<current>' as it doesn't match ...`.

Note: dry-run collects errors and keeps printing the plan, so lines still appear below the error. The error line is the assertion.

- [ ] **Step 7: Verify the release plan itself**

Run: `nix develop --command cargo release patch --workspace --no-confirm --allow-branch '*'`

Expected, against a workspace at 0.6.3:
- `Upgrading <crate> from 0.6.3 to 0.6.4` for every crate
- **no** `Tagging` line and **no** `Pushing` line
- no `error:` lines

Do **not** pass `--execute`. This must not cut a real release.

---

### Task 2: The release tag workflow

**Files:**
- Create: `.github/workflows/release-tag.yml`

**Interfaces:**
- Consumes: pushes to `staging`, and the `[workspace.package] version` at two commits.
- Produces: the annotated tag `v<version>`, and a `cli-npm.yml` run against it.

- [ ] **Step 1: Create the workflow**

On push to `staging`, one job with `contents: write` and `actions: write`:

1. `actions/checkout@v4` with `fetch-depth: 0` and `fetch-tags: true` — `github.event.before` must be reachable and the tag list must be present.
2. Extract the workspace version at `github.event.before` and at `github.sha` using the same `grep`/`sed` pipeline as `cli-npm.yml`.
3. Emit a `version` output only when all of these hold; otherwise print why and exit 0:
   - `github.event.before` is non-empty and not all zeroes (branch creation);
   - `github.event.before` is a reachable commit (force push);
   - the version at `github.sha` is non-empty (this one is `exit 1` — an unreadable version at HEAD would silently disable releases forever);
   - the version at `before` is non-empty;
   - the two versions differ;
   - `refs/tags/v<version>` does not already exist on the remote.
4. Create the annotated tag at `github.sha` and push it.
5. `gh workflow run cli-npm.yml --ref "v<version>"`.

Step 5 is not optional. **A tag pushed with `GITHUB_TOKEN` does not trigger any workflow** — GitHub suppresses events raised by the built-in token to stop workflows recursing, with `workflow_dispatch` and `repository_dispatch` as the only exceptions. Without the dispatch this workflow would create tags and publish nothing. `cli-npm.yml` resolves the version from the ref's own `Cargo.toml` and refuses a mismatch, so dispatching it against the tag gives the same contract a tag push would have.

`concurrency: release-tag` with `cancel-in-progress: false`, so two quick merges cannot interleave.

- [ ] **Step 2: Lint the workflow**

Run: `nix run nixpkgs#actionlint -- .github/workflows/release-tag.yml`
Expected: exit 0, no output.

- [ ] **Step 3: Prove the predicate over real commits**

The whole design rests on this predicate, so simulate it against staging history rather than reasoning about it. Pick two commit pairs: one across a real version bump, one across an ordinary merge. Run the same extraction the workflow runs, and show the predicate is true then false.

Expected: `changed` for the bump pair, `unchanged` for the other. An empty version on either side means the pipeline is broken.

- [ ] **Step 4: Amend Task 1's commit**

This belongs with Task 1: `push = false`/`tag = false` and this workflow are two halves of one decision, and either alone is broken — the config without the workflow never tags, the workflow without the config double-tags. Task 1 already committed the config so its dry runs had a clean tree to plan against, so add the workflow to that commit instead of creating a second one.

```bash
git add .github/workflows/release-tag.yml
git commit --amend --no-edit
```

---

### Task 3: The stable promote workflow

**Files:**
- Create: `.github/workflows/cli-npm-promote.yml`

**Interfaces:**
- Consumes: the `v<version>` tags from Task 2, and the `@tonk/cli` versions `cli-npm.yml` publishes from them.
- Produces: the npm `stable` dist-tag, pointing at the release commit `stable` holds.

- [ ] **Step 1: Create the workflow**

On push to `stable` and on `workflow_dispatch`, with `fetch-depth: 0` and `fetch-tags: true`, in order:

1. Refuse unless `$GITHUB_REF` is `refs/heads/stable`. `workflow_dispatch` takes any ref, and dispatching against `staging` would point `@stable` at a version `stable` does not hold.
2. Read the workspace version; refuse a prerelease.
3. Require `refs/tags/v<version>^{commit}` to equal `$GITHUB_SHA`, with an error that names the fast-forward command. Without this, promoting to a staging commit *past* the release commit — which is the natural milestone target — makes `@stable` serve a tarball that does not contain the code `stable` holds, while `cli.yml` builds `tonk-latest` from stable's real HEAD.
4. Require `@tonk/cli@<version>` to exist on the registry. Capture npm's output and branch on `code E404`, so a registry outage is not reported as an unpublished release.
5. `npm dist-tag add @tonk/cli@<version> stable`, then `npm dist-tag ls`.

A separate file rather than a job inside `cli-npm.yml`: adding a branch trigger there would require `if:` guards on all three existing jobs, and a single missed guard means publishing on every stable push. The cost is that npm-registry mutations now live in two files.

`node-version: 22` deviates from `cli-npm.yml`'s `20`, which the runners now flag as deprecated on every run. Bumping that file is a separate change; do not fold it in here.

- [ ] **Step 2: Lint the workflow**

Run: `nix run nixpkgs#actionlint -- .github/workflows/cli-npm-promote.yml`
Expected: exit 0, no output.

- [ ] **Step 3: Verify the version extraction against real branches**

```bash
for ref in origin/staging origin/stable; do
  printf '%s: ' "$ref"
  git show "$ref:Cargo.toml" | grep -A30 '^\[workspace.package\]' \
    | grep -m1 '^version' | sed -E 's/.*"([^"]+)".*/\1/'
done
```

Expected: two distinct parseable versions. An empty result means the pipeline is broken.

- [ ] **Step 4: Verify the guards against live state**

Confirm the prerelease guard fires on a hyphenated version; that `npm view @tonk/cli@<stable's version>` reports `code E404`; and that a published version resolves. Confirm the tag guard would fire today, since no `v0.6.0` tag exists for the version `stable` holds.

Expected: promoting `stable` **today** correctly fails, twice over — no `v0.6.0` tag and 0.6.0 unpublished. That is by design and self-heals after one full cycle. Do not add a workaround.

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/cli-npm-promote.yml
git commit -m "ci(cli-npm): require stable to sit on the release commit before promoting"
```

---

### Task 4: Documentation

**Files:**
- Modify: `rust/tonk-cli/npm/README.md` (the "Publishing (maintainers)" section and the closing paragraph)
- Modify: `README.md` (the install line)
- Modify: `rust/tonk-cli/src/update/swap.rs` (the npm remedy string)

**Interfaces:**
- Consumes: the commands from Tasks 1-3.
- Produces: nothing consumed by later tasks.

- [ ] **Step 1: Rewrite the publishing section**

Four numbered steps: the one-time `NPM_TOKEN` note; cut the bump on a `release/*` branch with `cargo release <level> --execute`; PR it and let `release-tag.yml` tag the merge; promote by fast-forwarding `stable` to the release commit.

Must state, because each is a trap a maintainer would otherwise hit:

- which level cuts a final (`patch`/`minor`/`major`), and that `release` only strips an existing prerelease — from a final it writes no commit and still exits 0;
- that `release.toml` disables the push and the tag, so `--execute` produces only a commit;
- that promoting must target the release commit, with the exact `git push origin v<version>:refs/heads/stable`;
- that the `CLI npm` run on the tag must finish before promoting, that the resulting `is not published; nothing to promote` means "not yet", and that `gh workflow run cli-npm-promote.yml --ref stable` re-runs a promote.

Also fix the local `npm pack` recipe: it installs `./tonk-cli-0.5.0.tgz` while `cli/package.json` says `0.6.0`, so it fails as written. Glob it (`./tonk-cli-[0-9]*.tgz`) — the committed versions are placeholders CI stamps over, so any literal filename rots.

Replace the closing "Bump both together" paragraph. It survives from the manual era and now contradicts the section above it: CI stamps the `package.json`s and cargo-release owns the workspace bump.

- [ ] **Step 2: Update the root README install line**

Drop `@latest` — it is the default, and naming it invites confusion with `@stable`:

```
If `tonk` was installed some other way, `tonk update` says so instead
of interfering: use `npm i -g @tonk/cli` for an npm install (or
`@tonk/cli@stable` to pin to the last milestone), or your flake for a
nix one.
```

The clause is mid-sentence; the `or your flake for a nix one` tail must still parse.

- [ ] **Step 3: Fix the npm remedy string**

`ForeignInstall::Npm` in `rust/tonk-cli/src/update/swap.rs` says `run \`npm i -g @tonk/cli@latest\``. Under this model `@latest` means staging finals, so that moves a `@stable`-pinned user onto a faster channel. Mirror the root README instead: no explicit tag, with `@stable` offered beside it.

Run `cargo build -p tonk-cli` so the edit is proven to compile.

- [ ] **Step 4: Verify no stale instructions remain**

```bash
grep -rn "git tag v\|push origin v[0-9]\|--no-push\|atomic" rust/tonk-cli/npm/README.md README.md release.toml
```

Expected: only the deliberate `git push origin v0.6.4:refs/heads/stable` promote line. Anything else is a leftover from the manual or the atomic-push model.

- [ ] **Step 5: Commit**

```bash
git add rust/tonk-cli/src/update/swap.rs
git commit -m "fix(cli): stop the npm update remedy pinning @latest"
git add rust/tonk-cli/npm/README.md README.md
git commit -m "docs(cli): document the two-act release"
```

---

## Self-Review

**Spec coverage:**

| Spec section | Task |
| --- | --- |
| Constraint: nothing can push to `staging` | 1 (config), 2 (workflow) |
| `release.toml` (seven overrides) | 1 |
| Devshell addition, devshell-only | 1 |
| Why CI owns the tag | 2 |
| Stable promote workflow + promote-target invariant | 3 |
| Ordering dependency | 4 |
| Docs — npm README, root README, remedy string | 4 |
| Out of scope: changelog, crates.io, platforms | absent by construction |
| Blocked: npm token | called out in 4, Step 1 |

**Placeholders:** none.

**Type consistency:** the version-extraction pipeline is byte-identical across `cli-npm.yml`, `release-tag.yml`, and `cli-npm-promote.yml`. The tag `release-tag.yml` creates matches `release.toml`'s `tag-name` and `cli-npm.yml`'s `v*` glob.

**Known gap, deliberate:** nothing here can be end-to-end verified until the npm token is fixed, since every path terminates in a registry write. And `release-tag.yml`'s trigger cannot be exercised without a real merge to `staging`; Task 2 Step 3 simulates the predicate, which is the part that decides whether a release happens.
