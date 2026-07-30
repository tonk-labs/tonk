# npm distribution for the tonk CLI

This directory packages the native `tonk` binary (built from
`rust/tonk-cli`) for npm, so `npx @tonk/cli` and `npm i -g @tonk/cli`
work with no Rust toolchain.

## Layout

- `cli/` — the published wrapper package **`@tonk/cli`**. Its
  `bin/tonk.js` launcher resolves the matching platform package and
  execs the native binary. It declares the platform packages as
  `optionalDependencies`, so npm downloads only the one for the host.
- `darwin-arm64/` — **`@tonk/cli-darwin-arm64`** (`os: darwin`,
  `cpu: arm64`).
- `linux-x64/` — **`@tonk/cli-linux-x64`** (`os: linux`, `cpu: x64`).

The platform packages' `bin/tonk` binaries are **build artifacts**, not
committed (`.gitignore`d). Release CI (or the local pack test below)
injects them before publishing.

Adding a platform later = one nix build matrix row in
`.github/workflows/cli-npm.yml`, a new `<platform>/package.json` here,
and an entry in the wrapper's `optionalDependencies`.

## Publishing (maintainers)

Publishing runs in CI so every platform binary is built reproducibly —
you cannot build the Linux binary from a Mac. It is **tag-driven**;
nothing publishes on a normal push.

The bump and the tag are separate acts on separate machines, and have to
be. Rulesets on `staging` require a pull request and forbid
non-fast-forward with no bypass, so nobody can push a release commit
directly; and because merge commits are disabled, merging rewrites the
SHA, so a tag created locally would point at a commit that never lands.
So `cargo release` makes only the commit, and `release-tag.yml` creates
the `v<version>` tag in CI once that commit is on `staging`.

1. One-time: add an `NPM_TOKEN` repository secret — an npm automation
   token with read-write on the whole **`@tonk` scope**, not a package
   subset, so platform packages added later keep working. A token
   scoped too narrowly still passes `npm whoami` and then fails with a
   bare `404 Not Found - PUT`, which reads like a missing package.

2. Cut the bump on a `release/*` branch off current `staging`:

   ```sh
   git fetch origin && git switch -c release/0.6.4 origin/staging
   nix develop            # cargo-release lives in the devshell
   cargo release patch --execute   # 0.6.3 -> 0.6.4
   ```

   Dry-run is the default; `--execute` acts. `release.toml` sets
   `push = false` and `tag = false`, so this writes exactly one commit,
   `chore: release 0.6.4` (`Cargo.toml` plus `Cargo.lock`), and touches
   nothing else. A release is refused from any branch but `staging` or
   `release/*`, and refused outright if the tree is dirty.

   Which level to use:

   | Command | From | To | Ends up on |
   | --- | --- | --- | --- |
   | `cargo release patch` (also `minor`, `major`) | 0.6.3 | 0.6.4 | `@latest` |
   | `cargo release rc` | 0.6.3 | 0.6.4-rc.1 | `@next` |
   | `cargo release rc` | 0.6.4-rc.1 | 0.6.4-rc.2 | `@next` |
   | `cargo release release` | 0.6.4-rc.2 | 0.6.4 | `@latest` |

   `release` **only strips an existing prerelease**. Run from a final
   version it has nothing to strip, so it changes no version and makes
   no commit while still exiting 0 — which reads as success. To cut a
   final directly, use `patch`, `minor`, or `major`.

   The first `rc` fixes the target version and there is no combined
   level-plus-prerelease flag, so if scope grows mid-cycle, re-target
   explicitly: `cargo release 0.7.0-rc.1 --execute`.

3. Open a PR for that one commit and merge it to `staging`. On the
   merge, `release-tag.yml` notices that `[workspace.package] version`
   changed across the pushed range, creates the annotated tag
   `v0.6.4` at the merged commit, and starts `cli-npm.yml` — which
   builds both platform binaries and publishes them.

   It tags on a version *change* only. A missing `v<version>` is never
   on its own a reason to tag, which is why staging can sit well past
   0.6.3 with no `v0.6.3` and nothing fires.

4. At a milestone, promote by fast-forwarding `stable` **to the release
   commit itself**:

   ```sh
   git push origin v0.6.4:refs/heads/stable
   ```

   `stable` is always an ancestor of `staging`, so that is a
   fast-forward. If it is rejected as non-fast-forward, something put a
   commit on `stable` that is not on `staging` — sort that out rather
   than forcing it.

   `cli-npm-promote.yml` then re-points the `stable` dist-tag; it never
   publishes. It requires `stable`'s HEAD to be exactly the commit
   `v<version>` names and fails otherwise: fast-forwarding to some later
   staging commit would leave `@stable` serving a tarball that does not
   contain the code `stable` holds, while `cli.yml` builds the
   `tonk-latest` GitHub release from stable's real HEAD.

   Wait for the `CLI npm` run on the tag to finish before promoting.
   Promote too early and it fails with
   `@tonk/cli@<version> is not published; nothing to promote`, which
   means "not yet" rather than a broken release. Re-run a promote with
   `gh workflow run cli-npm-promote.yml --ref stable`.

### Dist-tags

| Tag | Points at | Install |
| --- | --- | --- |
| `next` | prereleases from `staging` | `npx @tonk/cli@next` |
| `latest` | finals from `staging` | `npx @tonk/cli` |
| `stable` | the release commit `stable` holds | `npx @tonk/cli@stable` |

`latest` tracks staging finals rather than `stable` on purpose. `stable`
moves on milestones, so mapping `latest` onto it would make the default
install progressively more stale.

The workflow **refuses to publish if the tag disagrees with the Cargo
workspace version**, and `release-tag.yml` derives the tag from that
version, so they cannot diverge. `cli-npm.yml` can also be run manually
(`gh workflow run cli-npm.yml --ref v0.6.4`) to retry a failed publish
without cutting a new tag; it skips any version already on the registry.

To verify locally without publishing, from `rust/tonk-cli/npm`:

```sh
# darwin-arm64 only (the binary you can build on a Mac)
nix build --accept-flake-config ../../..#tonk-cli
install -Dm0755 ../../../result/bin/tonk darwin-arm64/bin/tonk
npm pack ./darwin-arm64 ./cli            # produces .tgz tarballs
# smoke-test the launcher against the packed tarballs. The glob avoids
# hardcoding a version: the committed package.json versions are stale by
# design (see below), so a literal filename here rots.
tmp=$(mktemp -d) && npm --prefix "$tmp" install ./tonk-cli-darwin-arm64-*.tgz ./tonk-cli-[0-9]*.tgz
"$tmp/node_modules/.bin/tonk" --help
```

There is one version in this repo: `version.workspace` in the root
`Cargo.toml`, which `cargo release` bumps. CI stamps it into all three
`package.json`s at publish time, so `tonk --version` always matches the
published `@tonk/cli` version. The versions committed in those
`package.json`s are placeholders — nothing reads them, and hand-editing
them to "keep up" is the manual step this process removed.
