# Scoped agent connection captures

Running-product evidence for `ACCT-C14` / `HANDOFF-21`, captured on 2026-09-16
from the unstaged worktree based on `8acaa1897d3ed09a7bbde972f55060761d89f7f9`.
These captures do not update the older canonical visual inventory's source pin.

- UI build: `b32834be7306c317`, with `connection-invites` enabled.
- Worker Wasm: `d96c72c7e5fe6d1e`; guest Wasm: `0f376527840324b9`.
- Manifest: `8bdf4d6528b531352b43b464ec6b9eca3867a020d0a04e1723061dbf0c80d05a`.
- Isolated Chrome 153, local native access service and current local CLI.
- Full two-holder journey plus layout checks passed in 23.55 seconds.

The screenshots show public fixture identities and grant-management information,
with keyboard focus on the revoke action. No invitation seed or bearer link is
present. The panel scrolls independently on short viewports.

| Capture | Viewport | Appearance |
| --- | --- | --- |
| [Desktop](desktop.png) | 1200 × 900 | Light, reduced motion |
| [Narrow](narrow.png) | 390 × 844 | Light, reduced motion |
| [Short](short-dark.png) | 390 × 540 | Dark, reduced motion |

Root visual review confirmed wrapping and visible actions. Automated checks
verified exact viewport widths, no inner/outer horizontal overflow, real Tab
navigation from Refresh to revoke, focus indication, and a 44px action height.
These are local Chrome captures, not Safari, CI, published npm or hosted-service
verification. The [execution record](../../../../plans/001-cli-space-connections.md)
retains failures and the other evidence boundaries.
