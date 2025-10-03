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

Tonk is a containerized format, host environment and protocol for multiplayer software you keep and share like files.

Every file contains both its application and its data, is designed to work across any connection, on
any machine, last forever, and remain under user control.

## What is Tonk?

Tonk represents a "credibly neutral platform" for building and sharing local software. It embodies
principles of malleable software (as easy to change as it is to use) and user ownership (people
retain control of their tools and data).

### Key Features

- **Virtual File System**: Document-based storage backed by Automerge CRDTs with real-time sync
- **Bundle Format**: Self-contained .tonk files containing both application and data
- **Web Host Environment**: A simple website shell for running web bundles packaged in .tonk files
- **WASM Core**: High-performance Rust implementation that runs in browsers and Node.js
- **Real-time Sync**: Automatic peer-to-peer synchronization via WebSockets
- **Offline-First**: Works without internet, syncs when reconnected

## Quick Start

> ⚠️ Tonk is under heavy development. See the [quickstart guide](docs/src/quickstart.md) for setup
> instructions.

```bash
# Build core
cd packages/core-js
pnpm install && pnpm build

# Try the latergram example
cd examples/latergram
pnpm install && pnpm bundle create

# Run host-web to load the .tonk file
cd packages/host-web
pnpm dev
```

## Resources

- [Documentation](https://tonk-labs.github.io/tonk)
- [Website](https://tonk.xyz)
- [GitHub](https://github.com/tonk-labs/tonk)
- [Community](https://discord.gg/cHqkYpRE)

## License

MIT © Tonk Labs
