# Round 2: native Desktop implementation

Live Figma review: 2026-10-20, [Round 2](https://www.figma.com/design/dPj7gCM4NsHNiDUGSxdMJq/MINI-DESKTOP?node-id=91-1938).
This supersedes the Page 0 notes where noted below. No Figma content was modified.

## Reviewed updates

- Connections (91:2410) separates Tonk's chat agent from agent accounts.
- Provision account (102:2304), handoff (102:2518), and completion (102:2561)
  now specify name, initial space/permissions, a single-use expiring setup link,
  and a resulting account key in the Address Book.
- Share menu (91:2005) now contains Publish only. The reused component description
  still mentions Invite; the actual screen and meeting decision take precedence.
- View window (95:4130) and note (95:4250) now specify no full-width top bar:
  controls/address live in the sidebar, chat labels its authority, and the model
  picker moves to the composer. Additional frames cover all chats, collapsed
  sidebar, pointer-at-top controls, loading, and chat-agent settings.

## Distinction applied to implementation

Identity and connection method are separate dimensions:

| Concept | Meaning | UI behavior |
| --- | --- | --- |
| Tonk chat agent | The configured provider used by the native chat, acting through the user's current space authority. | Separate Chat Agent section, real provider/model/connection state, and Settings opens the existing provider dialog. Chat header says it edits as you. |
| Provisioned agent account | A sponsored persistent identity with its own name/key. | Separate Agent Accounts section. Provisioning remains unavailable until the account protocol exists; never reinterpret existing grant/client records as accounts. |
| External access grant | Authorization for a recipient to access a space. It may be used by multiple clients. | Separate External Access section, grouped by space; Revoke Access describes revoking the grant for every client using it, not deleting an account. |
| CLI authorization | A grant explicitly marked `kind: terminal` by the worker. | Label CLI authorization; preserve and optionally show recipient key. This is authorization origin, not proof of a currently running CLI session. |
| MCP or pasted-link use | Ways a tool/chat may obtain or exercise access. | Explain these methods, but do not guess a row's method from a client name or claim a pasted link is an active session. Current API does not report this distinction. |

An agent account could use MCP and CLI; one client could work with different accounts.
MCP and CLI may also occur together. Do not use mutually exclusive account types
such as "MCP agent account" or infer separate identities from installation names.
A setup link that provisions/claims an account must remain distinct from a link
that grants access to a space.

Current `GET /api/account/connections` exposes recipient, optional kind, grants,
confirmation, and registered installations. The worker sets kind=terminal from
terminal_request. None of these fields establishes sponsored account kind.
The UI now retains these actual fields. Space summary counts are labelled access
grants rather than agents, preventing one grant from being counted as one identity.

## Implemented in the continuation

- Full-height 300-point view sidebar; native window controls, hide/menu buttons,
  sync/name/view breadcrumb and compact share control live in the sidebar.
- The embedded WKWebView stays mounted while the sidebar collapses. Its rounded
  12-point surface has an 8-point outer inset while the sidebar is shown.
- Collapsed mode fills the window. A 52-point control strip appears at the top
  pointer position, and Command-B restores the sidebar. A native, non-consuming
  event tracker works above WKWebView; hidden controls cannot steal web clicks.
- Chat title, explicit “edits as you” authority, compact messages, provider label,
  composer model/settings picker and send control follow the revised layout.
  Existing tool activity and failures remain visible rather than being discarded
  to mimic sample content.
- Inline, space-scoped chat history with search, date groups, active selection,
  back/new-chat controls. Opening history preserves the embedded view; reopening
  the current chat does not reconnect its provider.
- Loading view and Chat Agent Settings use the new structure while preserving
  real provider configuration, sign-in and model availability.
- Account grouped rows, debounced name autosave, email and passkey metadata.
  The account reader uses the current profile branch, the account DID for email,
  and the same sender-to-custody relationship used by the worker for passkeys.
  Only display metadata returns to Swift; no credential IDs or sealed payloads.
  Read failures are visible and retryable. Local passkey metadata is a fallback.
- Address Book key-copy controls, identity header, shared-space provenance and
  wrapping local label pills. No verified agent identity is inferred from labels.
- Exact Round 2 vector assets, six-column icon picker, six primary color choices
  with existing additional colors retained, and sync popover hover/hold behavior.
- Connections keeps chat, provisioned identities and external grants distinct;
  Publish remains the only share-menu entry, per the Round 2 frame/discussion.

## Backend and contract gaps

- Account provisioning needs verified account DID/kind/sponsor and the setup-link
  create/claim/expire/cancel lifecycle. The existing grant mint endpoint is not a
  substitute for the single-use, seven-day account setup link. The provisioning
  boundary is explicit; it does not fabricate a completed account.
- MCP versus pasted-link telemetry cannot be inferred. If shown per row, it needs
  authenticated recipient association, authorization origin and observed transport
  as separate fields. The worker's explicit `kind: terminal` identifies CLI
  authorization origin, not proof of a running session. Client names are labels.
- Passkey removal has no supported operation in this baseline. Adding a passkey
  and account deletion use the existing browser account ceremonies, which require
  user interaction. Email/passkey display is implemented, not backend-blocked.
- View-target sharing still needs a defined, tested redemption continuation; the
  existing links grant/open the whole space. Public expiry and usage counts are
  not exposed by the current publication interface. Supported values are stated.
- The All Links customization handoff needs a defined home-space target and
  capability. It must not accidentally grant authority over the account root.
- Contact labels remain local annotations. Cross-device contacts, verified agent
  markers and authoritative onboarding provenance need supported data contracts.
- No background monitor was scheduled; this implements the inspected Round 2
  snapshot. Figma itself was not modified.

## Validation

- Focused native chrome/account checks passed, including window action forwarding,
  tracking-region click passthrough, active-branch account reads, account scoping,
  metadata-only results and reset isolation.
- Full `bash desktop/scripts/dev.sh test:unit`: 156 tests reported, zero failures,
  four expected skips (two opt-in Claude tests and two local diagnostic-server
  tests). After the last visual refinements, 21 focused chrome/account regressions
  passed again. Logs: `/tmp/round2-final-tests.log` and
  `/tmp/round2-final-focused.log`.
- Final `TONK_SIGN_IDENTITY=- bash desktop/scripts/build-app.sh` and
  `codesign --verify --deep --strict desktop/.build/Tonk.app` pass.
  `node mcp/scripts/sync-desktop.mjs --check` and `git diff --check` pass.
- Native CUA checks: 1200 × 800 view/sidebar, collapse and Command-B restore,
  pointer-at-top reveal and its restore button, inline history filtering and
  reopening, composer Settings, dark loading state and native close behavior.
  The live WKWebView remained mounted throughout sidebar/history transitions.
- Native launcher checks at 800 × 600: final Account with email and two invented
  passkeys fits without clipping; Address Book search is in the toolbar, labels
  precede provenance, key-copy glyphs are visible; icon picker has the exact
  exported vectors, six columns and primary/extended color choices.
- macOS still owns launcher toolbar glass, sidebar selection, sheet shells and
  traffic-light rendering. These native surfaces vary from the static wireframes;
  the revised view window no longer retains the old full-width titlebar.
- Deployed account reads/writes, passkey ceremonies, provider execution, remote
  sync/pause and link redemption were not exercised. Account fact reads were
  verified in WebKit against controlled responses and checked against the worker
  schema/read path; a deployed end-to-end account smoke test remains unverified.
- Fixtures use isolated data directories and a disposable ad-hoc-signed app.
  They do not establish deployed worker, provider, passkey or sync behavior.
- No live grants/accounts were created or revoked. No installed app was replaced,
  and no release or installation was performed. The implementation is being committed
  and submitted for review at the user’s request.

## PR integration status

The implementation commit is `830ba3073e`, based on `d44a6face0`. The branch also
contains 11 earlier Desktop precursor commits not in `origin/main` by ancestry.
A read-only merge check against `b2fde22922` found conflicts in 19 files, including
ContentView, TonkApp, runtime/window handling, related tests, and two inherited
Rust/profile changes. Main now contains a different space-window structure.
The PR is submitted as a draft: reconciliation with current main and rerunning
checks on the integrated result remain outstanding. The local verification above
applies to this branch, not to a merged candidate. No conflict resolution or
replacement of main's newer behavior is implied by these results.
