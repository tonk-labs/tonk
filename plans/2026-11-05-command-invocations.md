# Command invocations: telling the person what became of a command

Status: implemented on `worker/zen-cerf-5f12ha`.

## The problem

A command run from the command palette vanished. The palette transacted
the command, closed, and never heard back: the transact returns before
any provider runs, a provider returned `()`, and failures went to `log!`.
Nothing on any branch said whether the command ran, failed, or matched
nothing at all.

## The design we agreed on

**An invocation is the command's own entity, kept in the session
overlay of the branch the command ran on.** The overlay is local and
never replicated, so following a command writes nothing durable.

**Tracking is the invoker's choice, made per run.** The invoker stamps the
command with `xyz.tonk.command/invoked-at` (when it was run) and
`xyz.tonk.command/concept` (which command it is). The timestamp also
makes each run its own entity: two runs of "rename to Plans" no longer
collide. A command nobody stamped gets no invocation, so the per-keystroke
`intent/interpret`, `tonk:load` and `site/select` cost nothing extra.

**What an invocation holds** (`invocation` in `core.yaml` and
`profile.yaml`, which keep identical copies; Rust in
`tonk-schema/src/invocation.rs`):

| attribute | what it is |
| --- | --- |
| the command's own fields | copied, so the command's own `status` view can render the invocation |
| `xyz.tonk.invocation/case` | `case:active`, `case:done`, `case:failed`, `case:aborted`, `case:unhandled` |
| `xyz.tonk.invocation/state` | what the provider says it is doing: an entity with a `xyz.tonk.state/status` |
| `xyz.tonk.invocation/output` | what it produced; for a failure, an `error:` entity whose status says why |
| `started`, `finished`, `expires` | the worker's clock, in milliseconds |

**Providers return an `Outcome`.** `Command::Output` is now
`Result<Done, Failure>` for every command, and every provider was moved to
it. Where a provider used to `log!` and return, it now returns `Err` with a
message a person can read. `Done` can name the entity the command
produced.

**Providers report progress through `CommandEnv::task()`.** That handle
does nothing for an untracked command. Reports go to one reporter per
dispatch, which writes at most every 100 ms, in one overlay write and one
re-evaluation of the branch's subscriptions. A provider never waits on
subscribers.

**Commands no provider takes.** A rule that handles a command at commit
time says so by concluding `command/completion` on the command entity
(`notebook/retitle` does). The reactor's commit now returns what its
rules concluded (`Commit::conclude`, from dialog's staged
`TransactionBatch::induced`), and the dispatcher reads it. No provider and
no completion means `case:unhandled`.

**Expiry.** A finished invocation gets `expires`: by default 30 s for a
success and 10 min for anything else, or the account's `command/retention`
(durable, on the profile; no settings control yet). Expired invocations
are retracted by a timer started when they finish, by the next tracked
dispatch on the branch, and when a successor worker restores the overlay.
On restore, an invocation still `active` becomes `aborted`: the worker
running it is gone.

**The palette** mints `this: urn:uuid:…` for each run, stamps it, and
follows its `case` on the branch it ran on. A line in the bar says what
was run and where it got to; a success clears itself after a few seconds,
and anything else stays until clicked or replaced.

**The console** is an installable component, `tasks`, at
`/console/tasks`. It lists the space's invocations, newest first. Opening
one renders it through its command's `status` view.

## Checked along the way

- A transient's facts held in the overlay do not fire the rules that watch
  it (`a_transient_in_the_session_overlay_fires_no_rule` in
  `tonk-evaluator`). Without this, copying a rule-handled command into the
  overlay would replay it on the next commit.
- `TransactionBatch::induced` is in dialog `v0.4.0-rc.3`, so no dialog
  change was needed.

## Known gaps

- **The console sees only its space.** A space's pages cannot read the
  profile branch, so commands run on the profile (most hub commands, and
  profile commands from the palette) are not listed at `/console/tasks`.
  The palette's line shows them. Listing them needs a hub page or a read
  grant from a space to the profile.
- **Secrets entered the overlay.** A command field carrying a secret (a join
  URL's fragment) was copied in plain text. Closed by sealing them
  (`plans/2026-11-05-sealed-command-secrets.md`).
- **`/evaluate` does not surface rule conclusions**, so a rule-handled
  command evaluated through it reads as unhandled. Only the palette
  tracks commands today, and it uses `/transact`.
- **Ceremonies.** A passkey command's provider returns once it has asked
  the page; the rest is still reported through `CeremonyStatus`, not
  the outcome. Folding `CeremonyStatus` and `EmailStatus` into
  invocations is a follow-up.
