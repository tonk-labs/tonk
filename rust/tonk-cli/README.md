# tonk

A local-only CLI for reading and writing tonk facts via asserted-notation.

`tonk` is the headless companion to tonk-ui, without a browser: it operates on
the selected **spot** — a named fact store resolved through a central
registry, so the CLI works from any directory. The mutating verb is `eval`,
which runs a notation document through the analyze → query → plan → commit
pipeline. The other subcommands are read-only introspection, one-shot setup,
sync, and sharing helpers. The crate also exposes a small library surface
(`tonk::eval`, `tonk::site`, …) so integration tests and SDK consumers can
drive the same code paths as the binary.

## Usage

```sh
# Link an account first: creating a spot bootstraps a repository, which
# requires one. Once per machine.
tonk account link
# Create a spot (stored canonically, e.g. ~/Library/Application Support/tonk/spots/garden).
tonk spot new garden
# Use an existing spot in another project directory:
tonk use garden

# Evaluate a notation document: inline, from a file, or piped. A bare
# `<concept>:` is a query, so the concept has to be declared already —
# `concept` is built in; your own come from `tonk concept add` below.
tonk eval -c 'concept:'
tonk eval ./doc.notation
cat doc.notation | tonk eval -
tonk eval ./doc.notation --format json --quiet

# Inspect the branch.
tonk schema       # every named attribute + concept as re-submittable notation
tonk concept ls   # user-defined concepts: name<TAB>description
tonk view ls      # entities with a text/html claim: name<TAB>entity<TAB>bytes
tonk guide        # baked-in asserted-notation reference (also: guide notation|views|all)

# Argument-based data verbs — a constrained front-end over `eval`.
# Dialog vocabulary: you assert claims and retract them. A retraction
# is itself an assertion invalidating an old claim, not a delete.
tonk schema habit                             # one concept's schema, as re-submittable notation
tonk assert habit --help                      # the concept's real flags (fields, types, required)
tonk assert habit --name "Run" --target "5k"  # mint a new instance (typed flags from the branch schema)
tonk assert habit <entity> --target "10k"     # assert superseding claims on an existing instance
tonk query habit                              # every instance (add --json for machine output)
tonk query habit <entity>                     # one instance
tonk retract habit <entity> --field target    # retract one field (a many field loses every value)
tonk retract habit <entity>                   # retract the whole instance

# Authoring — schema, views, and the space home.
tonk concept add habit --attr name:text:one   # anchored concept + typed attributes
tonk view add habit --template '<b>{name}</b>'  # declarative view (auto-surfaces an unset home)
tonk home habit                               # put habit's directory on the space home

# CSV transfer over the main branch.
tonk export --out data.csv
tonk import data.csv

# Remotes and sync.
tonk remote add prod https://access.example.com \
  --revocation-url https://artifacts.example.com/revocations
tonk remote set-upstream prod
tonk push
tonk pull
tonk status       # synced | ahead | behind | diverged | no-upstream

# Link this native profile to a passkey-backed account.
tonk account status
tonk account link --name workstation
tonk account logout

# Delegate access to the space.
tonk invite                    # audience-open: anyone holding it can claim
tonk invite --remote prod      # selected remote must carry a revocation relay
tonk invite --recipient-root did:key:z6Mk... # seed-free targeted invite
tonk invite --no-remote        # embed none; the claimer wires an upstream by hand
tonk join 'https://...#invite' --name garden
```

## Telemetry

Release builds send one anonymous `cli_command_run` event per
invocation (command name, duration, exit class — never document
content, paths, or URLs). `tonk telemetry off`, `DO_NOT_TRACK=1`, or
`TONK_TELEMETRY=0` disable it; builds without a baked-in key send
nothing. Full inventory: [`docs/telemetry.md`](../../docs/telemetry.md).

## How it works

### Spots and sites

A **spot** is a named entry in `spots.json`, a registry kept under the
platform data dir (`~/Library/Application Support/tonk/` on macOS). Each
entry points at a **site**: the working directory holding the actual dialog
repository (`main`, opened on the `main` branch — multi-branch and multi-repo
workflows are intentionally not exposed). Sites live canonically under
`spots/<name>/`, or anywhere you like via `tonk spot new --site <path>`.
Commands resolve which spot to use as `--spot` > `TONK_SPOT` > the nearest
directory bound by `tonk use <name>`, then open its central site. There is no
machine-global fallback. Parallel sessions in separate directories therefore
hold their own spot without repeating a flag. The directory is only a key into
the registry — no site data or pointer file is stored there. `tonk spot unbind`
removes an exact binding. `spots.json` is plain JSON, so any application can
read the registry without going through the CLI.

To adopt an existing `.tonk/` directory (from a pre-spots checkout, or
somewhere you keep data outside the canonical store) as a spot, point
`--site` at it: `tonk spot new proj --site ~/proj/.tonk`. The local identity
is a shared profile (`tonk identity` prints its DID; `--reset` mints a fresh
one).

### The eval pipeline

`tonk eval` resolves its source (inline `-c`, a path, `-` or piped stdin),
opens the site, and drives `tonk_evaluator::evaluate` against the `main`
branch's transaction. The evaluator analyzes the notation, runs the synthesized
queries, stages mutations, and fires installed effects, yielding a transaction
that tonk commits. The response is rendered as YAML notation (default) or JSON;
`--quiet` drops the matches section and emits only the envelope. Exit codes are
distinct per failure stage (`ParseError`, `AnalyzeError`, `CommitError`,
`IoError`) so agent harnesses can branch without parsing stderr.

When an upstream is configured, a committing eval is wrapped with an automatic
pull-before / push-after. `--no-sync` (or `TONK_NO_SYNC`) skips it; manual
`tonk push` / `tonk pull` stay available either way.

### Accounts

The CLI has at most one active remote account. Every `tonk account link`
requires a fresh browser/passkey handoff; switching accounts is logout followed
by link. Historical certificates remain available for local spot writes, but
only the latest active attachment can authorize a remote request.

`tonk account logout` commits locally first, so it works offline. Existing
spots remain readable and editable, while fetch, pull, push, account sync, and
other access-service requests are denied before HTTP. Logout queues a
signed, generation-specific detach intent; the device list may remain stale
until a later account operation reaches the provider and flushes that outbox.

A detach intent is signed once and never edited, so a client-error response
is a permanent verdict on it: an unknown attachment, a payload the service
disagrees with, or a service with no detach route can never accept that
intent, and the CLI drops it rather than retrying forever. Timeouts, rate
limits, and server errors are retried instead, and while one is still
queued for a provider, linking to that same provider is refused because its
one-active-generation rule would reject the activation. `tonk account link
--abandon-detach` drops those undelivered intents and links anyway; the
earlier device can stay listed until `tonk account revoke` removes it.

Detach is not revocation. It hides the exact attachment and permits a later
fresh handoff without publishing an immutable revocation. `tonk account revoke
<DEVICE_DID>` permanently revokes the selected grant, which can never be
reactivated. Use `tonk identity --reset` only for destructive local identity
rotation.

| State | Local query/edit/commit | Remote sync |
| --- | --- | --- |
| Logged out | allowed | denied before HTTP |
| Active account, spot delegated to it | allowed | allowed with only that account's grant |
| Active account, spot delegated only to another account | allowed | denied before HTTP |

### Sync and sharing

`push` / `pull` are fast-forward sync over `Branch::push()` / `Branch::pull()`,
with errors that name the upstream-not-configured and non-fast-forward cases.
`status` classifies the local branch against its upstream without merging.

Remotes are UCAN-S3 access services registered on the repository's meta branch.
Their immutable-artifact relay is separate metadata supplied with
`tonk remote add --revocation-url`; it is never inferred from the access host.
`tonk invite` mints a UCAN delegation chain over the repo and prints an
audience-open invite URL (anyone holding it can claim by redelegating from the
embedded ephemeral key); `tonk join` claims one into a fresh spot
(`tonk join <url> --name <spot>`).

A bare `tonk invite` resolves the repo's remote, builds the link on that
remote's origin, and embeds it so the claimer auto-configures the same access
service. `--remote <NAME>` picks one when several are registered; `--no-remote`
mints without one. A selected remote without relay metadata remains listable
but invitation minting fails with an explicit configuration error.

## Built on

`tonk` drives documents through `tonk-evaluator` (analyze → compile → evaluate),
parses with `tonk-notation`, reads schema types from `tonk-schema`, builds
invites with `tonk-invite`, and talks to dialog repositories, storage, UCAN
credentials, and the UCAN-S3 remote through the `dialog-*` crates.
