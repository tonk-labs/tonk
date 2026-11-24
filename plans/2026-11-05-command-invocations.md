# Command invocations: telling the person what became of a command

Status: implemented on `worker/zen-cerf-5f12ha`, on dialog 0.5 stacks.

## The problem

A command run from the command palette vanished. The palette transacted
the command, closed, and never heard back. Providers returned `()` and
logged their failures, so no branch said whether the command ran, failed,
or matched nothing.

## What dialog 0.5 already gives us

A command lands in its stack's `control:` layer for one tick, and dialog
records its life on the command's entity in `control:`, also for one tick:

- `dialog.command/invoked` when it lands;
- `dialog.command/answered`, `dialog.command/failed` or
  `dialog.command/unhandled` once the providers ran;
- `dialog.command/progress` whenever a provider reports some
  (`Command::progress`).

A provider answers `Result<Changes, dialog_repository::Failure>`, and dialog
turns an `Err` into the `failed` row. The first version of this branch
built all of that by hand on the overlay: a reporter loop, `case`
bookkeeping, `Commit::conclude` for rule-handled commands, an expiry sweep
and abort-on-restore. That code is gone. What is left is keeping those
one-tick rows for a command someone follows.

## The design

**Following is the invoker's choice, made per run.** The invoker gives the
run its own `this` and stamps it with `xyz.tonk.command/invoked-at` (when it
was run) and `xyz.tonk.command/concept` (which command it is). A command
nobody stamped gets nothing kept, so the per-keystroke `intent/interpret`
and `tonk:load` cost nothing extra.

**Rules keep the life of a followed command in `session:`.** The
`invocation-schema` block, identical in `core.yaml` and `profile.yaml`,
reads dialog's rows in `control:` and writes them, with inductive rules,
into the stack's `session:` layer. That layer is the stack's own, kept for
the process, and never committed or synced.

| attribute (in `session:`) | from |
| --- | --- |
| `xyz.tonk.invocation/invoked` | `xyz.tonk.command/invoked-at` |
| `xyz.tonk.invocation/of` | `xyz.tonk.command/concept` |
| `xyz.tonk.invocation/answered` | `dialog.command/answered` |
| `xyz.tonk.invocation/failed` | `dialog.command/failed` |
| `xyz.tonk.invocation/unhandled` | `dialog.command/unhandled` |
| `xyz.tonk.invocation/state` | `dialog.command/progress`, an entity with `xyz.tonk.state/status` |
| `xyz.tonk.invocation/output` | the provider's `Done`, or the rule that handled the command |

**Providers return an `Outcome`.** `Command::Output` is
`Result<Done, Failure>` for every command. The dispatcher maps a `Failure`
to dialog's, and for a followed command writes `Done`'s output into
`session:`. Progress goes through `CommandEnv::task().status(state, words)`,
which invokes `Command::progress` on the stack and writes the state's words
into `session:`. Both do nothing for a command nobody follows.

**The case is derived, not stored.** A reader computes it: `failed` if
there is a failure; otherwise `done` if it was answered or has an output;
otherwise `unhandled` if dialog said so; otherwise `active`. A command a
rule handles at commit time (`notebook/retitle`) is unhandled as dialog
sees it. Its rule writes the output, so it reads as done.

**The palette** reads one subscription over the invocation, with every
field past `invoked` optional, and shows a line in the bar. A success
clears itself after a few seconds; anything else stays until clicked or
replaced.

**The console** is an installable component, `tasks`, at
`/console/tasks`. It lists the space's invocations, newest first. Opening
one renders its output.

## Known gaps

- **Nothing expires.** Invocations live as long as the stack's session.
  The 30 s / 10 min expiry and the `command/retention` setting went with
  the overlay. Dialog has no retraction schedule for `session:` rows, so
  bringing them back needs one there or a sweep here.
- **No `aborted`.** The session goes with the worker, so a restart forgets
  the invocation instead of marking it stopped. The page that was
  following it sees it disappear.
- **The command's own fields are not kept.** They live one tick in
  `control:`. A command's `status` view cannot render an invocation from
  its fields, so the console shows the output instead.
- **The console sees only its space.** A space's pages cannot read the
  profile branch, so commands run on the profile are not listed at
  `/console/tasks`. The palette's line shows them.
- **Secrets.** A command's fields are no longer copied, so this gap is
  smaller. A secret still passes through `control:`, as it did before.
- **Ceremonies.** A passkey command's provider returns once it has asked
  the page. The rest is still reported through `CeremonyStatus`, not the
  outcome.
