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

At the core of Tonk is [Dialog DB](https://github.com/dialog-db/dialog-db), an embeddable, local-first database that stores everything as facts: semantic triples of (entity, attribute, value). Facts are never deleted, only superseded or retracted. This append-only, content-addressed design means prolly-tree indexes depend only on data, not insertion order. Replicas sync deterministically without conflicts.

You interact with Dialog through two primitives:

- **Concepts** group related facts into structures. Define a `Person` with a name, location, and photo. Then create a `ClubMember` concept that reuses name and photo but leaves out location — multiple views over the same data.
- **Rules** derive new concepts from existing facts. "A `FamilyMember` is any `Person` whose last name matches another and who shares the same home location."

Add new rules, new concepts, or extend existing ones. Declaratively express what data you expect and it works without a single migrations.

We encourage you to check out the repository to learn more.

# What's in This Repo

This is a Rust workspace (with some JS/WASM packages) where we are implementing our early experimentations on the Tonk substrate.

> ⚠️ This repo is heavily in flux, and not meant to be friendly for public access or contributions. If you would like to try some of our experiments, see the [Released Experiments](#released-experiments) section below!

### Rust Crates

| Crate                   | Purpose                                                                                             |
| ----------------------- | --------------------------------------------------------------------------------------------------- |
| **tonk-cli**            | CLI tool (`tonk` binary) — the primary interface for managing spaces, concepts, instances, and sync |
| **tonk-space**          | Core space primitives: operators, delegation, ownership, storage                                    |
| **tonk-common**         | Cross-platform utilities (logging, etc.)                                                            |
| **tonk-blobs**          | Content-addressed blob storage (filesystem + IndexedDB)                                             |
| **tonk-access-service** | Cloudflare Worker that authorizes S3/R2 access via UCAN                                             |
| **tonk-ui**             | Leptos-based web frontend                                                                           |
| **tonk-worker**         | Browser service worker (WASM) for offline web support                                               |
| **tonk-core**           | Core library (in progress)                                                                          |

### JavaScript Packages (`packages/`)

| Package   | Purpose                                               |
| --------- | ----------------------------------------------------- |
| **core**  | WASM-compiled core library for browsers (sync engine) |
| **relay** | Communication relay                                   |

# Released Experiments

## Carry

[Carry](https://github.com/tonk-labs/carry) is a local-first semantic database for humans and machines in the form of a CLI tool. It allows you to assert, query, and manage structured data in a local Dialog DB repository.

This is useful for a few things, but one that's been working well for us is as a persistent memory layer for local LLM-driven workflows. You can read more about the use cases in [the documentation](https://tonk-labs.github.io/carry/philosophy.html).

Carry is synced from this monorepo and published separately.

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
