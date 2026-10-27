# Desktop links and Settings implementation

Base: bc6a3cbccb (main). Approved scope: audit proposals 2 and 7.

- [x] Simplify All Links and per-space link rows; retain Copy, permissions, local-secret availability, busy states and confirmations.
- [x] Add Account / Chat Agent / Advanced tabs; preserve drafts and save semantics; remove duplicate footer action.
- [x] Run focused checks, build and exercise native fixture UI.

Out of scope: sharing-flow rewrite, window chrome, overview cards, chat entry and identities.

## Result and verification

- All Links uses a native inset list with space sections, without counts or group cards. Per-space links also lose their group card.
- Copy and access/expiry remain visible. More and contextual menus expose usage, Reset, Revoke, or Publishing Settings as appropriate. Usage is explicitly toggled from the menu rather than expanded by selecting a row, keeping selection from changing row height unexpectedly.
- One Account & Settings footer action opens native Account / Chat Agent / Advanced tabs in the hub. Chat Agent entry points select that tab directly; Back restores the prior hub page.
- Provider and Advanced drafts retain stable tab identity. Display name autosaves, provider Apply stays explicit, and remote Save still requires restart. Standalone provider/Advanced dialogs retain their existing dismissal controls.
- Removed generic passkey education; kept explanations of saving, scope, and consequential account actions.

Checks on 27 October 2026:

- First link spike: 27 AccessActionsTests passed.
- Combined focused check: 39 access/provider tests passed.
- Final `bash desktop/scripts/dev.sh test:unit`: 213 tests reported, 4 opt-in tests skipped, 0 failures (115 Tonk + 98 HarnessCore).
- Final `TONK_SIGN_IDENTITY=- bash desktop/scripts/build-app.sh`: passed; isolated review bundle also passed `codesign --verify --deep --strict`.
- Native fixture walkthrough: All Links and space link layouts; More and right-click usage disclosure; missing-local-secret explanation; Reset confirmation and cancellation; all three Settings tabs; unapplied provider/model and remote drafts preserved across tab switches; Back to All Links and to the previous space.
- Visually checked light mode at 800×600 and the compact 700-pixel width (existing window minimum prevents 500-pixel height), plus the final build in dark mode at compact width. Final passkey copy removal is visible in the final native accessibility tree.
- `git diff --check`: passed.

Limits: fixtures use invented accounts and unavailable link secrets; no live grant mutation, passkey ceremony, provider login, or remote change was performed. No full VoiceOver or Reduce Transparency/Motion walkthrough. The separate sharing-flow redesign is not included. Changes were committed on `codex/desktop-links-settings`; the original dirty checkout is untouched.


## Space-management integration

PR #1192 merged as `55b155cb99` after 47 passing checks and 3 intentional skips. Its review finding about a lost Create response was fixed with original-receipt recovery and two regression tests before merging.

The links/Settings commit was rebased onto that merge. The resulting desktop and MCP source trees match the combined candidate verified locally: 220 tests reported, 4 opt-in tests skipped, 0 failures. The combined native fixture showed New Space, the Rename context action and naming sheet, and all three Settings tabs. Rename was cancelled without changing data. Live space mutations were not exercised.
