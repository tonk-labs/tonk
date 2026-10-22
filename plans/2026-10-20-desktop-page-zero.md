# Desktop Page 0 implementation

Follow-up: [Round 2 review and identity/connection distinctions](2026-10-20-desktop-round-two.md) supersedes the initial Invite menu, agent-count terminology, view chrome, Account gaps, and missing-design notes below. This file records the original Page 0 checkpoint; current completion and limitations are in Round 2.

Source: [MINI DESKTOP / Page 0](https://www.figma.com/design/dPj7gCM4NsHNiDUGSxdMJq/MINI-DESKTOP?node-id=51-2475).
Discussion: JDD/10 calendar/2026-10-20 Tonk Desktop Design.md.
Base: d44a6face0, preserving the latest desktop links, separate view windows,
and synced appearance work. Primary checkout has unrelated MCP/benchmark work.
Implementation: isolated managed worktree `desktop-page-zero/tonk`.

## Completed checkpoints

- [x] 800 × 600 launcher, 192-point sidebar, compact view cards and grouped Links.
- [x] Default Copy Link split control, Invite boundary, public read-only Publish sheet,
  sync beside space name with status popover and pause/resume command.
- [x] Address Book with name/key search, shared spaces, local saved contacts and labels.
- [x] Connections with existing registered client names and disconnect action.
- [x] Native Account page with display-name editing and existing account-management handoff.
- [x] 1200 × 800 view window, 270-point chat sidebar, sidebar sharing controls,
  space/view breadcrumb, centered loading state, preserved hidden floating action button.
- [x] Existing icon-color controls retained; provider sheet renamed Agent Settings and compacted.
- [x] Exact Figma SVG assets packaged locally, light/dark group and accent colors.
- [x] Build, unit tests, native layout checks, and implementation limits recorded below.

## Behavior and boundaries

Copy Link reuses an active shared link whose secret is retained on this Mac. If no
shared link exists it creates one; if an existing link's secret is unavailable it
requires an explicit Reset rather than silently creating more access. A clipboard
failure never reports success, and a newly minted secret remains available for retry.
Older shared links remain visible and resettable. Agent grants are managed under
Connections rather than offered as a new link kind in sharing.

Contact labels are local, account-keyed metadata. Encountered members are displayed
from the current space rosters; only explicit Save persists a contact. Labels cannot
change cryptographic identity kind or confer access. Signing out clears the displayed
identity/rosters so another account cannot see the previous account's contacts.

## Remaining gaps and required decisions

| Area | Current behavior | Missing dependency / outstanding detail |
| --- | --- | --- |
| Agent onboarding and invitations | Add Agent and Invite explain the unavailable flow; existing agent connections remain manageable. | Human-sponsored agent identity onboarding and an onboarded-agent invitation API are not provided by this desktop baseline. The meeting also assigns a new Add Agent design. No arbitrary DID invite or editable agent-kind toggle was invented. Default person links are not claimed to implement agent onboarding. |
| View-specific sharing | Copy Link grants/opens the whole space. | A defined, tested view target through link redemption is still needed. Appending a view route to secret-bearing links without verifying redemption would be misleading. |
| Publish scope/expiry | Public read-only access, whole space, never expires, using the existing publish/unpublish implementation. | No supported expiring public grant or view-scoped publication controls are exposed by the current API. The sheet states the supported values rather than presenting inert selectors. |
| Views/subspaces | Real catalog views are shown; Home is marked Main view, others View. | View/root/component semantics and subspace hierarchy remain unresolved in the meeting. No invented subspaces or guessed view types. |
| Address Book | Local names/keys/labels and current shared-space membership. | No cross-device contact store, directory discovery, verified agent marker, or arbitrary scoped invitations. Whether encountered people should be saved automatically remains a product decision. |
| Connections | Existing grants and known registered client names. | No fabricated last-active, usage, or activity counts; these require backend evidence. |
| Account | Native name editing and sign-out; passkeys/deletion open the existing account page. | Native passkey listing/enrollment/deletion and email management are not ported. The Page 0 account form is therefore partial. Passkey release availability was explicitly unresolved in the meeting. |
| View chrome | Chat/share controls moved left; sync and breadcrumb in the native toolbar. | The full-width native macOS titlebar remains. The promised revised compact view-chrome design is absent from Page 0; this is not pixel-identical to a future revision. |
| Native styling | Figma spacing, groups, assets, sizes, and hierarchy implemented with native controls. | macOS owns traffic lights, toolbar glass, sheet shells and sidebar selection rendering. Their rendering differs from the static Figma chrome. |
| Data study | Existing runtime views preserved. | Separate from Page 0; concepts/query design awaits further input. |

## Validation, 2026-10-20

- `bash desktop/scripts/dev.sh test:unit`: 151 tests reported, zero failures,
  four skipped (two opt-in live Claude tests and two tests requiring local diagnostic servers).
  Includes default-link reuse/missing-secret/clipboard-failure coverage, contact persistence
  and sign-out isolation, access behavior and updated sidebar sizing.
- `TONK_SIGN_IDENTITY=- bash desktop/scripts/build-app.sh`: succeeds with compiled asset bundle; `codesign --verify --deep --strict` succeeds.
- `node mcp/scripts/sync-desktop.mjs --check`: succeeds; no generated bridge drift.
- `git diff --check`: clean.
- Native CUA visual inspection: light/dark launcher at 800 × 600; sharing menu; Publish sheet;
  Address Book; Connections; Account; separate view window at 1200 × 800 with 270-point sidebar.
  Layout fixes after inspection included sidebar minimum width, contact/detail proportions,
  group contrast, explicit toolbar Copy Link label, and removal of misleading view-type text.
- Visual inspection uses the real built SwiftUI views with the existing explicit, isolated
  `--links-fixture` and `--activity-fixture` paths. Invented data is not runtime proof.
  The view fixture has no live embedded application. No account credentials or installed app
  were replaced; a disposable ad-hoc-signed app was used.
- Live sign-in, sync/pause, remote publish/reset/revoke, display-name writes, provider sessions,
  and view deep-link redemption were not exercised against a deployed account.
- No release, notarization, installation, commit or PR was performed.

## Reference map

Saved Figma contexts under `plans/design-reference/page-zero` document the inspected frames:
space 61:1017; Invite 61:1454; Publish 62:1505; Sync 62:1765;
Address Book 62:1992; All Links 77:1228; Connections 77:1456;
View 79:1574; Loading 79:1704; Icon Color 83:1692;
Agent Settings 83:1958; Account 85:1860. Runtime assets are local; reference URLs
are not dependencies of the application.
