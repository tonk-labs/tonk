# tonk

A local-first CLI for reading and writing tonk facts via asserted-notation.

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
# Attach that exact local space to the account selected in Tonk.
tonk space link garden
# Use an existing space in another project directory:
tonk space use garden

# Every local replica, with the owner each space names.
tonk space

# Connect this CLI to an existing space (no ownership transfer).
tonk join 'TOOL_LINK'

# This creates a person invite. Open it in a browser; it is not a CLI credential.
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

# Delegate access to the space.
tonk invite                    # audience-open: anyone holding it can claim
tonk invite --remote prod      # mint against a named remote
tonk invite --recipient-root did:key:z6Mk... # seed-free targeted invite
tonk invite --no-remote        # embed none; the claimer wires an upstream by hand
tonk join 'TOOL_LINK'           # scoped tool access; no CLI login or browser flow
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

Manage accounts in the Tonk UI. The CLI imports a space-scoped invitation key
and delegation chain; it does not sign into an account or change membership.
The recipient has a separate DID, even though its authority comes from the
browser account. Existing local replicas and legacy credentials are retained.
CLI space creation and transplant stay local and do not provision account hosting.

### Sync and sharing

`push` / `pull` are fast-forward sync over `Branch::push()` / `Branch::pull()`,
with errors that name the upstream-not-configured and non-fast-forward cases.
`status` classifies the local branch against its upstream without merging.
Its `tonk.status.v3` JSON describes only the selected space, sync state, access
kind, and any space-scoped authority. Version three deliberately removes the
cached CLI account/session section: unrelated legacy account state neither
authorizes nor changes the selected space.

Remotes are UCAN-S3 access services registered on the repository's meta branch.
A revocation is an ordinary `ucan/revoke` invocation, so it goes to the access
service like everything else and a mint needs nothing extra. A remote may still
carry a separate artifact relay, supplied by hand with `tonk remote add
--revocation-url`; it is never inferred and never required.
`tonk invite` mints a UCAN delegation chain over the repo and prints an
audience-open invite URL (anyone holding it can claim by redelegating from the
embedded ephemeral key). These older sharing links are not CLI access
credentials: obtain a fresh scoped space
invitation for `tonk join`.

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

### Connect a tool

`tonk join 'TOOL_LINK' [--name NAME]` accepts supported scoped tool links from
the browser's **connect a tool** action. It imports the invitation's identity
and space grants without CLI login, browser approval, or account selection.
The tool receives only the signed space scopes in the link; association with
the issuing account does not grant account authority or create a human member.
Resume with
`tonk --space NAME join`; `TONK_SPACE` does not select a resume target.
The command reports `Agent connection confirmed` only after pulling the space,
publishing its grant-set acknowledgement, and finishing the directory binding.
Copies of one link share the same invitation identity and revocation boundary.
Create a new link when independently revocable access is required. Confirmation
is completed setup, not exclusive tool presence or proof that it is online.

Imported identities stay separate from existing CLI accounts and replicas.
Credentials are retained locally; expiry or revocation blocks further authorized
remote work and keeps downloaded data available offline. `--via`, `--no-open`
and `--switch-account` are not accepted by `join`. There is no `connect`
command or legacy browser-approval fallback. Different browser accounts and
spaces can issue independent links into the same CLI; each remains a separate
credential and local replica. Browser issuance remains subject to the published
CLI release gates.

Connection imports trust the built-in Tonk deployment (`https://tonk.network`).
For an explicitly selected development deployment, set `TONK_CONNECTION_ORIGIN`
to its HTTPS origin or a loopback HTTP origin, for example
`http://127.0.0.1:8787`. This setting chooses service routing only. The importer
requires the signed grant endpoint to match that origin's `/ucan/` and verifies
`/.well-known/tonk` without following redirects. It never chooses an approval
page from the invite, loads an unrelated account's endpoint, or sends the
secret-bearing fragment to discovery.

### Older invitation links and interrupted handoffs

The old `join --agent` flow has been removed. New `tonk join URL` imports only
scoped tool links. It rejects person-sharing and account-approval links before
creating a replica, credential, binding, account, or membership. Person invites
open in the browser and create a separate member. Older CLI binaries may still
accept those links, so use the current binary when this separation matters.
The tool link carries its own identity and grants; the CLI never signs into the
issuing account.

`tonk --space NAME join` resumes scoped connection imports and can finish an
already-persisted legacy person-import journal. A new person URL cannot start
that compatibility path. Existing credentials, replicas, aliases, and unsynced
edits are retained; rejection never converts, rebinds, repairs, or deletes them.
Import a new scoped invitation to access a browser space independently of any
legacy account attachment. Account management is available in the Tonk UI.
