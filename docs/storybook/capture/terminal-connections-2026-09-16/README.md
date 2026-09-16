# Selected terminal connection captures

Running-product evidence for `ACCT-C15` / `HANDOFF-22`, captured on 2026-09-16
from the dirty worktree based on `8acaa1897d3ed09a7bbde972f55060761d89f7f9`.
These captures retain their own artifact provenance and do not update the
canonical visual inventory's older source pin.

- UI build: `63775cbb5273ce15`, with `connection-invites` enabled.
- Worker Wasm: `d9de3fc28f1b5483`.
- Manifest: `4b560a0893107c2a90e758bc8e1bf7e43454a01ff75d135120b82e214b34a493`.
- Artifact: `/private/tmp/tonk-terminal-ui-navigation`.
- Isolated local Chrome, native access service, and freshly built local CLI.

| Viewport | Selection and request | Approval action | Terminal management |
| --- | --- | --- | --- |
| 1200 × 900, light | [Request](terminal-selection-desktop-request.png) | [Approve](terminal-selection-desktop-approve.png) | [Manage](terminal-management-desktop.png) |
| 390 × 844, light | [Request](terminal-selection-narrow-request.png) | [Approve](terminal-selection-narrow-approve.png) | [Manage](terminal-management-narrow.png) |
| 390 × 540, dark | [Request](terminal-selection-short-dark-request.png) | [Approve](terminal-selection-short-dark-approve.png) | [Manage](terminal-management-short-dark.png) |

Selection request and approval captures use different scroll positions in the
same panel. The management panel also scrolls on short viewports. Screenshots
show public fixture identities and grants, with no private key or bearer link.
Visual review of desktop request, narrow approval, and short dark management
confirmed readable wrapping and visible focused actions. Automated checks cover
exact viewport sizes, no horizontal overflow, real Tab focus, and 44px action
targets. The panels introduce no animation. Top-document CDP reduced-motion
emulation did not propagate into the opaque guest; guest media-query behavior
is **not** verified by these captures.

## Executed local journeys

- One, several, and all-current selected spaces: passed in 29.24 seconds.
  The CLI retained its own key, installed the complete selection accountlessly,
  and could not perform account-catalogue operations.
- Explicit legacy-account conversion: passed in 39.84 seconds. New scoped
  aliases are independent; old aliases, paths, local data and unsynced edits
  remain. Default remote use through the deactivated legacy attachment refuses.
- Offline management with a revoked queued addition followed by a fresh grant:
  passed in 26.50 seconds with the final CLI. The rejected delivery is reported
  and recorded without an alias, the cursor advances, the later grant imports,
  and an exact local query still reads the retained edit after revocation.
- Signed decline: three navigation-artifact runs passed in 6.11, 6.22 and
  9.48 seconds. Earlier intermittent refusal before staging prompted the
  snapshot regression described in the final checkpoint below.

Lower-layer codec, worker, real HTTP/SQLite/D1, CLI crash recovery and expiry
checks are recorded in the [execution plan](../../../../plans/001-cli-space-connections.md).
This evidence does not establish Safari, CI, staging, published npm/browser
compatibility, hosted-service behavior, or global revocation propagation.
## Final local verification checkpoint

The local `ACCT-C15` / `HANDOFF-22` journey is verified on the later snapshot-fix
artifact. The nine screenshots above remain captures of the navigation artifact;
their appearance is not represented as a capture of this later build.

- Artifact: `/private/tmp/tonk-terminal-ui-snapshot`.
- UI build: `d7f3967a6db1fa1a`; worker Wasm: `4ebdb9f0b3ee3f13`.
- Manifest: `e96677dc59ffedfad02552587c6f5aea28e75abaaa18defcba49786359f04c00`.
- Exact one/many/all-current selection: passed in 26.97 seconds.
- Fresh signed decline: passed in 8.83 seconds.
- Offline management, revoked queued addition and later fresh grant: passed
  in 36.98 seconds.
- Explicit conversion with retained legacy state: passed in 13.95 seconds.

A focused worker regression failed before and passed after a snapshot fix.
The fingerprint now pins the root proof and exact sorted repository/subject
selection, while approval preparation rechecks the currently selected grants.
Mutable presentation and eligibility changes do not themselves invalidate that
selection snapshot. Four worker tests passed in 7.21 seconds; strict workspace
Clippy also passed. These results establish the reproduced defect and its fix,
not the cause of every earlier intermittent pre-staging refusal. Monitor that
historical symptom in staging; the external and guest-media boundaries above
remain unverified.
