# Refuse remote-less invites, and offer to fix the spot

## Problem

Sharing a spot whose `main` branch has no remote upstream mints an invite that
looks completely normal and can never work. The recipient claims it, the join
flow's `pull_joined_content` fails with `BranchHasNoUpstream`, and they land in a
permanently empty space — no seeded standard library, so `<tonk-display
model="tonk:site">` renders "Model not found".

`resolve_remote_url` (`rust/tonk-worker/src/router/create_invite.rs:357`) already
sees this. It returns `Ok(None)` and the mint proceeds, appending no `&remote=`
to the URL. Its own doc comment argues against exactly this outcome — "silently
demoting to local-only would mask config drift the inviter can't see (redeemers
would hit a downstream sync error with no link to the root cause)" — and then
does it for the most common case.

A spot reaches that state easily. `CreateSpaceHandler`
(`rust/tonk-worker/src/router/repository.rs:483`) creates local-only first,
navigates the creator in second, and attaches the remote third, best-effort, with
a failure reduced to a `log!`. So the remote can be absent because it failed to
attach, because `<tonk-default-remote auto>` never filled the hidden input
(`rust/tonk-workspace/src/default_remote.rs:135` is `if let Some`, no else), or
simply because the attach is still in flight while the user is already inside the
spot clicking Share.

## Decision

A mint that cannot resolve a remote refuses instead of degrading.

When the reason is repairable — the spot has no upstream at all — the share
button opens a dialog offering to turn sync on, and confirming attaches the
remote, mints the invite, and puts it on the clipboard in one click.

## Refusal cases

`resolve_remote_url` returns `Ok(None)` for three situations. All three refuse;
only the first is repairable.

| Case | Reason | Repairable |
|---|---|---|
| `main` has no upstream | `NotSynced` | yes — offer to attach |
| upstream is not `Upstream::Remote` | `UnshareableRemote` | no |
| remote's site is not `SiteAddress::Ucan` | `UnshareableRemote` | no |

`resolve_remote_url` keeps its signature; a caller-side helper maps `None` to the
typed reason. Both mint paths use it: the `tonk:invite` command handler
(`repository.rs:705`) and the HTTP route `POST /api/repository/{repo}/invite`
(`create_invite.rs:130`), which returns an error rather than a remote-less URL.

A deliberately local-only spot therefore stops producing invites. That is the
point: a remote-less invite always lands the recipient in a space that can never
fill, so it has no legitimate use.

`run_invite` currently mints a keypair and delegates before it looks at the
remote. The check moves ahead of `generate_ephemeral`, so a refusal creates no
key material.

## How the refusal reaches the UI

It cannot ride the transact response. On wasm, `spawn_dispatch`
(`rust/tonk-worker/src/router/transact.rs:208`) detaches command dispatch through
`spawn_local`, so the response returns before the handler runs. The failure has
to travel the way success already does.

Success travels as an overlay fact: `run_invite` asserts `Credential { seed, link
}` into the reactor's session overlay, and `<tonk-share>` subscribes to a fully
inline predicate over the raw `xyz.tonk.credential/link` attribute
(`rust/tonk-fab/src/logic.rs:1428`) — inline because seeded rules and views are
frozen per-space.

So the refusal becomes a sibling overlay-only concept keyed on the space's
subject entity, carrying the reason and **the triggering command's `time`**,
which `tonk:invite` already supplies as `dom.event/time-stamp`:

| Attribute | Type | Meaning |
|---|---|---|
| `xyz.tonk.share/blocked` | Text | `not-synced` or `unshareable-remote` |
| `xyz.tonk.share/detail` | Text | the user-facing sentence |
| `xyz.tonk.share/time` | Float | echoed from the command that was refused |

All three cardinality-one, asserted together, keyed on the subject.
`<tonk-share>` reads them with a second inline predicate. Nothing is seeded, so
it works on every existing spot; no `concept!:` entry is added to `core.yaml`
because nothing declarative renders it.

Echoing `time` is what makes the fact safe. It is cardinality-one on the subject,
so it lingers in the overlay and replays on every reconnect. `<tonk-share>` acts
only on a frame whose `time` matches the click it is currently holding a
clipboard write for, and ignores every other frame. That removes any need to
retract it, clear it on success, or guard against a stale error firing on page
load.

## The repair command

There is no working command that attaches a remote to an existing spot.
`space/enable-sync` survives in `core.yaml:183`, but the `#enable-sync` dialog and
`enable-sync-form` it posts to are gone, and its only matching handler is
`CreateSpaceHandler`, which always calls `create_space_inner` first — so
submitting it would mint a *new* spot and attach the remote to that. The doc
comment at `repository.rs:446` claiming it serves an Enable-sync form "against an
existing space" stopped being true when create switched to freshly-minted
identities. Fixing that command is out of scope; nothing posts to it today.

Instead, a new routeless transient `tonk:enable-sync`, dispatched from the FAB the
way `tonk:pause-sync` and `tonk:invite` are:

| Field | Source |
|---|---|
| `space` | the spot's subject DID |
| `remote` | this origin + `/ucan/` |
| `time` | the confirm click's timestamp |
| `share` | marker; present means mint after attaching |

Its handler calls `enable_sync_inner` (`repository.rs:1780`), which routes to the
idempotent `ensure_remote_config` — the same helper the HTTP attach route uses,
including its `refresh_branch` reconciliation. On success with the `share` marker
it calls `run_invite` directly.

That is what makes the flow one click: the FAB needs no completion signal for the
attach, because success arrives as a new `xyz.tonk.credential/link` on the
subscription it is already holding — the identical path a normal mint takes. An
attach failure asserts the same time-keyed status fact with the error.

Being a brand-new command, it has no frozen-descriptor problem: the FAB supplies
the predicate inline in its claim JSON (as `invite_claim_json` does at
`logic.rs:1376`), and the handler matches on trigger attributes, so nothing needs
reseeding.

## The prompt

A second `<wa-dialog>` beside `fab-space-create` in `rust/tonk-fab/src/markup.rs`.

> This spot only exists on this device. Turn on sync so the people you share with
> can open it.

Primary button "Turn on sync & copy link", secondary "Not now". The URL is this
origin + `/ucan/`, resolved the way `default_remote.rs:80` does it — including the
`window.tonk.context.origin` fallback for a sealed guest, where
`location.origin` is the opaque `"null"`. No input, no typing.

## Flow

1. Click share. `<tonk-share>` opens the clipboard write and dispatches
   `tonk:invite`, unchanged.
2. A blocked frame arrives with matching `time`. Abandon the pending clipboard
   write via its `reject` — the browser drops the write and leaves the existing
   clipboard contents alone. Set the button to a `blocked` state and open the
   dialog.
3. Confirm. This is a fresh user activation, so open a **new** clipboard write
   here, dispatch `tonk:enable-sync`, and set the button back to `copying…`.
4. Attach and mint succeed. The new link arrives on the existing subscription,
   the clipboard write settles, the button reads `copied`.
5. Attach fails. The time-keyed status fact carries the error, the dialog shows it
   inline, the clipboard write is abandoned.

`UnshareableRemote` skips the dialog entirely: the button settles to `failed` with
its message.

## Timeout

`<tonk-share>`'s module doc already flags the gap (`rust/tonk-fab/src/share.rs:46`):
"A failed mint has no explicit error signal in this design … there is no timeout."
A pending copy is abandoned only on disconnect.

Add a bounded timer that settles `Failed`. Without it, any mint failure other than
the two above still hangs the button forever, and step 3 now holds a clipboard
write open across a network round-trip.

## Testing

Worker:

- each of the three `None` cases refuses, and asserts the status fact
- the status fact echoes the triggering command's `time`
- the refusal path generates no keypair
- `EnableSyncHandler` attaches to the **existing** repository — a direct
  regression guard against the `CreateSpaceHandler` always-creates trap
- it mints only when the `share` marker is present

FAB, native `#[test]` in `logic.rs` alongside the existing `invite_claim_json` and
`invite_link_query_body` tests:

- the `tonk:enable-sync` claim JSON names the space, remote, time and marker
- the blocked-status query body reads the raw attribute and rejects an empty
  subject

FAB, wasm tests in `share.rs`:

- a blocked frame with matching `time` opens the dialog and abandons the copy
- a blocked frame with a different `time` is ignored
- the timeout settles `Failed`

Existing tests and bench scenarios that mint invites against local-only
repositories will now fail. Sweeping them is part of the work.

## Out of scope

- The dead `space/enable-sync` command and `<tonk-sync-state>`'s enable-sync
  trigger, which points at a dialog that no longer exists
  (`rust/tonk-workspace/src/sync.rs:490`)
- Why the create path's remote attach fails or is skipped in the first place —
  this makes the consequence visible, it does not remove the cause
- A standing "this spot is local" affordance in the FAB. The prompt appears only
  on a share attempt, which is the moment the missing remote starts to matter
