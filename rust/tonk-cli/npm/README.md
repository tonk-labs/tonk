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

To verify locally without publishing, from `rust/tonk-cli/npm`:

```sh
# darwin-arm64 only (the binary you can build on a Mac)
nix build --accept-flake-config ../../..#tonk-cli
install -Dm0755 ../../../result/bin/tonk darwin-arm64/bin/tonk
npm pack ./darwin-arm64 ./cli            # produces .tgz tarballs
# smoke-test the launcher against the packed tarballs:
tmp=$(mktemp -d) && npm --prefix "$tmp" install ./tonk-cli-darwin-arm64-*.tgz ./tonk-cli-0.5.0.tgz
"$tmp/node_modules/.bin/tonk" --help
```

The npm package version tracks the Rust workspace version
(`version.workspace` in the root `Cargo.toml`), so `tonk --version`
matches the published `@tonk/cli` version. Bump both together.
