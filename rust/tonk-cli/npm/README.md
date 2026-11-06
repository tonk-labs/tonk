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

Adding a platform later = one nix build matrix row in each of
`.github/workflows/cli-npm.yml` and `release.yml`, a new `<platform>/package.json` here,
and an entry in the wrapper's `optionalDependencies`.

## Publishing (maintainers)

Publishing runs in CI so every platform binary is built reproducibly;
you cannot build the Linux binary from a Mac. npm Trusted Publishing is
attached to `.github/workflows/cli-npm.yml`, which uses GitHub OIDC and
`npm publish` with no repository npm token.

A release is a `v*` tag on a commit already on `main`. Push one with git:

```sh
git fetch origin
git tag v0.7.0-rc.1 origin/main
git push origin v0.7.0-rc.1
```

or draft a release in the GitHub UI with a new tag targeting `main`. The
tag decides everything else:

| Tag | Channel | Web | CLI release | npm |
| --- | --- | --- | --- | --- |
| `v0.7.0-rc.1` (has a `-`) | staging | staging | pre-release, and the rolling `tonk-staging` | `next` |
| `v0.7.0` (no `-`) | stable | tonk.network | GitHub `latest` | `latest` |

The tag is the version. CI stamps it into `[workspace.package] version`
and `Cargo.lock` before building, so `tonk --version` and all three
`package.json`s match the tag. The version committed in `Cargo.toml` is
only what untagged builds report.

A final release is normally the same commit as its last rc, so stable
runs exactly what staging ran:

```sh
git tag v0.7.0 'v0.7.0-rc.2^{commit}'
git push origin v0.7.0
```

### Recovery

Never move or re-push an existing tag. Re-run the failed workflow run for
that tag instead. npm skips versions that already landed and continues
with the missing platform or wrapper packages; the GitHub release is
updated in place.

### Dist-tags

| Tag | Points at | Install |
| --- | --- | --- |
| `next` | newest prerelease tag | `npx @tonk/cli@next` |
| `latest` | newest final tag | `npx @tonk/cli` |

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

The versions committed in the `package.json`s are placeholders. CI
stamps the release tag's version into all three at publish time, and
nothing else reads them.

## CLI artifact smoke gates

`cli-smoke.yml` runs after PR builds and gates both GitHub and npm
publication. It tests the exact downloaded artifacts for every currently
shipped platform; it does not add support for new platforms.

| Artifact | Runtime | Additional checks |
| --- | --- | --- |
| Linux x86_64 | Clean Debian and Alpine containers, no Nix store | Static x86_64 ELF, no interpreter or shared-library dependencies |
| macOS arm64 | Fresh native macOS runner, no Nix installation | arm64 Mach-O executable, system libraries only, no Nix load commands, valid signature |

Both platforms check direct `--version`/`--help`, packed npm platform
OS/CPU selectors, the wrapper's exact optional dependency version, offline
installation, byte-for-byte binary preservation, and launcher version/help.
The disposable packages are stamped to the artifact version, matching release
packaging. A local HTTPS fixture rejects an untrusted certificate and reaches
manifest validation with the trusted CA supplied via `SSL_CERT_FILE`.
The fixture deliberately rejects the wrong update channel before any download
or replacement. It does not modify the macOS keychain or host CA store.

PR macOS artifacts use the Nix build's ad-hoc signature. They do not need
Developer ID credentials or Apple notarization. Release callers additionally
require the resolved tag version, a Developer ID Application signature, and
Apple notarization on the downloaded bytes. The existing signing action still
signs/notarizes before upload; these checks do not create credentials or submit
new artifacts for notarization.

From the repository root, run the guard regressions on Linux x86_64 or macOS
arm64 with Python, a C compiler, npm, and native inspection tools:

```sh
python3 scripts/tests/test-cli.py
```

Run the real artifact smoke check on its native platform:

```sh
# In a clean Debian/Alpine container with Python, binutils, OpenSSL and npm:
python3 scripts/check-cli.py /path/to/tonk --platform linux-x64
# On macOS arm64 with Python, OpenSSL, npm and Apple command-line tools:
python3 scripts/check-cli.py /path/to/tonk --platform darwin-arm64
```

The reusable workflow provides the complete runtime setup. No package is
published and no Tonk account is used by these checks. Release-only Apple
notarization verification may contact Apple; the TLS fixture uses only loopback.

After an authorized release, also download and checksum-verify the exact
GitHub archives and repeat the corresponding artifact checks. Verify a fresh
`npx @tonk/cli@<released-version> --version` and the intended `latest` or `next`
alias independently: offline tests cannot prove registry propagation or
dist-tag correctness. Windows, Linux arm64 and macOS x86_64 are not currently
shipped and therefore are not added to this gate.
