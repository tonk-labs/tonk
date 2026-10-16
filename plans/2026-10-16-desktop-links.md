# Desktop around links and access

Status: implemented on `feat/desktop-links`; live account paths unverified (see Evidence).

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

## Evidence (2026-10-16)

- `swift test` (desktop), after review fixes: HarnessCore 81 passed and
  TonkTests 55 passed (2 skipped each). `AnimatedSidebarTests.testRapidAnimated
  ReversalFinishesAtLatestVisibility` is flaky in this session: it failed
  once here and identically on the untouched base `0df8e73610`, then passed.
- Fixture renders (`--local-runtime --links-fixture --fixture-page <page>
  --snapshot <png> --snapshot-sidebar <png>`) show the overview, the agents
  page and the sidebar. The split-view sidebar is not captured by in-process
  window rendering on macOS 26, hence the separate sidebar render.
- Worker: `it_records_the_invitation_on_mint` (wasm, headless Firefox) proves
  the listed `invitation` equals the entity a claimer derives from the link.
- Home-alias notation (`name: this: id:tonk/space`) checked against a local CLI
  space: `matches_after[0].results[0].fields.entity`.
- NOT run: any path against a signed-in production account (mint, revoke,
  reset, publish, roster, agent connections, sync), or opening a view in the
  live runtime. These need a manual pass with a real account.
