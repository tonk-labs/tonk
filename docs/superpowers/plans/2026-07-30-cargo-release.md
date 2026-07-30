# cargo-release Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the version bump and the `v*` tag a single act, and point npm's `stable` dist-tag at whatever `stable` holds.

**Architecture:** A root `release.toml` overrides six cargo-release defaults that would misfire on a 34-crate workspace. `cargo-release` joins the flake devshell. A new workflow re-points the npm `stable` dist-tag on pushes to `stable` — a metadata move, never a publish.

**Tech Stack:** cargo-release 0.25.x (nixpkgs), GitHub Actions, npm CLI, nix flakes.

Spec: `docs/superpowers/specs/2026-07-30-cargo-release-design.md`.

## Global Constraints

- Tag name must be exactly `v{{version}}` — `cli-npm.yml` triggers on the `v*` glob and would never fire on cargo-release's default `tonk-cli-v0.6.4`.
- Commit message must be exactly `chore: release {{version}}` — matches the existing convention (lowercase "release", no crate name).
- `publish = false` is mandatory. The default `true` targets all 34 crates and none are on crates.io.
- `allow-branch = ["staging"]`. Version bumps happen only on `staging`; `stable` is only ever fast-forwarded.
- No changelog generation. No crates.io publishing. No new platform targets.
- No emojis in code, commits, or output.

**Note on verification style:** this change is configuration and CI, not executable logic — there is no unit-testable function anywhere in it. Verification is therefore dry-runs, `actionlint`, and registry reads against real state. Do not manufacture unit tests for a TOML file; the checks below are the real ones.

---

### Task 1: release.toml and the devshell

**Files:**
- Create: `release.toml` (repo root)
- Modify: `flake.nix:56` (insert into the alphabetized `commonBuildInputs` list, immediately after `cargo-nextest`)

**Interfaces:**
- Consumes: nothing.
- Produces: the `cargo release <level>` command, available in `nix develop`, emitting one commit `chore: release <version>` and one annotated tag `v<version>`.

- [ ] **Step 1: Create `release.toml`**

```toml
# Releases are cut with `cargo release <level>` from `staging` only. Each
# run produces one commit and one `v<version>` tag, pushed together, so
# the bump and the tag cannot drift apart the way 0.6.1 through 0.6.3 did.
#
# Every setting here overrides a default that misfires on this workspace.
# See docs/superpowers/specs/2026-07-30-cargo-release-design.md.

# The version is shared via `version.workspace`, so a release is one
# commit for all 34 crates, not one commit each.
consolidate-commits = true

# `cli-npm.yml` triggers on the `v*` glob. The default of
# `{{prefix}}v{{version}}` yields `tonk-cli-v0.6.4`, which never matches.
tag-name = "v{{version}}"

pre-release-commit-message = "chore: release {{version}}"
tag-message = "chore: release {{version}}"

# None of these crates are on crates.io. Without this, a release attempts
# `cargo publish` on all 34. Note that cargo-release still logs a
# cosmetic `Publishing <crates>` line under this setting -- it is not
# evidence of a registry attempt.
publish = false

# The guardrail against the original failure: a release cut from a
# feature branch. Bumps belong on `staging`; `stable` only fast-forwards.
allow-branch = ["staging"]
```

- [ ] **Step 2: Add cargo-release to the devshell**

In `flake.nix`, inside `commonBuildInputs`, add `cargo-release` directly after `cargo-nextest` to keep the list alphabetized:

```nix
            cargo-nextest
            cargo-release
```

- [ ] **Step 3: Verify the tool resolves in the devshell**

Run: `nix develop --command cargo-release --version`
Expected: a version string, `0.25.x` or later. If nix cannot evaluate, the flake edit is malformed.

- [ ] **Step 4: Verify the config is parsed as intended**

Run: `nix develop --command cargo release config | grep -E '^(publish|consolidate-commits|tag-name|pre-release-commit-message|allow-branch)'`
Expected exactly:

```
allow-branch = ["staging"]
publish = false
consolidate-commits = true
pre-release-commit-message = "chore: release {{version}}"
tag-name = "v{{version}}"
```

- [ ] **Step 5: Verify the branch guard rejects this branch**

You are not on `staging`, so the guard must fire. Run:

`nix develop --command cargo release rc --workspace --no-confirm`

Expected: `error: cannot release from branch '<current>' as it doesn't match 'staging'`.

Note: dry-run collects errors and keeps printing the plan, so a `Pushing ...` line still appears below the error. That is expected — only `--execute` aborts at the check. The error line is the assertion.

- [ ] **Step 6: Verify the release plan itself, overriding the guard**

Run: `nix develop --command cargo release rc --workspace --no-confirm --allow-branch '*'`

Expected, against a workspace at 0.6.3:
- `Upgrading workspace to version 0.6.4-rc.1`
- exactly one tag in the `Pushing` line: `v0.6.4-rc.1`
- no `error:` lines

Do **not** pass `--execute`. This must not cut a real release.

- [ ] **Step 7: Commit**

```bash
git add release.toml flake.nix
git commit -m "build: cut releases with cargo-release

The workspace version and the v* tag were produced by separate acts, so
they drifted -- 0.6.1 through 0.6.3 landed inside feature PRs with no
tag. cargo-release makes the bump, the tag, and the push one command.

Every setting in release.toml overrides a default that misfires on a
34-crate workspace with a shared version: without them a release is 34
commits, 34 tags named tonk-cli-v*, and 34 attempted crates.io
publishes."
```

---

### Task 2: The stable promote workflow

**Files:**
- Create: `.github/workflows/cli-npm-promote.yml`

**Interfaces:**
- Consumes: the `v<version>` tags from Task 1, and the `@tonk/cli` versions that `cli-npm.yml` publishes from them.
- Produces: the npm `stable` dist-tag, pointing at the version `stable` holds.

- [ ] **Step 1: Create the workflow**

```yaml
name: 'CLI npm promote'

# Promoting `stable` re-points the npm `stable` dist-tag at the version
# stable now holds. It never publishes: because promotion is a
# fast-forward, that version was already published from `staging` when
# its `v*` tag fired `cli-npm.yml`. A dist-tag move is metadata only.
#
# This lives apart from `cli-npm.yml` deliberately. Adding a branch
# trigger there would need `if:` guards on all three of its jobs, and one
# missed guard means publishing on every push to stable.
on:
  push:
    branches:
      - stable
  workflow_dispatch:

concurrency:
  group: cli-npm-promote
  cancel-in-progress: false

jobs:
  promote:
    name: 'Point @stable at the promoted version'
    runs-on: ubuntu-latest
    permissions:
      contents: read
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with:
          node-version: 22
          registry-url: 'https://registry.npmjs.org'
      - name: Promote
        env:
          NODE_AUTH_TOKEN: ${{ secrets.NPM_TOKEN }}
        run: |
          set -euo pipefail
          version="$(grep -A30 '^\[workspace.package\]' Cargo.toml \
            | grep -m1 '^version' \
            | sed -E 's/.*"([^"]+)".*/\1/')"
          echo "stable holds $version"
          # `stable` is only ever fast-forwarded to a final release. A
          # prerelease here means the wrong commit was promoted.
          if [[ "$version" == *-* ]]; then
            echo "::error::stable holds prerelease $version; promote a final release instead"
            exit 1
          fi
          # The version must already be on the registry. If it is not,
          # stable holds a release that never published -- worth failing
          # on rather than silently pointing the tag at nothing.
          if ! npm view "@tonk/cli@$version" version >/dev/null 2>&1; then
            echo "::error::@tonk/cli@$version is not published; nothing to promote"
            exit 1
          fi
          # Only the wrapper needs the tag: its optionalDependencies pin
          # exact platform versions, so `@tonk/cli@stable` resolves the
          # right platform package on its own.
          npm dist-tag add "@tonk/cli@$version" stable
          npm dist-tag ls @tonk/cli
```

`node-version: 22` deviates from `cli-npm.yml`'s `20`, which the runners now flag as deprecated on every run. Bumping that file is a separate change; do not fold it in here.

- [ ] **Step 2: Lint the workflow**

Run: `nix run nixpkgs#actionlint -- .github/workflows/cli-npm-promote.yml`
Expected: exit 0, no output.

- [ ] **Step 3: Verify the version extraction against real branches**

The `grep`/`sed` pipeline is copied from `cli-npm.yml`, but confirm it against both branches:

```bash
for ref in origin/staging origin/stable; do
  printf '%s: ' "$ref"
  git show "$ref:Cargo.toml" | grep -A30 '^\[workspace.package\]' \
    | grep -m1 '^version' | sed -E 's/.*"([^"]+)".*/\1/'
done
```

Expected: `origin/staging: 0.6.3` and `origin/stable: 0.6.0` (or later values, but two distinct parseable versions — an empty result means the pipeline is broken).

- [ ] **Step 4: Verify both guards fire correctly against live registry state**

Simulate the two failure paths and the success path locally:

```bash
# Prerelease guard
v=0.6.4-rc.1; [[ "$v" == *-* ]] && echo "guard fires: prerelease rejected"
# Unpublished guard: stable is at 0.6.0, which was never published
npm view "@tonk/cli@0.6.0" version >/dev/null 2>&1 \
  && echo "0.6.0 published" || echo "guard fires: 0.6.0 unpublished, promote would fail"
# Success path: 0.4.0 is published
npm view "@tonk/cli@0.4.0" version >/dev/null 2>&1 \
  && echo "success path: 0.4.0 resolvable, dist-tag move would proceed"
```

Expected: all three lines print. The middle one confirms a real and useful property — promoting `stable` **today** would correctly fail, because stable holds 0.6.0 and 0.6.0 was never published.

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/cli-npm-promote.yml
git commit -m "ci(cli-npm): point the npm stable dist-tag at promoted versions

stable moves on milestones, so it can sit well behind staging. Mapping
npm's latest onto stable would mean the default install serves
increasingly old code -- which is the situation today, with latest at
0.4.0 against a 0.6.3 workspace.

So latest tracks staging finals and stable gets its own dist-tag. Since
promotion is a fast-forward, the version is already published and this
only re-points a tag; it never publishes. Both guards fail loudly rather
than pointing the tag at nothing."
```

---

### Task 3: Documentation

**Files:**
- Modify: `rust/tonk-cli/npm/README.md:25-60` (the "Publishing (maintainers)" section runs from line 25 to the end of the 60-line file; keep the local `npm pack` recipe near the end intact)
- Modify: `README.md:93-95` (the install line)

**Interfaces:**
- Consumes: the commands from Tasks 1 and 2.
- Produces: nothing consumed by later tasks.

- [ ] **Step 1: Rewrite the publishing section**

Replace the numbered steps and the paragraph beginning "The tag is the source of the version" in `rust/tonk-cli/npm/README.md` with:

```markdown
Publishing runs in CI so every platform binary is built reproducibly —
you cannot build the Linux binary from a Mac. It is **tag-driven**;
nothing publishes on a normal push.

1. One-time: add an `NPM_TOKEN` repository secret — an npm automation
   token with read-write on the whole **`@tonk` scope**, not a package
   subset, so platform packages added later keep working. A token
   scoped too narrowly still passes `npm whoami` and then fails with a
   bare `404 Not Found - PUT`, which reads like a missing package.
2. From `staging`, cut the release. This is one command; it bumps the
   workspace version, commits, tags, and pushes together:

   ```sh
   cargo release rc        # 0.6.3 -> 0.6.4-rc.1, publishes to @next
   cargo release release   # 0.6.4-rc.2 -> 0.6.4, publishes to @latest
   ```

   Dry-run is the default — add `--execute` to act. Releases are
   refused from any branch but `staging`.

3. Promote by fast-forwarding `stable`. That re-points the `stable`
   dist-tag; it does not publish again.

The first `rc` fixes the target version, and there is no combined
level-plus-prerelease flag, so if scope grows mid-cycle, re-target
explicitly: `cargo release 0.7.0-rc.1`.

### Dist-tags

| Tag | Points at | Install |
| --- | --- | --- |
| `next` | prereleases from `staging` | `npx @tonk/cli@next` |
| `latest` | finals from `staging` | `npx @tonk/cli` |
| `stable` | the version `stable` holds | `npx @tonk/cli@stable` |

`latest` tracks staging finals rather than `stable` on purpose. `stable`
moves on milestones, so mapping `latest` onto it would make the default
install progressively more stale.

The workflow **refuses to publish if the tag disagrees with the Cargo
workspace version**, and `cargo release` keeps them in step by
construction. `cli-npm.yml` can also be run manually
(`gh workflow run cli-npm.yml --ref staging`) to retry a failed publish
without cutting a new tag; it skips any version already on the registry.
```

- [ ] **Step 2: Update the root README install line**

`README.md:93-95` currently reads:

```
If `tonk` was installed some other way, `tonk update` says so instead
of interfering: use `npm i -g @tonk/cli@latest` for an npm install, or
your flake for a nix one.
```

Replace with — note `@latest` is dropped, since it is now the default and naming it invites confusion with `@stable`:

```
If `tonk` was installed some other way, `tonk update` says so instead
of interfering: use `npm i -g @tonk/cli` for an npm install (or
`@tonk/cli@stable` to pin to the last milestone), or your flake for a
nix one.
```

The clause is mid-sentence; the `or your flake for a nix one` tail must still parse.

- [ ] **Step 3: Verify no stale instructions remain**

Run: `grep -rn "git tag v\|push origin v" rust/tonk-cli/npm/README.md README.md`
Expected: no output. Any hit is a leftover manual-tagging instruction that Task 1 replaced.

- [ ] **Step 4: Commit**

```bash
git add rust/tonk-cli/npm/README.md README.md
git commit -m "docs(cli): document cargo-release and the three dist-tags

Replaces the manual bump-then-tag steps, which are what drifted, and
spells out the token scope: a token narrow enough to pass npm whoami and
still fail on PUT has now cost two releases."
```

---

## Self-Review

**Spec coverage:**

| Spec section | Task |
| --- | --- |
| `release.toml` (six overrides) | 1 |
| Devshell addition | 1 |
| Stable promote workflow | 2 |
| Docs — npm README, root README | 3 |
| Model, version flow, ordering wart | documented in 3, enforced by 1 and 2 |
| Out of scope: changelog, crates.io, platforms | absent by construction |
| Blocked: npm token | called out in 3, Step 1 |

**Placeholders:** none. Every step has literal content.

**Type consistency:** the version-extraction pipeline is byte-identical between `cli-npm.yml` and the new promote workflow, and Task 2 Step 3 verifies it against both branches. Tag format `v{{version}}` in Task 1 matches the `v*` glob `cli-npm.yml` already uses.

**Known gap, deliberate:** nothing here can be end-to-end verified until the npm token is fixed, since every path terminates in a registry write. Task 2 Step 4 gets as close as read-only checks allow.
