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

Tonk is a containerized format, host environment and protocol for multiplayer software you keep and
share like files.

Every file contains both its application and its data, is designed to work across any connection, on
any machine, last forever, and remain under user control.

## What is Tonk?

Tonk represents a credibly neutral platform for building and sharing local software. It embodies
principles of malleable software (as easy to change as it is to use) and user ownership (people
retain control of their tools and data).

### Key Features

- **Virtual File System**: Document-based storage backed by Automerge CRDTs with real-time sync
- **Bundle Format**: Self-contained .tonk files containing both application and data
- **Host-Web Runtime**: Complete browser runtime environment for loading and executing .tonk
  applications
- **WASM Core**: High-performance Rust implementation that runs in browsers and Node.js
- **Real-time Sync**: Automatic peer-to-peer synchronization via WebSocket relay servers
- **Offline-First**: Applications work without internet, sync when reconnected

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

## Thanks

Special thanks to our external friends and supporters:

- **[Automerge](https://automerge.org/)** - A library of data structures for building collaborative
  applications.
  - [Alex Good](https://patternist.xyz/) for supporting our queries and collaborating with us on the
    WASM implementation.
- **[Ink & Switch](https://www.inkandswitch.com/)** - An independent research lab exploring the
  future of tools for thought.
  - [Peter Van Hardenburg](https://www.pvh.ca/) for sharing his
    [trail-runner](https://github.com/pvh/trail-runner) project with us, which the host web package
    is heavily based on.
  - [Chee](https://chee.party/) for sharing papers and imagining with us what's possible.
- **[Common Tools](https://common.tools/)** A new fabric for computing.
  - [Alex Komoroske](https://www.komoroske.com/) for being a nexus of ideas. He publishes an
    incredible weekly update.
- **[Grjte](https://grjte.sh/)** - For charting the frontier with us every step of the way.
- **[Boris Mann](https://bmannconsulting.com/)** - For being a super connector and arguing with us
  about files.

<br/>
There are many more who have expressed their support and contributed something meaningful and we
can't possibly list everyone, but know that we are so grateful to work with together with you.
❤️

## License

Simplicity and freedom

MIT © Tonk Labs
