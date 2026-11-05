# Tonk

```
                                    .-            ::.
                                    -##.   ..     +@@@+
                              -:    ###-  *##
                              +##+  ###* *##%
                              ########%*###*   +*%
                        *###*:+##=*..:=####*++*#@
                          ######*==-.----%#**##%..:                        :=
                          .:=##-=:*#===*-##%@@*#@-                      ::.:=.
                      =#%%%#%%***##+#-*-##@@@@@              -..      .---++
                      =@%####@@@%%=*#@#*@@@@                -:      :--++
                        -+****#*#%@@@@%@@@-                  :.:   =::     .....
                      ++*#%*-+++*#@@@@@@@.                .:::-+:.#+   ..  .::::.
                      -%     =+#@   . %@@@*                     =##@        .
      ..                   *#%        .%@@+                       =###
    ##@%#*%                             +@%+                           #=
    :*#%%-:@                              @#=                            +-
      +@@@##@                               %+-                             *
                                        ::: :+=-     ...:-==.
                                    ......:-++:  ::.::::=+
                                    ++=-::+%@@#:--:...:-=+
                            .--           @@%%#*+-:::-=+-
                        .=++=--=+-     -==#%%####****#-
                    .%@@@      +%+ ..-*%#
              -=    =@@@@#       --:+%-     .    +@@%  :+#%*-:.
          :+-==+=   =@@@       ==+%  *#     +@+%@@@@ #=    %#****@:
          ###*=*  ..       .- .*=*. *        -@@#.-=#+      .**#*@+
          :=-:=@.+=*   ::= :..===+= #-:.     .#-...++ .=....   =@+
                %%%.   . -+    @*---%=*@@*:  ......-=  ==...-+  +%
          -++    .+%=*::+.*-    ##*@+--#@@@+.....=:.==. *-*+%  +%+-
```

Tonk is a system for co-computing over live data in a programmable environment. It allows for the provisioning and syncing of **space** which is a data store that people and programs read and write through links with object capabilities attached. Each client that opens the
link keeps a local copy of the space and syncs its changes with the other copies. A space contains its data, the schemas that describe the data, views (web pages that show and edit the data), rules, and a signed history of all changes.

## System Components

[ARCHITECTURE.md](ARCHITECTURE.md) describes six different system components,
its technical basis, and the crates that implement it.

## Typical uses

- State machines for programmable agentic automations, views can be used for human operators to interact
- These state machines can become templates and then quickly run with pluggable credentials, for on-demand, situated automations.
- Cost-efficient, agent-first internal team tools.
- Low-friction distribution of software that serves small groups of people.
- Multi-agent coordination control and messaging plane
- Edge-device or remote applications that must work offline and sync later.

For background, see
[Defining the common primitive](https://tonk.xyz/updates/defining-the-common-primitive).

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

## The `tonk` CLI

`tonk` is a local-first command-line companion for reading and writing Tonk data without a browser. It operates on a *space* — a named Dialog repository in a central registry. `tonk space use <name>` binds the current directory to a space without copying data there; `--space` and `TONK_SPACE` override that binding for one invocation or process. The CLI speaks the same asserted-notation as the rest of Tonk: you assert claims, query them, and define concepts and rules with `tonk eval`, plus read-only introspection (`tonk show`, `tonk concept`, `tonk view`) and built-in guides (`tonk help -g`, e.g. `tonk help notation`).

### Install

```sh
curl -fsSL https://github.com/tonk-labs/tonk/releases/latest/download/install.sh | sh
```

This detects your platform, verifies the download against the release checksums, and installs the `tonk` binary to `/usr/local/bin` (or `~/.local/bin` if that is not writable). Apple Silicon macOS and x86_64 Linux are published. If `tonk` isn't found afterward, the install location isn't on your `PATH` — add it, e.g. `export PATH="$HOME/.local/bin:$PATH"`. Set `TONK_INSTALL_DIR` to install elsewhere.

To install the pre-release channel instead, set `TONK_CHANNEL=staging`:

```sh
curl -fsSL https://github.com/tonk-labs/tonk/releases/latest/download/install.sh | TONK_CHANNEL=staging sh
```

To follow the latest build of `main`, set `TONK_CHANNEL=dev`. It moves with every merge, so it can read and write whatever `main` does:

```sh
curl -fsSL https://github.com/tonk-labs/tonk/releases/latest/download/install.sh | TONK_CHANNEL=dev sh
```

You can also download a `tonk-<platform>.tar.gz` directly from the [releases page](https://github.com/tonk-labs/tonk/releases) and extract the `tonk` binary onto your `PATH`. Newly published macOS binaries are Developer ID signed and notarized by Apple.

### Update

```sh
tonk update
```

This upgrades an install made by the install script on its recorded
channel. A default install follows the stable GitHub release; an install
made with `TONK_CHANNEL=staging` follows the rolling staging release, and
one made with `TONK_CHANNEL=dev` the rolling build of `main`. It
verifies the download against the release checksums, checks the new
binary runs, and only then replaces the old one — so a failed update
leaves your working `tonk` untouched. On macOS it preserves the published
Developer ID signature while replacing the executable.

`tonk` checks for new releases once a day and prints a one-line notice
on stderr when one exists. Turn that off with `tonk update
--disable-check` (or `TONK_NO_UPDATE_CHECK=1`), and back on with `tonk
update --enable-check`. It never runs in CI.

Check what you have with `tonk --version`. `tonk update` and the daily
release check follow the channel in the matching install receipt; the
ambient `TONK_CHANNEL` value does not switch an existing binary. Rerun
the installer with or without `TONK_CHANNEL=staging` (or `dev`) to deliberately
switch that installation's channel.

If `tonk` was installed some other way, `tonk update` says so instead
of interfering: use `npm i -g @tonk/cli` for a stable npm install,
`npm i -g @tonk/cli@next` for a prerelease npm install, or your flake for
a nix one. Re-running the install command also still works:

```sh
curl -fsSL https://github.com/tonk-labs/tonk/releases/latest/download/install.sh | sh
```

### Quick start

```sh
tonk space new garden      # register a space and select it
tonk eval -c 'person:'     # run a notation document (inline, file, or piped)
tonk show                  # the schema: concepts and their fields
tonk help notation         # built-in asserted-notation reference
```

## Agents

An agent can [try Tonk locally](docs/try.md) on Apple Silicon macOS, with no
account or link. It can use Tonk through the `tonk` CLI (start with `tonk help agent`
and `tonk help tutorial`) or through an MCP connection. For the data model and
the code layout, read [ARCHITECTURE.md](ARCHITECTURE.md). The MCP contracts are
in [`mcp/AGENT-LINKS.md`](mcp/AGENT-LINKS.md), and the Claude Code plugin is
in [`plugins/tonk`](plugins/tonk).

# What's in This Repo

This repository contains the Tonk Rust workspace (sync and storage, the web
app, the CLI and the agent connections) and the macOS desktop app.

> ⚠️ This repo is heavily in flux, and not meant to be friendly for public access or contributions. If you would like to try it, the [`tonk` CLI](#the-tonk-cli) is the easiest entry point.

The crates and their roles are listed in
[ARCHITECTURE.md](ARCHITECTURE.md#crate-map).

## Developer setup

The toolchain comes from the Nix flake. Run commands inside its shell:

```sh
nix develop . -c dev:web          # web app with a local access service
nix develop . -c test:native:debug
nix develop . -c test:web:debug   # browser (wasm) tests, see docs/wasm-testing.md
nix develop . -c lint
nix develop . -c dev:desktop      # build and open Tonk Desktop
```

`nix develop` with no command opens the shell and lists every command.
Without Nix, see [`docs/wasm-tests-without-nix.md`](docs/wasm-tests-without-nix.md).

# Resources

- [Website](https://tonk.xyz)
- [Updates](https://tonk.xyz/updates)
- [For agents: llms.txt](https://tonk.xyz/llms.txt)
- [Discord](https://discord.com/invite/hBPQ9xPWF7)

## License

MIT © Tonk Labs
