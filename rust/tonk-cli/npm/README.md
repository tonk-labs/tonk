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

1. One-time: add an `NPM_TOKEN` repository secret (an npm automation
   token for the `@tonk` scope with publish rights).
2. Bump the version (see below) and land it.
3. Push a `v<version>` tag matching the Cargo workspace version, e.g.
   `git tag v0.5.0 && git push origin v0.5.0`. That fires
   `.github/workflows/cli-npm.yml`, which builds both binaries, stamps
   the version across all three package.jsons, and publishes the
   platform packages first, then the wrapper.

The tag is the source of the version, and the workflow **refuses to
publish if it disagrees with the Cargo workspace version** — so the
bump has to land before the tag. The dist-tag is derived, not chosen:
a prerelease version (`0.5.1-rc.1`) publishes under `next`, anything
else under `latest`.

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
