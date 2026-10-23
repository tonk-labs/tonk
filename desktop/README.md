<p align="center">
  <img src="Resources/Tonk.png" width="180" alt="Tonk app icon">
</p>

<h1 align="center">Tonk</h1>

<p align="center">
  A native macOS workspace for Tonk spaces and agents.
</p>

## Download Tonk Desktop

For macOS 15+ on Apple Silicon. Unzip the download and move **Tonk Nightly.app**
to **Applications**.

| Channel | Download | Published from |
| --- | --- | --- |
| Nightly | [Download for Mac](https://github.com/tonk-labs/tonk/releases/download/tonk-desktop-dev/Tonk-macos-arm64.zip) | Latest successfully published `main` build |

Nightly is its own app, **Tonk Nightly** (`xyz.tonk.nightly`), and uses
`tonk.foundation` by default. The stable app, **Tonk** (`xyz.tonk`), uses
`tonk.network`; it is not published until Desktop ships to production. The two
have separate identities and local data, so they can be installed side by side.
Builds are Developer ID signed and notarized, and include
`Tonk-macos-arm64.sha256` and `Tonk-desktop-manifest.json` with the exact
version, channel and source commit. Downloads are manual.

## Build

Requires macOS 15+, full Xcode 26+, and Node 22+ for shared-source generation.
Run from the monorepo root:

```sh
# Shared MCP dependencies (once)
npm --prefix mcp ci --ignore-scripts

# With Nix
nix develop
dev:desktop

# Without Nix
bash desktop/scripts/build-app.sh
open desktop/.build/Tonk.app
```

Builds are stable by default. Set `TONK_DESKTOP_CHANNEL=nightly` to build
`desktop/.build/Tonk Nightly.app`, which defaults to `tonk.foundation`.

The root flake and lockfile cover the whole monorepo. Run `menu` for commands,
including `build:desktop`, `dev:desktop:open`, `test:desktop`, and
`test:desktop:workflow`. Desktop commands are available on macOS; Xcode remains
a host prerequisite. Shared generation and MCP tests use `node mcp/scripts/sync-desktop.mjs` and
`npm --prefix mcp test`.

## Use

Sign in to Tonk, open a space, and click **New chat**. Choose your model provider
in **Settings…**.

ChatGPT sign-in requires the [Codex CLI](https://learn.chatgpt.com/docs/cli).
Claude subscription access requires [Claude Code](https://code.claude.com/docs/en/setup).

Chats and drafts stay on your Mac. Messages go to your selected provider;
API keys are stored in macOS Keychain.

## Shared tools and context

The app is a consumer of [`../mcp`](../mcp/README.md). Build and test commands
regenerate the native catalog, guides and worker adapter from that package.
`node mcp/scripts/sync-desktop.mjs --check` checks all generated consumers.
New chats use query/evaluate/guide plus native proposal and inspection controls.
Host connection code stays in this app; Tonk semantics and authoring text live in
the monorepo MCP sources. See [MIGRATION.md](MIGRATION.md) for import provenance.

## Local end-to-end workflow

Build the app, serve a built Tonk web runtime locally, then run:

```sh
python3 desktop/scripts/serve-local-runtime.py /path/to/built/runtime 4187
# In another terminal:
bash desktop/scripts/dev.sh test:workflow
```

The runner makes a separately identified, ad-hoc-signed test copy and fresh
WebKit/application data. Reports are saved under `desktop/artifacts/agent-workflow-*`.
It drives the real native MCP, worker and rendered checkbox, then restarts the
app to check persistence. It does not invoke a language model or exercise
account-backed proposal acceptance, authentication, remote sync or sharing.

## CI signing and direct passkeys

`.github/workflows/desktop.yml` runs native tests, generator checks and an ad-hoc
build on relevant pull requests and main pushes. Signed artifacts are opt-in on
manual runs of main only. Missing credentials do not block ordinary build/tests.
Xcode stays a host prerequisite; wrappers clear inherited SDKROOT and select
`/Applications/Xcode.app` (override with `TONK_XCODE_DEVELOPER_DIR`).
Local recovery validation used Xcode 27; hosted macOS 26 CI remains separate.

Repository secrets:

- `MACOS_CERTIFICATE_P12`: the existing base64-encoded Tonk signing identity and private key archive used for CLI releases.
- `MACOS_CERTIFICATE_PASSWORD`: its archive password.
- `TONK_PROVISIONING_PROFILE_BASE64`: base64-encoded Apple profile for `8WVKS2F24C.xyz.tonk`, authorizing that exact certificate and Associated Domains.

The signing script imports the identity into a temporary runner keychain and
restores/deletes it on exit. It requires a matching unexpired profile, embeds
it, enables `TonkDirectPasskeyEnabled`, and checks the signed team, leaf
certificate, associated-domain entitlements, embedded profile and app flag.
Missing credentials or a mismatched profile fail the signing stage.

The optional manual artifact is signed and provisioned. Automatic releases use
`desktop-dev.yml`: every main push builds the nightly channel through the shared
`desktop-release.yml` build. Nightly is signed without the passkey profile, which
covers only `xyz.tonk`. Version tags do not publish Desktop until the stable
channel ships.
These release builds use Swift release configuration, hardened runtime, a secure
signing timestamp, required Apple notarization, and a stapled ticket. Publication
fails if signing, notarization or verification fails. Superseded dev builds do
not overwrite the current main build.

Release notarization also requires `APP_STORE_CONNECT_KEY_P8`,
`APP_STORE_CONNECT_KEY_ID`, and `APP_STORE_CONNECT_ISSUER_ID`, shared with CLI CI.

Direct passkeys are enabled only in the stable app on `https://tonk.network`; that domain
must serve an AASA `webcredentials` entry for `8WVKS2F24C.xyz.tonk`. CI does not
perform a real passkey ceremony or prove recovery of an existing account.

## Activity layout fixture

After an ad-hoc build, a disposable copy can be launched with `--local-runtime
--data-dir /absolute/path/to/fresh-data --web-data-id NEW-UUID --activity-fixture`.
Give the copy a unique bundle identifier and ad-hoc signature, as the workflow
runner does. This renders the actual ContentView with two activity-bearing turns
without connecting a model or loading account data. It requires empty chat state;
it is layout-only evidence, never live provider proof.
