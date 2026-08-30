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
tonk space use garden

# Every local replica, with the owner each space names.
tonk space

# Sign in. Tonk holds one account at a time.
tonk account login
tonk account logout

# A local space can move into your account. Once it belongs to one, it stays.
tonk space link garden

# What your account's directory lists, and pulling one of those spaces here.
tonk account space
tonk account space pull garden      # a unique directory name
tonk account space pull did:key:... # exact when names collide

# Sharing never changes ownership: the invitee joins as a member.
tonk invite

# Evaluate a notation document: inline, from a file, or piped.
tonk eval -c 'person:'
tonk eval ./doc.notation
tonk eval interactive.notation --home todo # install the document and replace the home atomically
cat doc.notation | tonk eval -
tonk eval -c 'person:' --json --quiet

# Inspect the branch.
tonk show         # every named field + concept as re-submittable notation
tonk concept      # concepts this space defines
tonk view         # entities with a template claim
tonk blob         # ingested blobs
tonk help         # baked-in asserted-notation reference (also: help notation|views|all)

# Argument-based data verbs — a constrained front-end over `eval`.
# Dialog vocabulary: you assert claims and retract them. A retraction
# is itself an assertion invalidating an old claim, not a delete.
tonk show habit                               # one concept's schema and usage
tonk assert habit --help                      # the concept's real flags (fields, types, required)
tonk assert habit --name "Run" --target "5k"  # mint a new instance (typed flags from the branch schema)
tonk assert habit <entity> --target "10k"     # assert superseding claims on an existing instance
tonk query habit                              # every instance (add --json for machine output)
tonk show habit <entity>                      # one instance
tonk retract habit <entity> --field target    # retract one field (a many field loses every value)
tonk retract habit <entity>                   # retract the whole instance

# Authoring — schema, views, and the space home.
tonk concept add habit --field name:text:one  # anchored concept + typed fields
tonk view add habit --template '<b>{name}</b>'  # declarative view (auto-surfaces an unset home)
tonk view add habit --kind directory --template-file habit.html --home
tonk space home habit                         # put habit's directory on the space home

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
tonk account login --name workstation
tonk account logout

# Delegate access to the space.
tonk invite                    # audience-open: anyone holding it can claim
tonk invite --remote prod      # mint against a named remote
tonk invite --recipient-root did:key:z6Mk... # seed-free targeted invite
tonk invite --no-remote        # embed none; the claimer wires an upstream by hand
tonk join 'https://...#invite' --name garden
```

`view add` authors `detail` by default; `--kind` also accepts `directory`,
`label`, and `title`. A first detail or directory view auto-surfaces only while
the home is blank. `--home` is explicit replacement authority: it installs the
view and replaces the prior home with this one concept in the same transaction.

## Telemetry

Release builds send one anonymous `cli_command_run` event per
invocation (command name, duration, exit class — never document
content, paths, or URLs). `tonk telemetry off`, `DO_NOT_TRACK=1`, or
`TONK_TELEMETRY=0` disable it; builds without a baked-in key send
nothing. Full inventory: [`docs/telemetry.md`](../../docs/telemetry.md).

## How it works

### Spaces and sites

A **space** is a named entry in `spaces.json`, a registry kept under the
platform data dir (`~/Library/Application Support/tonk/` on macOS). Each entry
points at a **site**: the working directory holding the actual dialog
repository (`main`, opened on the `main` branch — multi-branch and multi-repo
workflows are intentionally not exposed). Sites live canonically under
`spaces/<name>/`, or anywhere you like via `tonk space new --site <path>`.

A space either belongs to no account, or to exactly one. Which one is read
from the space itself — the founder row of the roster it carries on `main` —
so `tonk space` can name the owner of a space you merely joined, and no
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

Commands resolve `--space` > `TONK_SPACE` > the nearest directory bound by
`tonk space use <name>`.
There is no machine-global fallback, so parallel sessions in separate
directories hold their own space without repeating a flag. The directory is
only a key into the registry — no site data or pointer file is stored there.
`tonk space unbind` removes an exact binding. `spaces.json` is plain JSON, so
any application can read the registry without going through the CLI.

To adopt an existing `.tonk/` directory (from an older checkout, or
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

The CLI holds at most one account at a time. `tonk account login` (also spelled
`login`) runs a browser/passkey handoff and records that account; signing in as
someone else is `tonk account logout` followed by `tonk account login`. Linking
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

`tonk account space` reads the signed remote directory of the account you are
signed in to. `tonk account space pull <name-or-subject>` mounts one of those
spaces here as an owner or member replica; a name works when it identifies one
directory row, while the subject DID disambiguates duplicate names.

`tonk space rm` removes only the local replica (and, unless `--keep-data` is
used, its local bytes). It does not remove signed account directory facts,
revoke memberships or invitations, deprovision hosting, delete remote objects,
or erase a peer's replica.

Interrupting `tonk account login` before the callback arrives records no local
authority. The next run binds a fresh callback and prints a fresh URL. If the
browser registered the device but its callback response was lost, approving
again for the same account/device converges on the provider's one active
generation instead of creating an invisible duplicate.

New approval pages label the callback `tonk.cli-authorization.v2` and carry the
provider's canonical generation. For an unversioned callback from an older
page, the CLI treats every attachment field as a hint and asks the provider for
the exact generation authorized by the callback grant before writing locally.
An omitted `serviceUrl` still uses the command's configured provider.

`tonk account logout` commits locally first, so it works offline. Existing
spaces remain readable and editable, while fetch, pull, push, account sync, and
other access-service requests are denied before HTTP. Logout queues a
signed, generation-specific detach intent; the device list may remain stale
until a later account operation reaches the provider and flushes that outbox.

A detach intent is signed once and never edited. `detached`,
`alreadyDetached`, `superseded`, and `revoked` retire it; timeouts, rate limits,
server errors, and malformed/refused receipts leave it durable for inspection
and retry. Old cleanup never blocks login. If the provider reuses that exact
generation during same-account recovery, the CLI will not deliver its stale
detach while the generation is locally active; a later logout retries it.
While browser approval is in flight, a process-held handoff lock also defers
cleanup from concurrent account commands so an old detach cannot race the
provider registration before its callback becomes locally active. The lock is
retained through the command's account-registry and custody settlement, and is
released automatically if the waiting process exits.
`tonk account status` and later account commands retry queued cleanup within a
bounded deadline.

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
embedded ephemeral key); `tonk join` claims one into a fresh space
(`tonk join <url> --name <space>`).

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
