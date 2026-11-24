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

Tonk is a system for co-computing over live data in a programmable environment.
People and programs share a **space** through links that grant capabilities
such as reading or writing. A space holds its data, the schemas that describe
it, the views that show and edit it, its rules, and a signed history of changes.
Local copies can work offline and sync when they reconnect.

## Typical uses

- Automate workflows with state machines that agents run and people operate through views.
- Reuse a workflow as a template with different credentials.
- Build internal tools that agents and teammates use together.
- Share an application with a small group through a link.
- Coordinate multiple agents through shared tasks and messages.
- Run applications on remote devices that work offline and sync later.

## Web

[Open Tonk](https://tonk.network) to create a space or open one someone has
shared with you.

## MCP

Connect an agent to `https://tonk.network/mcp`. Sign in to Tonk and choose
which spaces the agent can read or write. For example, an agent with write
access can update a task that teammates see on their board.
[Add Tonk to Claude](https://claude.ai/customize/connectors?modal=add-custom-connector&connectorName=Tonk&connectorUrl=https%3A%2F%2Ftonk.network%2Fmcp)
or follow the [MCP setup guide](https://tonk-labs.github.io/docs/mcp.html).

## Desktop

Tonk Desktop is a native macOS workspace for spaces and agents.
[Download Tonk Nightly for Apple Silicon](https://github.com/tonk-labs/tonk/releases/download/tonk-desktop-dev/Tonk-macos-arm64.zip)
(macOS 15+), unzip it and move **Tonk Nightly.app** to **Applications**.
Nightly connects to `tonk.foundation` by default.

## CLI

The `tonk` CLI reads and writes space data from the terminal. Install it on
Apple Silicon macOS or x86_64 Linux, then run `tonk help tutorial` for a walkthrough.

```sh
curl -fsSL https://github.com/tonk-labs/tonk/releases/latest/download/install.sh | sh
```

Create a local space in a fresh working directory, then write and query a task.

```sh
tonk space new garden
tonk concept add task --field title:text:one --description "A task to do"
tonk eval -c 'task!: {title: Water the garden}'
tonk query task
```

## Resources

- [Website](https://tonk.xyz)
- [Updates](https://tonk.xyz/updates)
- [Defining the common primitive](https://tonk.xyz/updates/defining-the-common-primitive)
- [Discord](https://discord.com/invite/hBPQ9xPWF7)
- For agents only: [Tonk guide](https://tonk.network/guide/) and [llms.txt](https://tonk.xyz/llms.txt).

## License

[MIT](LICENSE) © Tonk Labs

## Development

From the repository root, use [Nix](https://nixos.org/download/) with flakes
enabled to start the web app and its local access service.

```sh
ACCESS_STATE_DIR="$PWD/.tonk-dev/access" nix develop . -c dev:web
```

Open [localhost:8080](http://localhost:8080). The state directory preserves the
local service's identity and data between runs.

```sh
nix develop . -c test:native:debug
nix develop . -c test:web:debug
nix develop . -c lint
```

Run `nix develop` for the full command list. See the
[architecture and crate map](ARCHITECTURE.md) for the implementation and the
[desktop build instructions](desktop/README.md#build) for macOS development.
