# tonk

A local-only CLI for reading and writing tonk facts via asserted-notation.

`tonk` is the headless companion to tonk-ui, without a browser: it operates on
the selected **space** — a named fact store resolved through a central
registry, so the CLI works from any directory. The mutating verb is `eval`,
which runs a notation document through the analyze → query → plan → commit
pipeline. The other subcommands are read-only introspection, one-shot setup,
sync, and sharing helpers. The crate also exposes a small library surface
(`tonk::eval`, `tonk::site`, …) so integration tests and SDK consumers can
drive the same code paths as the binary.

## Usage

```sh
# Create a local space and bind this directory to it.
tonk space new garden
# Use an existing space in another project directory:
tonk use garden

# Every local replica, with the owner each space names.
tonk space list

# Sign in. Tonk holds one account at a time.
tonk account link
tonk account logout

# A local space can move into your account. Once it belongs to one, it stays.
tonk space link garden

# What your account's directory lists, and pulling one of those spaces here.
tonk account spaces list
tonk account spaces pull did:key:...

# Sharing never changes ownership: the invitee joins as a member.
tonk invite

# Evaluate a notation document: inline, from a file, or piped.
tonk eval -c 'person:'
tonk eval ./doc.notation
cat doc.notation | tonk eval -
tonk eval -c 'person:' --format json --quiet

# Inspect the branch.
tonk schema       # every named attribute + concept as re-submittable notation
tonk concept ls   # user-defined concepts: name<TAB>description
tonk view ls      # entities with a template claim: name<TAB>entity<TAB>model<TAB>bytes
tonk blob ls      # ingested blobs: entity<TAB>content-type<TAB>name
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
tonk remote add prod https://access.example.com
tonk remote set-upstream prod
tonk push
tonk pull
tonk status       # synced | ahead | behind | diverged | no-upstream

# Sign in to a passkey-backed account.
tonk account status
tonk account link --name workstation
tonk account logout

# Delegate access to the space.
tonk invite                    # audience-open: anyone holding it can claim
tonk invite --remote prod      # mint against a named remote
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

### Spaces and sites

A **space** is a named entry in `spots.json`, a registry kept under the
platform data dir (`~/Library/Application Support/tonk/` on macOS). Each entry
points at a **site**: the working directory holding the actual dialog
repository (`main`, opened on the `main` branch — multi-branch and multi-repo
workflows are intentionally not exposed). Sites live canonically under
`spots/<name>/`, or anywhere you like via `tonk space new --site <path>`.

A space either belongs to no account, or to exactly one. Which one is read
from the space itself — the founder row of the roster it carries on `main` —
so `tonk space list` can name the owner of a space you merely joined, and no
record beside the space can drift out of step with it:

```text
NAME                 OWNER                     ROLE
scratch (z6Mkq7vp)   -                         local
garden (z6Mk4e2b)    you (z6Mkccc1)            owner
roadmap (z6Mkf0aa)   Ada Lovelace (z6Mkbbb9)   member
```

Every name is paired with an abbreviation of its stable identifier, git's
`Name <email>` discipline: `NAME` carries the space's subject so the same
space is recognizable across devices that named it differently, and `OWNER`
carries the founder's account root. Like git's short hashes the abbreviation
lengthens when a listing holds an ambiguous prefix; `--json` prints the full
DIDs. `ROLE` is the roster row this installation can claim — `local` when the
space carries no roster at all, `owner` for a founder row, `member` for a
member row, `-` (`unlisted` in `--json`) when the roster names nobody you
are, `unknown` when it cannot be read. A roster that is readable but does not
add up — a row stamped with two roles, or a second founder — still lists, with
what was wrong reported alongside. The two ownership rules are: a local space
can move into your account; once a space belongs to an account it stays
there, and reaches other people through `tonk invite`.

`--json` emits version-two rows. Version two dropped the per-space `account`
tag and the `access` flag that went with it, and added `owner`, `ownerName`,
and `ownerIsYou` read from the roster.

Commands resolve `--space` > `TONK_SPACE` > the visible `--spot` / `TONK_SPOT`
compatibility aliases > the nearest directory bound by `tonk use <name>`.
There is no machine-global fallback, so parallel sessions in separate
directories hold their own space without repeating a flag. The directory is
only a key into the registry — no site data or pointer file is stored there.
`tonk space unbind` removes an exact binding. `spots.json` is plain JSON, so
any application can read the registry without going through the CLI.

To adopt an existing `.tonk/` directory (from a pre-spots checkout, or
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

### Accounts

The CLI holds at most one account at a time. `tonk account link` (also spelled
`login`) runs a browser/passkey handoff and records that account; signing in as
someone else is `tonk account logout` followed by `tonk account link`. Linking
an account never enrolls the spaces already on this device.

Creating a space while signed out stays offline. Creating while signed in
provisions and publishes only that new space. `tonk space link <space>` moves
one local-only space into your account: it keeps its name, site, and binding,
and gains hosting, retained authority, and a row in the account directory your
other devices pull from. Nothing is deleted along the way, so an interrupted
link leaves a working local space and can simply be re-run.

The signed-in account parameterizes account-service operations and nothing
else: creating a hosted space, linking, pulling the directory, listing and
revoking devices, deletion. Editing is unrestricted — every replica this
device holds opens, reads, and writes whether you are signed out, signed in,
or signed into an account other than the space's owner. Spaces that belong to
another account stay on disk and stay listed across a switch; the `OWNER`
column, not a refusal, is what says whose they are.

Enforcement happens where it is real: at the service boundary. Push and pull
present the space's own delegation chain and the access service accepts or
rejects it, so a sync you cannot do fails there rather than being pre-judged
here. The CLI relays that refusal verbatim — the service's own reason on its
own line — wrapped in the likeliest fix: signing into the owning account when
the space is somebody else's, `tonk account devices` when it is yours and this
device may have been revoked. The reason is the part that came from the
boundary that actually said no; the fix is this CLI's inference from local
state, and it can be wrong.

`tonk account spaces list` reads the signed remote directory of the account you
are signed in to, and `tonk account spaces pull <subject>` mounts one of those
spaces here as an owner or member replica.

`tonk space rm` removes only the local replica (and, unless `--keep-data` is
used, its local bytes). It does not remove signed account directory facts,
revoke memberships or invitations, deprovision hosting, delete remote objects,
or erase a peer's replica.

Interrupting `tonk account link` leaves the handoff recorded so the next run
resumes it. A link token is one-time, so a service that refuses to reissue it
has ended that handoff and not this profile's ability to link: the next run
takes the completed grant if the browser approved in the meantime, and
otherwise prints a fresh URL rather than re-offering a spent token.

`tonk account logout` commits locally first, so it works offline. Existing
spaces remain readable and editable, while fetch, pull, push, account sync, and
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
| Signed in, space owned by or shared with that account | allowed | allowed with only that account's grant |
| Signed in, space belonging to another account | allowed | rejected at the service boundary, with the sign-in fix named |

### Sync and sharing

`push` / `pull` are fast-forward sync over `Branch::push()` / `Branch::pull()`,
with errors that name the upstream-not-configured and non-fast-forward cases.
`status` classifies the local branch against its upstream without merging.

Remotes are UCAN-S3 access services registered on the repository's meta branch.
A revocation is an ordinary `ucan/revoke` invocation, so it goes to the access
service like everything else and a mint needs nothing extra. A remote may still
carry a separate artifact relay, supplied by hand with `tonk remote add
--revocation-url`; it is never inferred and never required.
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
