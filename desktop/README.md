<p align="center">
  <img src="Resources/Tonk.png" width="180" alt="Tonk app icon">
</p>

<h1 align="center">Tonk</h1>

<p align="center">
  A native macOS workspace for Tonk spaces and agents.
</p>

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

The root flake and lockfile cover the whole monorepo. Run `menu` for commands,
including `build:desktop`, `dev:desktop:open`, `test:desktop`, and
`test:desktop:workflow`. Desktop commands are available on macOS; Xcode remains
a host prerequisite. Shared generation and MCP tests use `build:agent` and
`test:mcp`.

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
`node mcp/scripts/build-agent-surface.mjs --check` checks all generated consumers.
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

`.github/workflows/desktop.yml` runs native tests and an ad-hoc build on pull
requests. Pushes to `main` and manual runs build a signed Apple Silicon app,
verify its passkey provisioning, and upload a ZIP plus SHA-256 checksum for
14 days. They use the root Nix CI shell and the host Xcode installation on the
[macOS 26 runner](https://github.com/actions/runner-images/blob/main/images/macos/macos-26-arm64-Readme.md).

Repository secrets:

- `MACOS_CERTIFICATE_P12`: the existing base64-encoded Tonk signing identity and private key archive used for CLI releases.
- `MACOS_CERTIFICATE_PASSWORD`: its archive password.
- `TONK_PROVISIONING_PROFILE_BASE64`: base64-encoded Apple profile for `8WVKS2F24C.xyz.tonk`, authorizing that exact certificate and Associated Domains.

The signing script imports the identity into a temporary runner keychain and
restores/deletes it on exit. It requires a matching unexpired profile, embeds
it, enables `TonkDirectPasskeyEnabled`, and checks the signed team, leaf
certificate, associated-domain entitlements, embedded profile and app flag.
Missing credentials or a mismatched profile fail the signing stage.

This artifact is signed and provisioned, but is not notarized or published as a
release. Direct passkeys are enabled for `https://tonk.network`; that domain
must serve an AASA `webcredentials` entry for `8WVKS2F24C.xyz.tonk`. CI does not
perform a real passkey ceremony or prove recovery of an existing account.
