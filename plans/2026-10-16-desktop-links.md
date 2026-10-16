# Desktop around links and access

Status: in progress on `feat/desktop-links`.

Goal: re-implement Tonk Desktop around a links-first layout. The sidebar has
New link, All links, For agents and the spaces list, with sync status and the
account at the foot. Each space opens to an overview with People, Views, Recent
activity and Access (links with Reset / Revoke / Copy). Build everything the
worker supports today and flag the rest.

## What the worker supports (inspected at 0df8e73610)

| Need | Source |
| --- | --- |
| Person link (anyone, write, whole space, never expires) | `POST /api/repository/{s}/invite` -> `{kind:"open", url}` |
| List / revoke person links | `GET /api/repository/{s}/invites`, `POST .../invites/{cid}/revoke` (needs `/`) |
| Agent link (write, main branch, 90 days) | `tonk:agent-handoff` command (existing `createCLILink`) |
| List / revoke agent links | `GET /api/account/connections`, `POST /api/account/connections/{id}/revoke` |
| Public read link | `space/publish` / `space/unpublish` commands on profile main; address `{origin}/space/{did}` |
| People | `tonk:member` facts on space main (member, role, name, invitation) |
| Agents per space | connections filtered by subject; installations name the agent |
| Views | `view` concept rows (keyed by model) + `tonk/space` home alias; route `/space/{did}/{model}` |
| Sync | `GET .../branch/main/sync/status` |
| Account name | `GET /api/profile` `display_name` |

## Decisions

- Link URLs are bearer secrets the worker never stores. Desktop keeps the URL
  of a link it mints in the macOS Keychain, so Copy works for links made on
  this Mac. Links made elsewhere show Copy disabled with the reason.
- Person-link mint -> its revocable `targetCid` is found by diffing the invite
  list before and after; agent links likewise by connection id.
- Reset = mint a replacement, copy it, then revoke the old link. The worker has
  no rotate; composing it keeps a working link at every moment.
- Recent activity: there is no change feed. Show this Mac's own access history
  (links made, reset, revoked, published) from a local journal, plus "joined
  with a link" rows derivable from roster facts (no timestamp). Labelled as such.
- "Used by" for person links needs the invitation entity on each listed invite.
  Additive worker field `invitation` on `InvitationSummary`; Desktop degrades
  when an older worker omits it.
- Opening a view loads the existing runtime + chat surface at that view's URL.

## Not possible yet (flag to user)

Sub-spaces; read-only or expiring person links; per-link scope to a view or
sub-space; cross-member activity feed with authors and times; claim timestamps
and use counts; per-space icon/kind; avatars; agent link expiry/scope choice
(fixed 90 days, main, write); "Reset" for agent links as a worker primitive.

## Checkpoints

1. Pure access model + description formatting + tests (HarnessCore).
2. Runtime loaders/actions, Keychain link store, local journal.
3. Sidebar + space overview + links lists + New link sheet; ContentView shell.
4. Worker `invitation` field + test.
5. Build, unit tests, fixture screenshot; report.
