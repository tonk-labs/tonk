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
you cannot build the Linux binary from a Mac. npm Trusted Publishing is
attached to `.github/workflows/cli-npm.yml`; the workflow uses GitHub
OIDC and `npm publish`, with no repository npm token.

The bump and the tag are separate acts on separate machines, and have to
be. Rulesets on `staging` require a pull request and forbid
non-fast-forward with no bypass, so nobody can push a release commit
directly; and because merge commits are disabled, merging rewrites the
SHA, so a tag created locally would point at a commit that never lands.
So `cargo release` makes only the commit, and `release-tag.yml` creates
the `v<version>` tag in CI once that commit is on `staging`.

1. Cut the bump on a `release/*` branch off current `staging`:

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

   | Command | From | To | npm result after merge |
   | --- | --- | --- | --- |
   | `cargo release patch` (also `minor`, `major`) | 0.6.3 | 0.6.4 | tagged; unpublished until stable promotion |
   | `cargo release rc` | 0.6.3 | 0.6.4-rc.1 | publish `@next` |
   | `cargo release rc` | 0.6.4-rc.1 | 0.6.4-rc.2 | publish `@next` |
   | `cargo release release` | 0.6.4-rc.2 | 0.6.4 | tagged; unpublished until stable promotion |

   `release` **only strips an existing prerelease**. Run from a final
   version it has nothing to strip, so it changes no version and makes
   no commit while still exiting 0 — which reads as success. To cut a
   final directly, use `patch`, `minor`, or `major`.

   The first `rc` fixes the target version and there is no combined
   level-plus-prerelease flag, so if scope grows mid-cycle, re-target
   explicitly: `cargo release 0.7.0-rc.1 --execute`.

2. Open a PR for that one commit and merge it to `staging`. On the
   merge, `release-tag.yml` notices that `[workspace.package] version`
   changed across the pushed range, creates the annotated tag
   `v0.6.4` at the merged commit. A prerelease tag immediately dispatches
   `cli-npm.yml` and publishes `next`. A final tag is created but remains
   unpublished until the same commit reaches `stable`.

   It tags on a version *change* only. A missing `v<version>` is never
   on its own a reason to tag, which is why staging can sit well past
   0.6.3 with no `v0.6.3` and nothing fires.

3. For a final release, wait for `v<version>` to be created, verify the
   version has no prerelease suffix, then fast-forward `stable` **to the
   release commit itself**:

   ```sh
   git push origin v0.6.4:refs/heads/stable
   ```

   `stable` is always an ancestor of `staging`, so that is a
   fast-forward. If it is rejected as non-fast-forward, something put a
   commit on `stable` that is not on `staging` — sort that out rather
   than forcing it.

   The push starts `CLI npm`, which proves that the checkout, the
   immutable version tag, and `origin/stable` are the same commit before
   publishing `latest`. Watch that workflow run through all platform
   and wrapper packages. A later staging commit is not a valid promotion
   target, even when it is a descendant of the release.

### Recovery

Never mint a replacement tag or move an existing one. To retry a partial
prerelease publish, dispatch the existing prerelease tag:

```sh
gh workflow run cli-npm.yml --ref v0.6.4-rc.1
```

To retry a final publish, use the same command at the final tag, but only
after verifying `origin/stable` resolves to that tag commit:

```sh
git fetch --no-tags origin refs/heads/stable:refs/remotes/origin/stable
test "$(git rev-parse origin/stable)" = "$(git rev-parse 'v0.6.4^{commit}')"
gh workflow run cli-npm.yml --ref v0.6.4
```

The channel policy rejects a premature final retry. Publication retains
the existing partial-failure behavior: packages already present at that
version are skipped while missing platform or wrapper packages continue.

### Dist-tags

| Tag | Points at | Install |
| --- | --- | --- |
| `next` | newest explicitly released prerelease from `staging` | `npx @tonk/cli@next` |
| `latest` | final release commit held by `stable` | `npx @tonk/cli` |

Bare `npx @tonk/cli` and `npm install -g @tonk/cli` are stable installs.
Prereleases always require the explicit `next` tag.

The legacy npm `stable` alias is frozen at the cutover final for
compatibility. New automation and documentation must not use
`@tonk/cli@stable`; removing the alias later requires a separately
announced compatibility decision.

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
