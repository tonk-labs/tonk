# tonk

A local-first CLI for reading, writing, synchronizing, and sharing Tonk facts
via asserted notation.

`tonk` is the headless companion to tonk-ui, without a browser: it operates on
the selected **space** — a named fact store resolved through a profile-local
registry, so the CLI works from any directory. The mutating verb is `eval`,
which runs a notation document through the analyze → query → plan → commit
pipeline. The other subcommands are read-only introspection, one-shot setup,
sync, and sharing helpers. The crate also exposes a small library surface
(`tonk::eval`, `tonk::site`, …) so integration tests and SDK consumers can
drive the same code paths as the binary.

## Usage

```sh
# Create a space (stored canonically, e.g. ~/Library/Application Support/tonk/spots/garden).
tonk space new garden
# Use an existing space in another project directory:
tonk use garden

# Evaluate a notation document: inline, from a file, or piped.
tonk eval -c 'person:'
tonk eval ./doc.notation
cat doc.notation | tonk eval -
tonk eval -c 'person:' --format json --quiet

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
tonk get habit <entity>                       # one instance
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

# Manage several isolated native account profiles.
tonk account status
tonk account add --label personal --name workstation
tonk account add --label work
tonk account list
tonk account use personal
tonk account login
tonk account sync
tonk account logout

# Delegate access to the space.
tonk share                    # audience-open: anyone holding it can claim
tonk share --remote prod      # selected remote must carry a revocation relay
tonk share --recipient-root did:key:z6Mk... # seed-free targeted invite
tonk share --no-remote        # embed none; the claimer wires an upstream by hand
tonk join 'https://...#invite' --name garden

# One lifecycle inventory, offline by default.
tonk space list
tonk space list --all --json
tonk space list --refresh
tonk account spaces pull did:key:z6Mk... --name garden
tonk space archive did:key:z6Mk... --yes
```

## Telemetry

Release builds send one anonymous `cli_command_run` event per
invocation (command name, duration, exit class — never document
content, paths, or URLs). `tonk telemetry off`, `DO_NOT_TRACK=1`, or
`TONK_TELEMETRY=0` disable it; builds without a baked-in key send
nothing. Full inventory: [`docs/telemetry.md`](../../docs/telemetry.md).

## How it works

### Spaces and sites

A **space** is a named entry in the internal `spots.json` registry kept under the
platform data dir (`~/Library/Application Support/tonk/` on macOS). Each
entry points at a **site**: the working directory holding the actual dialog
repository (`main`, opened on the `main` branch — multi-branch and multi-repo
workflows are intentionally not exposed). Sites live canonically under
`spots/<name>/`, or anywhere you like via `tonk space new --site <path>`.
Commands resolve which space to use as `--space`/`--spot` >
`TONK_SPACE`/`TONK_SPOT` > the nearest
directory bound by `tonk use <name>`, then open its central site. There is no
machine-global fallback. Parallel sessions in separate directories therefore
hold their own space without repeating a flag. Equal canonical and legacy
environment values are accepted; conflicting values fail before opening a
repository. The directory is only a key into the registry — no site data or
pointer file is stored there. `tonk space unbind`
removes an exact binding. `spots.json` is plain JSON, so any application can
read the registry without going through the CLI.

To adopt an existing `.tonk/` directory (from a pre-registry checkout, or
somewhere you keep data outside the canonical store) as a space, point
`--site` at it: `tonk space new proj --site ~/proj/.tonk`. The local identity
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

### Accounts and profiles

One installation can retain several native profiles. Each profile owns a
distinct Dialog identity, account session and lock, account repository, space
registry, canonical space directory, deployment defaults, and credential
store. `tonk account add --label <name>` creates a profile, `account use`
selects one locally without a network request, and `account login` signs a
rooted profile back into its immutable account root. `account link` remains a
compatibility spelling: it creates or resumes an unrooted profile, and logs a
rooted selected profile back into that same account.

Directory bindings carry both the profile and the space name. A directory
bound to profile A's `garden` continues to open A even when B is selected;
explicit `--space garden` (or compatibility `--spot`), `TONK_SPACE` (or
`TONK_SPOT`), new spaces, joins, pulls, and account commands use the selected
profile. Different profiles may therefore
both have a space named `garden` without sharing state or authority.

After account login, Tonk asks that ceremony deployment for
`/.well-known/tonk`. A matching typed response supplies that profile's default
content access endpoint and revocation relay. Discovery failure leaves login
successful and reports `sync defaults: pending`; it never silently substitutes
a production service. Existing custom upstreams always win.

Interrupting `tonk account add` or `link` leaves the provisional profile and
handoff recorded so the next run resumes both. A link token is one-time, so a
service that refuses to reissue it has ended that handoff and not this
profile's ability to link: the next run
takes the completed grant if the browser approved in the meantime, and
otherwise prints a fresh URL rather than re-offering a spent token.

`tonk account logout` commits locally first, so it works offline. Existing
spaces remain readable and editable through their owning profile, while
fetch, pull, push, account sync, and
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

Account selection and logout are not revocation. Selection only changes the
default local profile. Detach hides the exact attachment and permits a later
fresh handoff without publishing an immutable revocation. `tonk account revoke
<DEVICE_DID>` permanently revokes the selected grant, which can never be
reactivated. Use `tonk identity --reset` only for destructive local identity
rotation; it refuses while the selected profile still owns registered spaces.

Profiles are an operational isolation boundary inside supported Tonk CLI
paths, not encryption or an OS-user security boundary. Another process running
as the same user may read the unencrypted local files. Remote services enforce
the signed delegation chain; Tonk prevents accidental use of profile B's grant,
provider, deployment, or account repository for profile A's space.

| State | Local query/edit/commit | Remote sync |
| --- | --- | --- |
| Logged out | allowed | denied before HTTP |
| Active account, space delegated to it | allowed | allowed with only that account's grant |
| Active account, space delegated only to another account | allowed | denied before HTTP |

### Space inventory and lifecycle

`tonk space list` joins local registration, signed account membership,
transport, synchronization, authority, and device-local visibility without
collapsing them into one “synced” flag. The default read is offline and does
not mutate repositories. `--refresh` may fetch provider inventory and remote
heads, but never pulls, merges, provisions, pushes, archives, or changes a
directory binding. `--all` includes suppressed, archived, ambiguous, and
orphaned rows; `--json` emits the stable version-1 lifecycle schema.

“Remove from this device” and “Archive from account” are deliberately separate:

| Operation | Local registration/data | Account discovery | Authority or remote/peer data |
| --- | --- | --- | --- |
| `tonk space rm` | hides locally; may delete local bytes | remains active | unchanged |
| explicit `account spaces pull` | mounts locally and clears this profile's suppression | unchanged | unchanged |
| `tonk space archive <SUBJECT>` | unchanged | permanently archived for this subject | unchanged |

Archive writes a signed monotonic tombstone. It never means “delete
everywhere”: offline peers and already-held copies cannot be compelled to
erase data. A saved-access artifact is also not a content backup. Tonk reports
an exact `confirmedRevision` only after that tree was accepted by the content
remote; a remote URL alone is not evidence that current content is recoverable.

Canonical vocabulary is `space`, `--space`, `TONK_SPACE`, `account spaces`,
and `share`. For at least this compatibility release the visible aliases below
reach the identical handlers and telemetry operation:

| Compatibility spelling | Canonical spelling |
| --- | --- |
| `spot` | `space` |
| `--spot` | `--space` |
| `TONK_SPOT` | `TONK_SPACE` |
| `account spots` | `account spaces` |
| `invite` | `share` |

### Sync and sharing

`push` / `pull` are fast-forward sync over `Branch::push()` / `Branch::pull()`,
with errors that name the upstream-not-configured and non-fast-forward cases.
`status` classifies the local branch against its upstream without merging.

Remotes are UCAN-S3 access services registered on the repository's meta branch.
Their immutable-artifact relay is separate metadata supplied with
`tonk remote add --revocation-url`; it is never inferred from the access host.
`tonk share` mints a UCAN delegation chain over the repo and prints an
audience-open invite URL (anyone holding it can claim by redelegating from the
embedded ephemeral key); `tonk join` claims one into a fresh space
(`tonk join <url> --name <space>`).

A bare `tonk share` resolves the repo's remote, builds the link on that
remote's origin, and embeds it so the claimer auto-configures the same access
service. `--remote <NAME>` picks one when several are registered; `--no-remote`
mints without one. A selected remote without relay metadata remains listable
but invitation minting fails with an explicit configuration error.

## Built on

`tonk` drives documents through `tonk-evaluator` (analyze → compile → evaluate),
parses with `tonk-notation`, reads schema types from `tonk-schema`, builds
invites with `tonk-invite`, and talks to dialog repositories, storage, UCAN
credentials, and the UCAN-S3 remote through the `dialog-*` crates.
