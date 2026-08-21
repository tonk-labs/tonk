# One vocabulary, one shape, one way in

**Goal:** Make the `tonk` CLI answerable from its own surface. A caller —
usually an agent, reading `--help` once and then working from memory — should
be able to guess a command's spelling, its flags, and the shape of its output
from any other command they have already used.

**Approach:** Nothing here is a new capability. Every item below is a place
where two parts of the CLI do the same thing two ways, and the fix is to pick
one. Items are ordered by what a caller hits first.

**Status:** items 0.1 – 0.5 are done on `fix/cli-simplify`. The rest are
specified here and not implemented; several are breaking and want a decision
before they land.

---

## 0. Landed

Recorded so the plan reads against the current surface rather than the one the
audit found.

**0.1 The `spot` → `space` rename is finished.** It had landed on the command
surface — `space` with a `spot` alias, `--space`, `TONK_SPACE` — and nowhere
else, so bare `tonk` on a fresh install answered *"no spots registered; create
one with `tonk spot new`"*, naming a command that no longer existed, in the
first line anyone read. Module and type names, every error and help string,
the guide index, and the on-disk layout now say space. The store moved from
`spots.json` + `spots/` to `spaces.json` + `spaces/`, converted in place the
first time a command reads the registry. `TONK_SPOT`, `TONK_SPOTS_STATE`, the
registry's `spots` key, `--spot`, `tonk spot` and `tonk account spots` are all
still read.

**0.2 `tonk context --json` is `tonk.context.v2`** — `spot` keys are now
`space`, and the document is camelCase like the CLI's other JSON.

**0.3 Every write verb takes `--dry-run`, `--no-sync` and `--quiet`.**
They existed only on `tonk eval`; the noun verbs hardcoded
`auto_sync::enabled(false)` at six call sites. `assert` builds its three into
the concept's own dynamic command, so they appear in
`tonk assert <concept> --help` beside the fields.

**0.4 One listing format.** `concept ls`, `view ls`, `blob ls`,
`remote list`, `space list`, `account spaces` and `account devices` share one
renderer: header row, tab-separated cells, `-` for an absent value, and a
parenthesised sentence naming what is missing when there is nothing to show.
Five of them used to print nothing at all when empty.

**0.5 `--json` on every read** except `tonk schema` and `tonk guide`.

---

## 1. Four commands answer "where am I"

`status`, `use`, `context` and `account status` all report the active space
and its state, in four different layouts — five counting `tonk identity`,
which prints the device and account DIDs a third way.

```text
$ tonk status                $ tonk use
space: demo (env)            current space: demo
no-upstream (set one …)      site: /…/spaces/demo
hash: #7PHx…                 selected via: env
                             next: tonk context

$ tonk account status        $ tonk context
signed in: yes               # Tonk context
account: did:key:…           space: `demo` · branch: `main` · selected via: `env`
account service: https://…   site: `/…/spaces/demo`
device: did:key:…            Changing cwd does not change the selected Tonk data.
status: ready                …
```

Three of them name the same field differently in the same breath:
`space:` / `current space:` / ``space: `demo` ``, and `(env)` / `selected via:
env` / `` selected via: `env` ``.

**Proposal.** `tonk context` is the one that answers the question — it is what
bare `tonk` runs, and its whole job is orientation. Fold the other three into
it as sections and keep them as aliases that print the one they own:

| command | becomes |
|---|---|
| `tonk context` | space, sync, account, concepts, workflows |
| `tonk status` | the sync section alone |
| `tonk use` | the space section alone |
| `tonk account status` | the account section alone |

One renderer, one field vocabulary, one JSON document with the sections as
top-level keys. `tonk status --json` becomes a projection of
`tonk context --json`, not a separate contract.

**Cost.** Breaking for anything parsing the three text outputs. The `--json`
forms landed in 0.5 are one release old, so re-shaping them now is cheaper
than it will ever be again.

**Open question.** Whether `status` should keep fetching the upstream head.
It is the only "where am I" command that touches the network, and folding it
into `context` would make bare `tonk` do so too. Suggested answer: `context`
reports the last-known sync state without fetching, and `status` keeps the
fetch as the thing that distinguishes it.

## 2. `use` and `unbind` are inverses in different groups

`tonk use <name>` binds the current directory to a space. `tonk space unbind`
clears that binding. One is top level, the other is a `space` subcommand —
and `unbind`'s own help says *"see `tonk use`"*, which is the tell.

**Proposal.** `tonk space use <name>` and `tonk space unbind`, with `tonk use`
kept as a visible top-level alias. Binding is space registry management; that
is where the noun already is. The top-level alias stays because `tonk use` is
short, is in the guide, and is what a person types.

**Cost.** Additive. Nothing breaks.

## 3. `link` means two unrelated things

- `tonk account link` — sign **this device** in to an account. Aliased
  `login`, which is what most of the hint text says.
- `tonk space link <name>` — hand **a local space** to the signed-in account.

Two different objects, two different transfers, one verb. The account one is
already half-renamed: `login` exists as a `visible_alias`, one hint says
`tonk account login` and about ten say `tonk account link`.

**Proposal.** Finish the split.

| now | becomes |
|---|---|
| `tonk account link` | `tonk account login` (canonical), `link` a hidden alias |
| `tonk account logout` | unchanged — it is already the pair for `login` |
| `tonk space link <name>` | `tonk space adopt <name>`, or keep `link` |

`login`/`logout` is a pair anyone recognises, and it frees `link` to mean
exactly one thing. Whether the space verb then keeps `link` or becomes
`adopt` is a taste call; the point is that only one of the two keeps it.

**Cost.** Additive if the old spellings stay as hidden aliases. Requires
sweeping the ~10 hints that say `tonk account link`.

## 4. Three unrelated operations are called `migrate`

| command | does |
|---|---|
| `tonk migrate` | copies a pre-tonk `.carry/` directory to `.tonk/` |
| `tonk migrate --legacy --site <name>` | upgrades a space written before the dialog format change, by driving a downloaded v0.6.7 binary |
| `tonk account migrate` | drains the legacy certificate store into the profile's access branch |

The second is bolted onto the first as a flag with
`conflicts_with_all = ["from", "do_move"]` plus two more flags gated on
`requires = "legacy"` — a subcommand wearing a flag's clothes. Its own doc
comment says *"Unrelated to the `.carry/` move above."*

**Proposal.** Three commands, named for what they convert:

```text
tonk migrate carry [--from <path>] [--move]   # .carry/ -> .tonk/
tonk migrate space <name> [--branch <name>]   # pre-dialog-upgrade space
tonk migrate account                          # certificate store -> access facts
```

`tonk migrate` with no subcommand lists the three and what each is for. The
existing spellings stay as hidden aliases; all three are one-time operations
whose users are following written instructions, so the aliases can be dropped
after a release.

`migrate space` is temporary either way — item 4b deletes it. If 4b lands
first, this item is a two-way split and simpler for it.

**Cost.** Breaking for `tonk migrate --legacy`, which is only ever run from
the upgrade instructions. `tonk account migrate` keeps working as an alias.

## 4b. Delete the pre-dialog-upgrade migration

`tonk migrate --legacy` is the last code in the tree that has to know how the
old build spelled `spot`. It downloads `v0.6.7`, hands it a `spots.json` in a
throwaway directory, drives `export` through it with `--spot`, and imports the
result. Only a handful of spaces ever predated the dialog format change, and
they are believed migrated.

**Gating condition: a staging release carrying a *working* migration.**
Stable is not required — the spaces that needed this are believed migrated
already, and the pre-release channel is where anyone who still needs it
would be.

The published `tonk-staging` (2026-08-21, built from `4761f1ac2`) does carry
the command, but it does not work: its `legacy.rs` is byte-identical to this
branch's base, where `tonk migrate --legacy` dies with *"failed to initialize
account-session state: stored account-session state is malformed: missing
field `pending_detaches`"*. The v0.6.7 child inherits the state directory
this build wrote and cannot parse it. Item 0.1's throwaway-registry fix is
what repairs that, by pointing the child at its own store.

So the order is: publish this branch to staging, then delete. The deletion
is kept as its own commit so it can be dropped if that publish has not
happened.

**What goes, when it goes:**

```text
rust/tonk-cli/src/legacy.rs                       whole module
rust/tonk-cli/tests/legacy_migration.rs           whole file
rust/tonk-cli/tests/fixtures/                     whole directory
  legacy-space-v0.6.7.tar.gz, legacy-account-v0.6.7.tar.gz, README.md
rust/tonk-cli/tests/blob.rs                       the two `migrate_blobs` cases
rust/tonk-cli/Cargo.toml                          [[test]] legacy_migration,
                                                  feature `legacy-migration`
rust/tonk-cli/src/lib.rs                          `pub mod legacy;`
rust/tonk-cli/src/bin/tonk.rs                     `legacy_migrate()`, and the
                                                  --legacy/--site/--branch arms
                                                  of `Command::Migrate`
```

Two things need a decision rather than a delete:

- **`tonk_account::LEGACY_FORMAT_REMEDY`** is the error a pre-upgrade space
  produces, and it names the command being removed. It has to keep existing —
  the failure is still legible and still needs an explanation — but the remedy
  becomes "install `v0.6.7` yourself and export" rather than a command tonk
  offers. `tonk_account::readability` and the `Readability::Legacy` arm stay:
  detecting the format is what makes the failure a sentence instead of
  `missing field 'branch'`.
- **`space.rs`'s `LEGACY_REGISTRY_FILE` / `LEGACY_SPACES_DIRNAME`** are the
  `spot` → `space` on-disk conversion from 0.1, not this. They are unrelated
  and are covered by item 8.

Doing this also removes the only test in the crate that needs the network
(`legacy-migration` downloads a real release archive) and the only one
currently failing: `it_migrates_credentials_before_repositories` dies at its
final push with *"subject is not provisioned"*, most likely from `26fd3c39b`
(deny service until the account confirms its email) against a `v0.6.7`-minted
account. It fails on `staging` too, so it is not worth fixing something that
is on its way out.

## 5. `blob add` is the one write that never syncs

Every other write verb pulls before and pushes after. `blob add` commits its
metadata transaction directly, with no `auto_sync::run_eval` anywhere in the
path — so a blob added to a synced space stays local until the next unrelated
write pushes it.

This is why `blob add` is not in 0.3: it is not a missing flag, it is a
missing behaviour. The flags would be dishonest before the sync exists.

**Proposal.** Route the metadata commit through `auto_sync`, then give it the
same three switches. `--dry-run` needs care: the bytes are imported before the
metadata transaction, so a dry run must skip the import too, not just the
commit — otherwise it writes to the blob store while claiming to write
nothing.

**Cost.** Behavioural. A `blob add` in a synced space starts pushing.

## 6. Two JSON envelope conventions

`tonk context --json` carries a top-level `schemaVersion: "tonk.context.v2"`.
Everything else carries a numeric `version: 1` on each row or object. Both are
versioned; neither can be recognised from the other.

**Proposal.** One envelope for every `--json` read:

```json
{ "schemaVersion": "tonk.<command>.v1", "rows": [ … ] }
```

**Cost.** Breaking for `tonk space list --json` and `tonk query --json`, which
shipped before this plan. Worth doing in the same release as item 1, or not at
all — a second re-shaping of the JSON is worse than either shape.

**Open question.** `tonk query --json` emits an `EvaluateResponse`, which is a
different thing again: a transaction envelope, not a listing. It may be right
to leave it alone and say so, since it is the read that shares its shape with
`tonk eval --format json`.

## 7. `--verbose` does nothing on the typed-error path

Errors leave the binary two ways:

- `print_failure(err)` — honours `--verbose`, printing the whole `{:#}` chain,
  but flattens every failure to `ExitCode::IoError`.
- `eprintln!("error: {err}"); err.exit_code()` — 20 call sites — keeps the
  typed exit code and silently ignores `--verbose`, so the chain that explains
  the failure is unreachable.

A caller who hits one of those 20 and runs it again with `-v` gets a
byte-identical message, which reads as the flag being broken.

**Proposal.** One helper that takes both: render through `failure_text` (so
`--verbose` works everywhere) and return the error's own exit code (so the
typed codes survive). Mechanical; no output changes without `-v`.

**Cost.** None. This is the cheapest item on the list and should go first.

## 8. Retire the compatibility aliases

`--spot`, `TONK_SPOT`, `tonk spot`, `tonk account spots`, the `spots` registry
key, `spots.json` and `spots/` all still work, and `space.rs` converts the
on-disk half automatically on first read. The env and flag halves cost
nothing to keep; the visible aliases cost a line each in `--help`, where they
teach a vocabulary the rest of the CLI no longer uses.

**Proposal.** Move `visible_alias` to `alias` (hidden) for `spot` and `spots`
one release after 0.1 ships, and drop `LEGACY_REGISTRY_FILE` /
`LEGACY_SPACES_DIRNAME` a release after that — by then every store that a
current `tonk` has touched has been converted.

`bench/bin/*.sh` was moved to the canonical spellings alongside 0.1, so
nothing in this repo depends on the old ones.

## 9. Smaller things

- **`tonk space rm --delete`** is a hidden no-op kept for compatibility.
  Delete it; deleting the data has been the default for long enough that a
  script still passing it is passing a flag that already did nothing.
- **`tonk agents --json` conflicts with `tonk agents set`**, rejected at
  runtime with a message. Once `set` is the only subcommand that writes, the
  cleaner shape is `tonk agents get --json` / `tonk agents set`, with bare
  `tonk agents` meaning `get`.
- **`tonk schema` has no `--json`** — deliberately, because its output is
  already a machine format and a re-submittable notation document is a better
  one than a JSON transcription would be. Worth one line in the help so the
  absence reads as a decision rather than a gap.
- **`tonk export` / `tonk import` are `hide = true`** but are the only bulk
  path in or out. If they are supported, unhide them; if they are not,
  say so in their help.

---

## Suggested order

1. **Item 7** (`--verbose` on typed errors) — mechanical, no output change.
2. **Item 2** (`space use` / `space unbind`) — additive.
3. **Item 3** (`login` canonical) — additive.
4. **Item 4** (`migrate` split) — additive with aliases.
5. **Item 4b** (delete `--legacy`) — blocked on a stable release carrying it;
   that promotion is the next release action, the delete the one after.
6. **Item 5** (`blob add` sync) — behavioural, wants its own review.
7. **Items 1 and 6 together** (fold the four status commands, one JSON
   envelope) — the breaking pair, best done in one release.
8. **Item 8** (retire aliases) — a release after 0.1 ships.
9. **Item 9** — alongside whatever is nearby.
