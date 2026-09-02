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

Tonk is a data substrate: a software environment as easy to change as it is to use. Where stacks are rigid and vertically integrated, substrates are malleable and horizontally connected. You modify software in the context of its use, not through a separate process. In a substrate, software truly becomes yours.

Substrates are essential when LLMs make code generation abundant and personal software becomes practical. We won't get there by speeding up the same engineering-heavy processes of traditional software practice. We need a new surface; one interoperable and owned by the person running it. Tonk is that surface.

## Dialog DB: The Foundation

At the core of Tonk is [Dialog DB](https://github.com/dialog-db/dialog-db), an embeddable, local-first database. Dialog stores everything as claims — semantic triples of (entity, attribute, value). Claims are never deleted, only superseded or retracted. This append-only, content-addressed design makes it straightforward to sync data across devices and collaborators without conflicts.

The primary interface is claims themselves — assert data, query it, retract it. On top of that, you can optionally define:

- **Concepts** to group related claims into queryable structures. Define a `Person` with a name, location, and photo. Then create a `ClubMember` concept that reuses name and photo but leaves out location — multiple views over the same data.
- **Rules** to derive new concepts from existing claims. "A `FamilyMember` is any `Person` whose last name matches another and who shares the same home location."

Add new rules, concepts, or extend existing ones as your needs evolve. No migrations required.

Check out the [Dialog DB repository](https://github.com/dialog-db/dialog-db) to learn more about how it works under the hood.

## The `tonk` CLI

`tonk` is a local-first command-line companion for reading and writing Tonk data without a browser. It operates on a *space* — a named Dialog repository in a central registry. `tonk space use <name>` binds the current directory to a space without copying data there; `--space` and `TONK_SPACE` override that binding for one invocation or process. The CLI speaks the same asserted-notation as the rest of the substrate: you assert claims, query them, and define concepts and rules with `tonk eval`, plus read-only introspection (`tonk schema`, `tonk concept ls`, `tonk view ls`) and a built-in notation reference (`tonk guide`).

### Install

```sh
curl -fsSL https://github.com/tonk-labs/tonk/releases/latest/download/install.sh | sh
```

This detects your platform, verifies the download against the release checksums, and installs the `tonk` binary to `/usr/local/bin` (or `~/.local/bin` if that is not writable). Apple Silicon macOS and x86_64 Linux are published. If `tonk` isn't found afterward, the install location isn't on your `PATH` — add it, e.g. `export PATH="$HOME/.local/bin:$PATH"`. Set `TONK_INSTALL_DIR` to install elsewhere.

To install the pre-release channel instead, set `TONK_CHANNEL=staging`:

```sh
curl -fsSL https://github.com/tonk-labs/tonk/releases/latest/download/install.sh | TONK_CHANNEL=staging sh
```

You can also download a `tonk-<platform>.tar.gz` directly from the [releases page](https://github.com/tonk-labs/tonk/releases) and extract the `tonk` binary onto your `PATH`. Newly published macOS binaries are Developer ID signed and notarized by Apple.

### Update

```sh
tonk update
```

This upgrades an install made by the install script on its recorded
channel. A default install follows the stable GitHub release; an install
made with `TONK_CHANNEL=staging` follows the rolling staging release. It
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
the installer with or without `TONK_CHANNEL=staging` to deliberately
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
tonk schema                # every named attribute + concept on the branch
tonk guide                 # built-in asserted-notation reference
```

# What's in This Repo

This is a Rust workspace where we are implementing our early experimentations on the Tonk substrate.

> ⚠️ This repo is heavily in flux, and not meant to be friendly for public access or contributions. If you would like to try it, the [`tonk` CLI](#the-tonk-cli) is the easiest entry point.

### Rust Crates

| Crate                   | Purpose                                                                                             |
| ----------------------- | --------------------------------------------------------------------------------------------------- |
| **tonk-space**          | Core space primitives: operators, delegation, ownership, storage                                    |
| **tonk-common**         | Cross-platform utilities (logging, etc.)                                                            |
| **tonk-blobs**          | Content-addressed blob storage (filesystem + IndexedDB)                                             |
| **tonk-access-service** | Cloudflare Worker that authorizes S3/R2 access via UCAN                                             |
| **tonk-ui**             | Leptos-based web frontend                                                                           |
| **tonk-worker**         | Browser service worker (WASM) for offline web support                                               |
| **tonk-core**           | Core library (in progress)                                                                          |

# Adjacent Projects

Tonk is pluralist by design and built to work alongside other protocols building on open technology. We think that is the best way to accomplish our mission. Here are some projects that are adjacent to us and think you should check out.

- [Ink and Switch](https://www.inkandswitch.com/)
- [Automerge](https://automerge.org/)
- [Common Tools](https://common.tools/)
- [Iroh](https://www.iroh.computer/)

If you are a friend or adjacent project and would like to be listed here, please reach out!

# Resources

- [Website](https://tonk.xyz)
- [Roadmap](https://tonk.xyz/roadmap)
- [Discord](https://discord.com/invite/hBPQ9xPWF7)

## License

MIT © Tonk Labs
