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
| `cargo release rc` | `v*-rc.N` tag | npm `@next` | on demand |
| `cargo release release` | `v*` tag | npm `@latest` | once per cycle |
| stable promote | branch | `tonk-latest` + CF prod, npm `@stable` | at milestones |

An rc exists for when someone needs a citable, installable artifact — a
tester, a pinned bug report. Not one per merge.

### Version flow

```
staging  cargo release rc        0.6.3 -> 0.6.4-rc.1   tag v0.6.4-rc.1 -> npm @next
         cargo release rc              -> 0.6.4-rc.2   tag v0.6.4-rc.2 -> npm @next
         cargo release release         -> 0.6.4        tag v0.6.4      -> npm @latest
             |
         promote (fast-forward)
             v
stable   0.6.4                        -> tonk-latest, npm @stable
```

All version changes happen on `staging`. `stable` only ever
fast-forwards.

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

**No change to the existing dist-tag derivation.** `cli-npm.yml` already
routes a hyphenated version to `next` and anything else to `latest`,
which is exactly this model, and its `v*` tag glob already matches
`v0.6.4-rc.1`. The only registry-facing addition is `@stable`.

**`@stable` is a dist-tag move, not a publish.** Because promotion is a
fast-forward, the release commit is identical on both branches and the
version is already on the registry. Promoting runs
`npm dist-tag add @tonk/cli@<version> stable`, which repoints a tag
without republishing. Only the wrapper needs it: its
`optionalDependencies` pin exact platform versions, so resolving
`@tonk/cli@stable` pulls the correct platform packages automatically.

### Ordering wart

The final tag fires npm `@latest` before stable is fast-forwarded, so
for the interval between the two, `@latest` describes a commit that is
not yet on stable. The commit is bit-identical either way, so the skew
is cosmetic. Accepted. If it ever matters, the zero-skew form is
`cargo release release --no-push`, ff stable, then push the tag.

## release.toml

At the workspace root. Every line overrides a default that would
otherwise do damage.

| Setting | Value | Why |
| --- | --- | --- |
| `publish` | `false` | Default `true` attempts `cargo publish` on all 34 crates. None are on crates.io. |
| `consolidate-commits` | `true` | Default `false` produces 34 separate commits, one per crate. |
| `tag-name` | `"v{{version}}"` | Default `{{prefix}}v{{version}}` yields `tonk-cli-v0.6.4`, which does not match `cli-npm.yml`'s `v*` glob and would never trigger a publish. |
| `pre-release-commit-message` | `"chore: release {{version}}"` | Matches the existing convention; the default injects a crate name and a capital R. |
| `tag-message` | `"chore: release {{version}}"` | Same reason. |
| `allow-branch` | `["staging"]` | Default `["*"]`. This is the guardrail against the actual failure mode: a release cut from the wrong branch. |

`push` keeps its default of `true`. Atomicity is the entire point — bump,
tag, and push as one act, leaving no window in which the tag can be
forgotten.

Dry-run is cargo-release's default; `--execute` is required to act.

## Devshell

Add `cargo-release` to `devShellBuildInputs` in `flake.nix`. Declarative,
no imperative install.

## Stable promote workflow

New file `.github/workflows/cli-npm-promote.yml`, triggered on push to
`stable`:

1. Read the workspace version from the stable checkout.
2. Verify `@tonk/cli@<version>` exists on the registry. If it does not,
   fail loudly — it means stable holds a version that was never
   released, which is a real problem worth surfacing rather than
   silently tagging nothing.
3. `npm dist-tag add @tonk/cli@<version> stable`.

A separate file rather than a job inside `cli-npm.yml`: adding a branch
trigger there would require `if:` guards on all three existing jobs, and
a single missed guard means publishing on every stable push. The cost is
that npm-registry mutations now live in two files.

## Docs

- `rust/tonk-cli/npm/README.md`, "Publishing (maintainers)": replace the
  manual bump-then-tag steps with the cargo-release commands, and
  document all three dist-tags. The existing text already specifies the
  correct token shape ("an npm automation token for the `@tonk` scope
  with publish rights"), which is what the live token is not. Keep the
  local `npm pack` verification recipe as-is.
- Root `README.md` line 94 references `@tonk/cli@latest`. Add `@stable`
  and say which is which.

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

Two gotchas found in the process, recorded so nobody re-derives them:

- cargo-release prints `Publishing <all crates>` **even with
  `publish = false` and `--no-publish`**. The line is cosmetic. Do not
  read it as evidence of a crates.io attempt.
- In dry-run mode cargo-release collects errors and continues printing
  the plan, so a blocked release still shows a `Pushing ...` line before
  failing at the end. Only `--execute` aborts at the check.

Remaining verification during implementation:

- `cargo release rc` dry run from `staging` in the real workspace: expect
  one commit, one tag `v0.6.4-rc.1`, no crates.io attempt.
- `actionlint` on the new promote workflow.

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
