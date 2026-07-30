# Release process: cargo-release and channel mapping

## Problem

The workspace version and the `v*` tag are produced by separate acts, so
they drift. `chore: release 0.6.1`, `0.6.2`, and `0.6.3` each landed
*inside* an unrelated feature PR (#632, #647, #651) as ordinary commits.
No `v0.6.x` tag exists. There is no point in the process where anyone
would think "and now tag", because there is no release moment — the bump
is a side effect of shipping a feature.

The result: npm `@tonk/cli` latest is **0.4.0** while the workspace is at
**0.6.3**, so `npx @tonk/cli` serves code from two minors ago.

A separate, compounding failure: every `CLI npm` run has failed on npm
auth, so even the tagged releases (`v0.4.0`, `v0.5.0`) never published
from CI. 0.4.0 reached npm by hand, ~40 minutes after its run failed.
That is out of scope here (see Blocked below) but it is why the drift
went unnoticed.

## Constraint: nothing can push to `staging`

Checked against the GitHub rulesets API, after an earlier draft of this
spec claimed staging was unprotected. **That claim was wrong**: it was
checked against the classic branch-protection endpoint
(`/branches/staging/protection`), which returns 404 for a branch governed
only by rulesets. Rulesets are a separate API and a separate object, and
`staging` has two active ones:

- Ruleset 3652307 "Require PR approval before merging to main branch":
  targets `~DEFAULT_BRANCH`, rules `pull_request` + `non_fast_forward`,
  `bypass_actors: []`, `current_user_can_bypass: never`.
- Ruleset 12020040 "Stable branch rules": targets `~DEFAULT_BRANCH`,
  rules `deletion`, `non_fast_forward`, `required_signatures`,
  `required_status_checks`. Bypass is one team, `bypass_mode:
  pull_request` — which is not a bypass of the PR requirement.

`staging` is the default branch, so both apply to it. Additionally
`allow_merge_commit: false`: only squash and rebase are enabled, so a PR
merge always produces a **new SHA**.

Two consequences that shape the whole design:

1. A local `cargo release --execute` cannot work. It ends in a push to
   the current branch, which is rejected, after the commit already
   exists.
2. A locally created tag is useless even if the push were separate. It
   points at the pre-merge commit, and squash-merging produces a
   different SHA — so the tag would name a commit that is not on
   `staging` and never will be.

Tags themselves are unaffected: the rulesets target branches, so CI can
push a `v*` tag freely.

## Model

Deploying and releasing are already separate systems in this repo. The
mistake is treating a staging push as a release. `cli.yml:74` says so
directly: the rolling channel carries `commit` in `manifest.json`
because "a rolling tag cannot identify a build". Staging builds are
already identified by commit, not by version.

So staging deploys continuously and unversioned. Versions stay scarce.

| Event | Trigger | Channel | Cadence |
| --- | --- | --- | --- |
| staging push | branch | `tonk-staging` + CF staging | every merge, unversioned |
| release PR merged with an `-rc.N` version | `v*-rc.N` tag, cut by CI | npm `@next` | on demand |
| release PR merged with a final version | `v*` tag, cut by CI | npm `@latest` | once per cycle |
| stable promote | branch | `tonk-latest` + CF prod, npm `@stable` | at milestones |

An rc exists for when someone needs a citable, installable artifact — a
tester, a pinned bug report. Not one per merge.

### Version flow

A release is two acts. `cargo release` makes the bump commit locally;
CI makes the tag once that commit lands on `staging`.

```
release/*   cargo release rc --execute        0.6.3 -> 0.6.4-rc.1
                | PR, squash-merge
                v
staging     release-tag.yml sees the version change
              -> tag v0.6.4-rc.1 -> dispatch cli-npm.yml -> npm @next

release/*   cargo release rc --execute        -> 0.6.4-rc.2   (repeatable)
                ... same path ...             -> npm @next

release/*   cargo release release --execute   -> 0.6.4
                ... same path ...             -> npm @latest
                |
            promote: git push origin v0.6.4:refs/heads/stable
                v
stable      0.6.4                -> tonk-latest, npm @stable
```

`patch`, `minor`, or `major` in place of `rc` cuts a final without going
through a prerelease at all.

All version changes happen on `staging`. `stable` only ever
fast-forwards, and only to a commit a `v*` tag names.

### Why CI owns the tag

`.github/workflows/release-tag.yml`, on push to `staging`:

1. Extract `[workspace.package] version` at `github.event.before` and at
   `github.sha`.
2. If they differ, and `v<version>` does not already exist, create an
   annotated tag `v<version>` at `github.sha` and push it.
3. Start `cli-npm.yml` against that tag.

The predicate must be a version **change**, not the absence of a tag.
Staging is currently at 0.6.3 with no `v0.6.3`; a tag-absence predicate
would tag an arbitrary HEAD 21 commits past the last release, which is
the failure this spec exists to remove. Where the range cannot be
diffed — `github.event.before` all-zeroes on branch creation, or
unreachable after a force push — the workflow does nothing rather than
guess.

Step 3 is not redundant. A tag pushed with `GITHUB_TOKEN` does not
trigger any workflow: GitHub suppresses events raised by the built-in
token to stop workflows recursing, with `workflow_dispatch` and
`repository_dispatch` as the only exceptions. So the tag push alone
would create a tag and publish nothing. Dispatching `cli-npm.yml`
against the tag ref reaches the same contract by the permitted route,
and needs no new secret — the alternative is a PAT or GitHub App token
whose only job is to launder the push event.

**Why bump on staging rather than stable.** If stable gets its own
version commit it stops being a strict ancestor of staging, and every
future promote needs a merge instead of a fast-forward. Bumping on
staging preserves the ff, and stable still ends up holding exactly
0.6.4. Today staging is +21 on stable and stable is +0 on staging, so
the ff property currently holds and is worth keeping.

**Why `@latest` tracks staging finals, not stable.** Stable moves on
milestones, so it can sit many versions behind. If `@latest` meant
stable, the default `npm install @tonk/cli` would serve increasingly
ancient code — which is the observed situation now (0.4.0 vs 0.6.3),
structurally rather than accidentally. Vetted installs use
`npx @tonk/cli@stable`.

**No change to `cli-npm.yml`.** It already routes a hyphenated version to
`next` and anything else to `latest`, which is exactly this model. Its
`v*` tag glob still covers a hand-pushed tag; the CI path reaches it by
`workflow_dispatch` instead, for the GITHUB_TOKEN reason above, and both
entry points resolve the version from the ref's own `Cargo.toml`. The
only registry-facing addition is `@stable`.

**`@stable` is a dist-tag move, not a publish.** `stable` is only
fast-forwarded to a commit a `v*` tag names, so the version is already on
the registry. Promoting runs
`npm dist-tag add @tonk/cli@<version> stable`, which repoints a tag
without republishing. Only the wrapper needs it: its
`optionalDependencies` pin exact platform versions, so resolving
`@tonk/cli@stable` pulls the correct platform packages automatically.

### The promote target is an invariant, not a convention

Promotion happens at milestones, so the natural fast-forward target is
whatever staging HEAD looks good — which is normally *past* the release
commit. That breaks `@stable`. The workspace version at that later
commit is still `<version>`, so the promote workflow would point
`@stable` at the `<version>` tarball, which was built from the release
commit and does not contain the extra commits. Meanwhile `cli.yml`'s
`publish-stable` builds the `tonk-latest` GitHub release from stable's
real HEAD. Two channels claiming to be stable, different binaries, no
error anywhere.

So `cli-npm-promote.yml` requires `v<version>` to resolve to exactly
`$GITHUB_SHA` and fails with the fast-forward command if it does not.
The documented promote is `git push origin v<version>:refs/heads/stable`,
which satisfies it by construction.

An earlier draft of this spec called the equivalent skew "cosmetic" on
the grounds that the release commit is bit-identical on both branches.
That only held while promotion was assumed to target the release commit;
it is an invariant now rather than an assumption.

### Ordering dependency

The tag's publish run must finish before `stable` is promoted, because
promoting checks the registry. Promote early and it fails with
`@tonk/cli@<version> is not published; nothing to promote`, which reads
like a design violation but means "not yet". Documented in the README,
along with `gh workflow run cli-npm-promote.yml --ref stable` to re-run
one.

## release.toml

At the workspace root. Every line overrides a default that would
otherwise do damage.

| Setting | Value | Why |
| --- | --- | --- |
| `publish` | `false` | Default `true` attempts `cargo publish` on all 34 crates. None are on crates.io. |
| `consolidate-commits` | `true` | Default `false` produces 34 separate commits, one per crate. |
| `push` | `false` | Default `true`. The ruleset rejects a push to `staging`, and cargo-release hits it *after* writing the commit, leaving a half-done release and a confusing error. |
| `tag` | `false` | Default `true`. A tag cut locally points at the pre-merge commit, and the squash merge produces a different SHA. `release-tag.yml` owns the tag. |
| `tag-name` | `"v{{version}}"` | Retained under `tag = false` for the manual escape hatch (`--tag` plus a hand push) and as the name `release-tag.yml` must match. Default `{{prefix}}v{{version}}` yields `tonk-cli-v0.6.4`, which does not match `cli-npm.yml`'s `v*` glob and would never trigger a publish. |
| `pre-release-commit-message` | `"chore: release {{version}}"` | Matches the existing convention; the default injects a crate name and a capital R. |
| `tag-message` | `"chore: release {{version}}"` | Same reason. |
| `allow-branch` | `["staging", "release/*"]` | Default `["*", "!HEAD"]`. The guardrail against a release cut from an unrelated feature branch. `release/*` is the normal path, since the bump needs a PR anyway; `staging` stays allowed for a maintainer who already has it checked out. `stable` is absent on purpose. |

Dry-run is cargo-release's default; `--execute` is required to act. Under
`push = false` and `tag = false`, `--execute` produces exactly one
artifact: the commit `chore: release <version>` on the current branch.

`cargo release release` deserves a warning in the docs: it only strips an
existing pre-version. Run from a final version it has nothing to strip,
changes no version, writes no commit, and exits 0 — a silent no-op that
reads as success. `patch`, `minor`, and `major` are the levels that cut a
final directly.

## Devshell

Add `cargo-release` to `devShellBuildInputs` in `flake.nix`. Declarative,
no imperative install.

Specifically **not** `commonBuildInputs`: `flake.nix` passes that to
`nix/rust.nix` as `buildInputs`, which become `nativeBuildInputs` on
`buildDepsOnly` and all 34 crate derivations. Adding a tool there changes
every crate hash, forces a cold cachix rebuild on both runners, and puts
a release tool in every build sandbox. `devShellBuildInputs` is
`commonBuildInputs ++ [...]`, so `nix develop` still gets it.

## Stable promote workflow

New file `.github/workflows/cli-npm-promote.yml`, triggered on push to
`stable` and on `workflow_dispatch`:

1. Refuse unless the ref is `refs/heads/stable`. `workflow_dispatch`
   accepts any ref, and dispatching against `staging` would point
   `@stable` at a version `stable` does not hold.
2. Read the workspace version from the stable checkout, and refuse a
   prerelease.
3. Require `v<version>` to resolve to exactly `$GITHUB_SHA` — see "The
   promote target is an invariant" above. The error names the
   fast-forward command.
4. Verify `@tonk/cli@<version>` exists on the registry. If it does not,
   fail loudly — it means stable holds a version that was never
   released, which is a real problem worth surfacing rather than
   silently tagging nothing. Capture npm's output and distinguish
   `code E404` from any other failure, so a registry outage does not get
   reported as an unpublished release.
5. `npm dist-tag add @tonk/cli@<version> stable`.

A separate file rather than a job inside `cli-npm.yml`: adding a branch
trigger there would require `if:` guards on all three existing jobs, and
a single missed guard means publishing on every stable push. The cost is
that npm-registry mutations now live in two files.

Note that the first promote after this lands will fail by design.
`stable` holds 0.6.0, npm's highest version is 0.4.0, and there is no
`v0.6.0` tag. It self-heals after one full cycle; no workaround.

## Docs

- `rust/tonk-cli/npm/README.md`, "Publishing (maintainers)": replace the
  manual bump-then-tag steps with the four-step flow — cut the bump on
  `release/*`, PR it, let CI tag, promote to the release commit — and
  document all three dist-tags. State which level cuts a final and that
  `release` is a no-op from one. State that promoting has to wait for the
  tag's publish run. The existing text already specifies the correct
  token shape ("an npm automation token for the `@tonk` scope with
  publish rights"), which is what the live token is not. Keep the local
  `npm pack` verification recipe, but glob the wrapper tarball rather
  than naming a version — the committed `package.json` versions are
  placeholders that CI stamps over, so a literal filename rots.
- Root `README.md` line 94 references `@tonk/cli@latest`. Add `@stable`
  and say which is which. Same for the npm remedy string in
  `rust/tonk-cli/src/update/swap.rs`: under this model `@latest` means
  staging finals, so telling an npm user to install `@latest` moves a
  `@stable`-pinned install onto a faster channel.

## Verification

All mechanics below were confirmed empirically in an isolated two-crate
workspace with an inherited `version.workspace`, before writing this
spec:

| Claim | Evidence |
| --- | --- |
| `rc` from 0.6.3 gives `0.6.4-rc.1`; repeating gives `rc.2` | executed both |
| `release` strips the prerelease to `0.6.4` | executed |
| One consolidated commit per release, `Cargo.toml` + `Cargo.lock` only | `2 files changed` |
| One annotated tag named `v<version>` | `git tag -l` = 1 entry; `git cat-file -t` = `tag` |
| `publish = false` genuinely skips the registry | crates absent from crates.io, no token configured, exit 0, no registry error |
| `allow-branch` blocks a wrong-branch release | `error: cannot release from branch 'feat/something' as it doesn't match 'staging'` |

That prototype ran with `push = true` and `tag = true`, so the tag rows
describe cargo-release's capability rather than this repo's
configuration. Under `tag = false` no local tag is produced at all; the
`v<version>` name lives on in `release-tag.yml`.

Two gotchas found in the process, recorded so nobody re-derives them:

- cargo-release prints `Publishing <all crates>` **even with
  `publish = false` and `--no-publish`**. The line is cosmetic. Do not
  read it as evidence of a crates.io attempt.
- In dry-run mode cargo-release collects errors and continues printing
  the plan, so a blocked release still prints the rest of its plan below
  the error. Only `--execute` aborts at the check. Read the `error:` line,
  not the absence of one.

Confirmed in this workspace, with cargo-release 1.1.2, after the config
landed:

| Claim | Evidence |
| --- | --- |
| all seven settings parse | `cargo release config` shows `push = false`, `tag = false`, `allow-branch = ["staging", "release/*"]`, `publish = false`, `consolidate-commits = true`, `tag-name`, `tag-message`, `pre-release-commit-message` |
| the branch guard fires | ``error: cannot release from branch `build/cargo-release` as it doesn't match `staging`, `release/*` `` |
| `patch` bumps to 0.6.4 with no tag and no push | 35 `Upgrading ... 0.6.3 to 0.6.4` lines, zero `Tagging`/`Pushing`/`error:` lines |
| `rc` bumps to 0.6.4-rc.1 | `Upgrading tonk-cli from 0.6.3 to 0.6.4-rc.1` |
| `release` from a final is a silent no-op | zero `Upgrading` lines, exits on the dry-run abort alone |
| the tag predicate discriminates | over real staging commits: `dbb9b9c4c..65d105b52` (0.6.2 → 0.6.3) yields `TAG v0.6.3`; `7875f6427..dbb9b9c4c` yields `no tag (version unchanged at 0.6.2)`; all-zeroes and unreachable `before` both yield no tag |
| both workflows lint | `actionlint` exit 0, no output |

## Out of scope

- Changelog generation. There is no changelog today, and
  conventional-commit changelogs are a separate argument.
- crates.io publishing. `publish = false` covers cargo-release; a
  Cargo-level `publish = false` per crate would be strictly stronger but
  is 34 edits for a risk that requires a crates.io token to materialise.
- New platform targets.

## Blocked

Nothing publishes until the npm token is fixed. `npm whoami` succeeds as
`tonk-labs`, and `tonk-labs` owns all three packages, but
`PUT @tonk/cli-darwin-arm64` returns 404 — npm's response for an
unauthorised publish. The token needs read-write on the whole `@tonk`
scope, not a package subset, so newly added platform packages keep
working. This spec is implementable and mergeable regardless; it just
cannot produce a published artifact yet.
