# One vocabulary, one shape, one way in

**Goal:** Make the `tonk` CLI answerable from its own surface. A caller —
usually an agent, reading `--help` once and then working from memory — should
be able to guess a command's spelling, its flags, and the shape of its output
from any other command they have already used.

**Approach:** Nothing here is a new capability. Every item below is a place
where two parts of the CLI do the same thing two ways, and the fix is to pick
one. Items are ordered by what a caller hits first.

**Status:** every consistency item below is landed on `feat/cli-simplify`
except the separately gated legacy-migration deletion in 4b.

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
first time a command reads the registry. The old command, flag, and environment
spellings are removed; only the on-disk reader remains so an existing store can
convert without data loss.

**0.2 `tonk context --json` moved to camelCase and `space` keys in v2.**
Item 1 subsequently added account and sync sections and moved the current
contract to `tonk.context.v3`.

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
$ tonk status                $ tonk space use
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
| `tonk space use` | the space section alone |
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

*Resolved that way, with one correction: there is no last-known state to
report. Dialog caches no upstream revision, so without a fetch the only thing
knowable locally is whether an upstream is configured at all. `context` says
exactly that — a `not-fetched` state rendering as "upstream configured, not
checked (run `tonk status`)" — rather than implying `synced`. The JSON carries
`fetched: false` alongside, so a reader acting on `state` knows which it got.*

## 2. `use` and `unbind` are inverses in different groups

`tonk use <name>` binds the current directory to a space. `tonk space unbind`
clears that binding. One is top level, the other is a `space` subcommand —
and `unbind`'s own help says *"see `tonk use`"*, which is the tell.

**Proposal.** `tonk space use <name>` and `tonk space unbind`, and no
top-level alias. Binding is space registry management; that is where the noun
already is, and the whole point of moving it is that the pair reads as a pair.
A top-level alias kept for typing convenience would put half the pair back
where it was, which is the thing being fixed.

**Cost.** Breaking. `tonk use` stops resolving. It is one line in the guide,
the README, and the bench harness, all updated with the move; the failure mode
is clap's "unrecognized subcommand" with a suggestion, not silent wrong
behaviour.

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
| `tonk account link` | `tonk account login`, and nothing else |
| `tonk account logout` | unchanged — it is already the pair for `login` |
| `tonk space link <name>` | keeps `link` |

`login`/`logout` is a pair anyone recognises. No hidden `link` alias: keeping
one would leave the collision in place, since `tonk account link` and `tonk
space link` would both still resolve, which is exactly the state being fixed.
Once the account side gives the word up, the space side can keep it — `link`
then means one thing, so renaming it to `adopt` would only cost recognition.

**Cost.** Breaking on `tonk account link`, which fails loudly. Requires
sweeping the ~10 hints that say it, plus the ones in tonk-ui and the account
service that name the command.

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

*`migrate space` landed and was then removed by item 4b, leaving two.*

`tonk migrate` with no subcommand names no conversion, so clap's help lists
the three rather than picking one.

No aliases for the old spellings. `--legacy`/`--site`/`--branch` cannot become
one anyway — a flag set is not a subcommand — and all three are one-time
operations run from written instructions, which is the case where a loud
failure costs least and a silent wrong default costs most.

`migrate space` is temporary either way: item 4b deletes it once a release
carries a working copy.

**Cost.** Breaking for `tonk migrate --legacy`, `tonk migrate --from`, and
`tonk account migrate`. All three fail loudly.

## 4b. Delete the pre-dialog-upgrade migration

`tonk migrate --legacy` is the last code in the tree that has to know how the
old build spelled `spot`. It downloads `v0.6.7`, hands it a `spots.json` in a
throwaway directory, drives `export` through it with `--spot`, and imports the
result. Only a handful of spaces ever predated the dialog format change, and
they are believed migrated.

**Done** on `refactor/cli-drop-legacy-migration`, stacked on the branch that
provides the working migration. The gating condition below is why it is
stacked rather than folded in: it must not merge before its base.

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

That constraint decides the output. Skipping the import means there is no
hash, because the hash *is* the imported bytes — so a dry run reports type,
size, and name on stderr and leaves stdout empty rather than printing a
`blob:<hash>` that names nothing stored. It is a separate `blob::plan`
function rather than a flag threaded through `add`, so the type makes the
absence of a reference structural instead of a runtime promise.

`auto_sync` also grows `around_commit`, the sync wrapper for any committing
write. `run_eval` had this sequence inlined for the one write path that
always had it.

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

*Resolved: left alone, and said so. `tonk query` is `tonk eval` with a query
and no writes, and the two emitting the same document is the useful property.
Wrapping one and not the other would break that to gain a version string the
response already carries. The guard test names it as the one exception.*

*Single documents — `context`, `status`, `space use`, `account status` — carry
`schemaVersion` beside their fields rather than a `rows` array they have no
rows for. The convention is the version string, not the array.*

## 7. Two error printers, and you have to pick which half to lose

Errors leave the binary two ways:

- `print_failure(err)` — honours `--verbose`, printing the whole `{:#}` chain,
  but flattens every failure to `ExitCode::IoError`.
- `eprintln!("error: {err}"); err.exit_code()` — 20 call sites — keeps the
  typed exit code and ignores `--verbose`.

So a call site that needs a real exit code cannot honour `-v`, and one that
honours `-v` cannot return a real exit code.

**How much `-v` actually loses today: nothing.** Every error enum the 20 sites
print is flat or interpolates its source (`#[error("io error: {0}")]`,
`#[error(transparent)]` onto another flat enum), so `{:#}` renders the same
string as `{}`. The gap is latent, not live — the first variant that gains a
non-interpolated `#[source]` is where it bites, and it bites silently.

**Proposal.** A `Coded` trait on the library's error enums, and one printer
that renders through `failure_text` and returns `error.exit_code()`. Three
things fall out: 20 copies of a 4-line block become 20 one-line arms, `-v`
starts working the moment an error grows a chain rather than needing to be
rediscovered then, and "every error enum maps to an exit code" becomes a
trait impl the compiler checks — which retires the `#[allow(dead_code)] fn
classify` that was standing in for that check and, being dead and allowed,
could not perform it.

**Cost.** None to output. This is the cheapest item on the list and goes
first.

## 8. Retire the compatibility aliases

`--spot`, `TONK_SPOT`, `TONK_SPOTS_STATE`, `tonk spot`, and `tonk account spots`
are removed. Keeping an invisible flag or environment alias is not free: an
old harness can silently select no space and continue against a different
binding. A loud parser failure is safer than that wrong-space fallback.

The on-disk compatibility reader is intentionally separate. `spots.json`, its
`spots` key, and the `spots/` root are still recognized once and converted to
the canonical layout. This preserves existing data without keeping two CLI
vocabularies alive.

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
  say so in their help. *Resolved: unhidden. They work, they are tested, and
  the legacy space migration is built on them — "plumbing" described who
  wrote them, not who can use them.*

---

## Status

Landed on `feat/cli-simplify`, in this order:

1. **Item 7** — one error printer. *Done.*
2. **Item 2** — `tonk space use`, no top-level alias. *Done, breaking.*
3. **Item 3** — `tonk account login`, no `link` alias. *Done, breaking.*
4. **Item 4** — `migrate carry|space|account`. *Done, breaking.*
5. **Item 5** — `blob add` syncs and takes the write switches. *Done.*
6. **Item 9** — the four smaller fixes. *Done, two breaking.*
7. **Items 1 and 6** — one orientation vocabulary, one JSON envelope.
   *Done, breaking.*
8. **Item 8** — remove every `spot` command, flag, and environment alias while
   retaining one-way on-disk conversion. *Done, breaking.*

Not landed:

- **Item 4b** (delete the legacy migration) — written, on
  `refactor/cli-drop-legacy-migration`, stacked on `feat/cli-simplify`. It
  must not merge before its base: that base is what makes a published
  migration work at all. See the item.

The two open questions are answered in their items: `context` stays offline
and reports `not-fetched`; `tonk query --json` keeps its `EvaluateResponse`.
