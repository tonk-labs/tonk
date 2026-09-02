# Journey catalog

## How to read this catalog

This is the complete map of user goals entered through the current `tonk` CLI
and Tonk browser shell. It is intentionally organized by what the user is
trying to accomplish, not by Rust module. The command and route inventories in
the feature documents map every public entry point back to these IDs.

Evidence is classified as:

- **whole**: an existing test appears to execute the user journey across its
  primary surfaces;
- **partial**: lower-layer or happy-path evidence exists, but the journey's
  failure/restart matrix is incomplete;
- **none**: no focused test was found at this entry point.

This is a source audit, not a fresh test run. “Whole” does not mean verified in
this storybook until the relevant checklist has executed.

Every row inherits the fixed interrupt and failure matrix in
[failure and recovery](cross-cutting/failure-and-recovery.md). “Missing” names
the most important gaps, not the only variants required.

## Accounts: browser lifecycle

| ID | User journey and entry | Starting variants | Existing evidence | Missing or weak evidence |
| --- | --- | --- | --- | --- |
| `ACCT-B01` | Open `/settings`; redirect legacy `/account*` without losing the query. | Fresh, linked, add-account, revoke, delete, callback queries. | Whole happy path in `account_flow`; one route unit test. | Mount failures, unsafe `next`, duplicated/malformed query, history/reload. |
| `ACCT-B02` | Choose Create and create a fresh account/passkey. | Root missing; provider-free root; add-account profile. | Whole happy path plus DOM form tests; account-action presenter unit tests distinguish passkey cancellation, missing PRF support, and unavailable browser ceremonies. | Whole-browser passkey failure matrix, local save failure, remote timeout, response lost after commit, reload at every stage. |
| `ACCT-B03` | Enter an email and follow the ceremony it names. | Address known, unknown, malformed, or the service unreachable. | Browser test expects zero credentials across a taken address and an edit to a free one; presenter tests cover lookup dispatch and account-options subscription failures. | Whole-browser lookup/subscription faults, restart and retry, concurrent availability race, local-root boundary. |
| `ACCT-B04` | Receive and open the activation email link. | Missing, damaged, valid, expired, already used link. | Presenter unit coverage prevents an unclassified service body from becoming activation-page copy. | Whole activation branches, duplicate click, reload, offline, malformed body, receipt-report failure. |
| `ACCT-B05` | See activation pending and resend an activation email. | Customer absent, Registered, Active, Suspended, unreachable. | Partial component logic; happy activation used by broader browser tests; presenter tests cover a failed activation watcher and resend fallback. | Whole-browser watcher failure, resend success/error/rate limit, stale email, Active idempotency, Suspended copy. |
| `ACCT-B06` | Work locally before activation, then attach provider service. | Local-only space; account ready/customer Registered; queued custody. | Whole happy paths for local creation, activation, provider attachment, later sync. | Offline/reload between each stage, duplicate provisioning, suspended customer, lost activation receipt. |
| `ACCT-B07` | Log in on a fresh browser/device with a synced passkey. | Root missing; account ready/unhydrated/unconfigured; service online/offline. | Whole browser login is exercised by multi-device flows. | Wrong passkey, cancel, unknown account, link succeeds but local attach fails, hydration timeout/restart. |
| `ACCT-B08` | Log out and sign back into the same account/profile. | Same root and server device row; local provider removed. | Whole real-browser regression; asserts one device row. | Offline logout, lost detach, reload during logout/login, concurrent revoke/delete, stale local grant. |
| `ACCT-B09` | Attempt login with a different account/root in a used profile. | Provider-free old root; registered old account; fresh profile available. | Partial unit/flow logic. | Explicit refusal text, zero writes, recovery through Add profile, wrong-root concurrent tabs. |
| `ACCT-B10` | Add a second browser account/profile and switch between them. | One/two accounts; disjoint and same-named spaces; one unavailable account. | Whole happy path for two accounts and disjoint space lists. | Switch failure/reload, storage failure, deleted/revoked target, name collisions, rapid repeated switch. |
| `ACCT-B11` | Load settings when account state is unconfigured or unhydrated. | `I2`, `I3`, provider online/offline. | Browser regression navigates to settings before email confirmation, waits through the enrollment race, requires email-verification guidance, and keeps display-name editing disabled; partial rendering/status tests cover other states. | Full offline-to-online convergence, malformed descriptor, Suspended recovery, and service return after offline. |
| `ACCT-B12` | View customer/account facts while service is unavailable or suspended. | `C2`, `C3`, `CX`; repository ready/unhydrated. | Partial DOM rendering; no journey fault injection. | Passkey facts remain available, email becomes unavailable, recovery and action gating. |
| `ACCT-B13` | Change the account display name. | Ready/unhydrated/revoked; Enter/blur; same/different value. | Focused browser coverage proves Enter ends the edit through the existing change-save path and the unfocused field has no decorative cursor; pending-verification coverage proves the field remains disabled, and presenter unit tests mask transport details. | Ready-state remote reject/conflict, duplicate Enter+blur, response loss, second-device concurrent rename, restart. |
| `ACCT-B14` | Add another passkey. | Active customer; missing/legacy passkey metadata; PRF/cancel/error. | Partial custody browser coverage plus presenter unit tests for cancellation, unsupported security capability, and unavailable browser integration. | Whole ceremony matrix, duplicate credential, fact/publish failure, reload and retry. |
| `ACCT-B15` | Inspect account summary and passkey facts. | Service online/offline; metadata present/legacy; account unhydrated. | Partial DOM rendering. | API non-2xx/malformed body, stale local facts, loading/reload accessibility. |

## Accounts: CLI and browser handoff

| ID | User journey and entry | Starting variants | Existing evidence | Missing or weak evidence |
| --- | --- | --- | --- | --- |
| `ACCT-C01` | Run bare `tonk account` or `account status`. | Root missing, unregistered, unconfigured, unhydrated, ready, malformed state. | Partial unit/integration coverage. | Process output/exit/JSON for every state, offline guarantee, unsupported version/corruption. |
| `ACCT-C02` | Start `tonk account login` and approve in an already-linked browser. | Default page; `--via`; `--no-open`; custom name; TTY/pipe; Safari cross-scheme warning; no onboarding state; onboarding-created/claimed and legacy spaces. | Loopback bridge contract tests, whole happy hybrid flows, and integration coverage for repeatable created/legacy space rotation. | Real Safari/Chrome/Firefox bridge pass, OS-open failure, callback collision, timeout, malformed callback, restart after every write, browser completion of deferred invite-seed rotation. |
| `ACCT-C03` | Start CLI login from an unlinked browser, create/login there, then approve. | Fresh browser root missing/provider-free; activation pending/active. | Whole browser flow for registration before linking CLI. | Account ceremony failure while callback waits, reload losing callback query, different-account/profile choice. |
| `ACCT-C04` | Decline a waiting CLI in the browser. | Linked/unlinked browser; default/direct callback. | Whole real-browser decline flow. | Tab close, callback POST failure, repeat decline, CLI gone before decline, machine output. |
| `ACCT-C05` | Cancel CLI login with Ctrl-C before authorization. | Callback bound; browser unopened/open; TTY/pipe. | Whole process test for SIGINT during callback wait. | SIGTERM/SIGHUP/terminal close; persisted session invariant; browser later posting to dead callback. |
| `ACCT-C06` | Resume or restart CLI login after interruption/crash. | Before approval; grant received; partial root/provider/session; hydration/push. | Existing process test explicitly expects a fresh callback before approval. | All post-approval crash points; decision whether declared pending states resume or are removed. |
| `ACCT-C07` | Attempt CLI login while an account is already active. | Same account/different account; stale active state; provider offline. | Partial direct state check. | Process output, no browser open, no writes, concurrent logout/login. |
| `ACCT-C08` | Run `tonk account logout`. | Active; pending; signed out; provider online/offline; concurrent account command. | Partial account unit test for local preservation/idempotency. | Session transition owner tests, pending states, lock contention, crash, detach failure/retry, process output. |
| `ACCT-C09` | Run `tonk account sync`. | Ready/unhydrated/unconfigured; offline/revoked/diverged. | Partial account authority/state integration tests. | Process matrix and bounded timeout; retry after partial pull/mount/push. |
| `ACCT-C10` | List account devices in human or JSON output. | Online/offline local facts; duplicate facts; self/other/revoked. | Partial account/device integration tests. | Empty/malformed account, pull timeout warning streams, stable JSON, concurrent revocation. |
| `ACCT-C11` | Revoke a device from the CLI. | Self/other/already revoked/unknown DID; service online/offline. | Partial `revoked_device` integration; browser revokes CLI whole flow. | CLI process matrix, partial multi-service publish, retry/idempotency, local state after self-revoke. |
| `ACCT-C12` | Open account or hosted-space deletion from CLI. | Browser opens/fails; `--no-open`; signed out; stale subject. | Partial URL construction and browser deletion flow. | Safe URL encoding/redirect, process exit/output, target changed before browser loads. |

## Account authority and destructive actions

| ID | User journey and entry | Starting variants | Existing evidence | Missing or weak evidence |
| --- | --- | --- | --- | --- |
| `AUTH-01` | View devices and revoke another device in browser settings. | Self/other/unknown/already revoked; deep link. | Whole browser revoke-CLI flow; DOM list tests; presenter tests require response uncertainty to lead to refresh before retry. | Offline, partial service publish, reload, concurrent revoke. |
| `AUTH-02` | Revoke the current browser/device. | Only device/multiple devices; other active profile; customer states. | Partial handler/state logic. | Whole self-revoke and landing/recovery, local data boundaries, second device observation. |
| `AUTH-03` | Arrive through `?revoke=DID`. | Linked/unlinked browser; DID present/absent/stale/malformed. | Partial unit logic and happy browser flow. | Cancel consumes query, reload, target disappears concurrently, safe profile switching. |
| `AUTH-04` | Review and delete one owned hosted space. | Owned/joined/unknown/already deleted; stale plan; customer unavailable. | Partial deletion implementation and broader browser coverage. | Whole exact-scope deletion, no-passkey worker authorization, stale/changed plan, partial remote failure, retry. |
| `AUTH-05` | Review and delete the whole account. | Zero/many owned spaces; joined spaces; multiple profiles/devices. | Whole browser account deletion/reuse-email flow; presenter tests say that passkey cancellation deleted nothing and reserve uncertain wording for request/response failures. | Whole-browser passkey denial, plan failure, response loss, partial space purge, restart, unrelated boundaries. |
| `AUTH-06` | Recover after account deletion. | Deleted selected profile; another local profile; retained joined/local replicas. | Whole profile/email release path plus rotation unit behavior. | Offline reload, second device stale state, explicit status/error on next remote action. |

## Spaces: local lifecycle and selection

| ID | User journey and entry | Starting variants | Existing evidence | Missing or weak evidence |
| --- | --- | --- | --- | --- |
| `SPACE-01` | List spaces with human or JSON output. | Empty; registered; missing data; unregistered site; account-listed remote copy. | Broad process coverage in `cli_space`. | Locked/malformed registry, stable JSON version, concurrent registry mutation. |
| `SPACE-02` | Select a space via `--space`, environment, or nearest binding. | All precedence levels; nested/stale/missing binding. | Broad `cli_space` and context tests. | CWD/binding changes during long command, malformed env, machine error contract. |
| `SPACE-03` | Create a new local-only space while signed out. | Canonical/custom site; valid/invalid/colliding name. | Whole CLI process coverage; browser pre-activation flow covers local-first use, and a focused FABB browser regression keeps post-create renaming active through whitespace. | Crash between data/registry/binding, read-only/full disk, concurrent same name. |
| `SPACE-04` | Create an account-owned space while signed in. | Customer Active/Registered/Suspended/offline; ready/unhydrated account. | Partial authority and browser flows; the shared FABB rename regression proves internal whitespace does not commit early. | All service states, ownership transaction crash, lost response, retry without duplicate space. |
| `SPACE-05` | Adopt an existing site with `space new --site`. | Valid site; already registered; reserved canonical name; malformed/old data. | Partial CLI process/migration coverage. | Crash and concurrent adoption, wrong repository identity, read-only site. |
| `SPACE-06` | Bind a directory with `space use`; unbind it later. | Parent/nested/exact/vanished path. | Broad process coverage. | Concurrent binding writes, malformed registry, symlink/case/platform variants. |
| `SPACE-07` | Remove a space locally with confirmation. | CLI TTY/no TTY; browser keyboard/touch; yes/no; owned/listed/local-only; data missing. | Broad process coverage plus a whole-browser confirmation/removal regression covering modal focus, form submission, profile removal, and Hub disappearance. | Signal/crash during removal, failed partial filesystem delete, concurrent use/sync, unrelated-space browser boundary. |
| `SPACE-08` | Unregister but retain data with `space rm --keep-data`. | Canonical/custom site; re-adopt later; name reserved. | Whole process coverage. | Crash between unregister/report, concurrent adopt, account directory disagreement. |
| `SPACE-09` | Set home concepts or read/write space AGENTS claim. | Blank/existing home; notation/dry-run/no-sync; file/stdin. | Authoring/agents integration coverage. | Partial write/restart, invalid model, remote conflict, pipe/encoding/large file. |

## Spaces: account directory, sync, and collaboration

| ID | User journey and entry | Starting variants | Existing evidence | Missing or weak evidence |
| --- | --- | --- | --- | --- |
| `SPACE-10` | Link a local-only space to the active account. | Signed out; same/different owner; creating profile absent with repository signer retained; same-owner retry after sharing; customer states; provider offline. | Eight `space_link` tests, including signer recovery and post-invite retry, plus authority/browser happy paths. | Crash/retry at every ownership/hosting/listing/upstream stage, partial remote commit, concurrent link. |
| `SPACE-11` | List spaces from the account directory in the CLI, Hub, or FABB. | Empty; duplicate names; joined/owned; active space excluded; unreplicated target; offline cached; stale deletion. | Eleven `account_spaces` tests cover many list/pull cases; focused FABB query and browser-DOM frame regressions cover current directory rows, name mirrors, active exclusion, and vintage unnamed entries. | Process JSON/error matrix, concurrent rename/delete, corrupted local account branch, and whole-browser cross-device directory convergence. |
| `SPACE-12` | Pull an account space by name or subject. | Unique/ambiguous/missing name; existing local name/subject; offline/revoked. | Broad `account_spaces` integration coverage. | Crash between fetch/site/registry, disk failure, concurrent pull, stale account fact. |
| `SYNC-01` | Inspect status without mutating local main. | No upstream/synced/ahead/behind/diverged/unreachable/revoked. | Broad sync/status tests. | Stable process output for all states, corrupt head, concurrent upstream move. |
| `SYNC-02` | Push local main. | `R0`–`R6`; account customer states; invite authority. | Broad sync and access tests. | Lost response after commit, concurrent push, retry/idempotency, bounded timeout. |
| `SYNC-03` | Pull upstream main. | `R0`–`R6`; dirty/ahead/diverged; remote changes during fetch. | Broad sync tests. | Crash after fetch/before ref update, concurrent local write, explicit divergence recovery. |
| `SYNC-04` | Auto-pull/write/auto-push around a write. | `--dry-run`, `--no-sync`, `TONK_NO_SYNC`, offline, conflict. | Data verb and sync integration coverage. | Whole command × all remote states, response loss, local commit succeeds/push fails recovery. |
| `COLLAB-01` | Mint an audience-open invite. | Zero/one/many remotes; shortening on/off/offline; upstream states. | Broad invite/integration coverage. | Shortcut timeout/malformed answer, lost push, stable URL privacy, concurrent revoke. |
| `COLLAB-02` | Mint a recipient-root invite. | Valid/wrong DID; accountless/account recipient; remote variants. | Partial invite tests. | Full claim boundary, wrong recipient, root rotation, URL leakage. |
| `COLLAB-03` | Join or claim an invite into a named space. | Fresh/existing name; remote/no remote; malformed/expired/revoked/already claimed; no passkey account. | Join, profile, and authority tests; accountless claim terminates at the onboarding account and remains usable after account union. | Process error matrix, crash between claim and registration, response loss, concurrent claim, browser-only invite-seed rotation after native login. |
| `COLLAB-04` | Revoke an invite and observe access loss. | Before/after claim; local replica; online/offline recipient; several remotes. | Invite revocation integration and real-browser access cutoff. | Multi-remote partial publish, later reconnection, local-write semantics, idempotent revoke. |
| `COLLAB-05` | Claim while accountless, later link an account, recover on another device. | Provider-free root; customer activation states; second device. | Whole integration/browser flow exists for claim retention and backup. | Restart at each retention/push stage, duplicate claims, different-root account attempt. |

## Authoring, data, rendering, and transfer

| ID | User journey and entry | Starting variants | Existing evidence | Missing or weak evidence |
| --- | --- | --- | --- | --- |
| `DATA-01` | Define/list a concept. | Empty/existing name; field types/cardinality; notation/dry-run/sync. | Strong authoring/process coverage. | Concurrent definition, remote conflict, malformed Unicode/large schema, crash after commit. |
| `DATA-02` | Define/list a detail, directory, label, or title view. | Inline/file template; explicit/default anchor; entity-like anchor; blank/existing home; `--home`. | Strong authoring/render coverage, including reported anchor/entity identity and pre-write anchor rejection; a standard-library contract keeps the blank-space agent prompt's displayed and copied text identical. | Template read failure, atomic home replacement under sync failure, route visibility and real clipboard parity in a browser. |
| `DATA-03` | Assert a new entity through schema-derived flags. | Required/optional/many fields; help; notation/dry-run/no-sync/quiet. | Strong `data_verbs` coverage. | Pipe/TTY distinctions, concurrent supersede, response loss after push. |
| `DATA-04` | Update an existing entity. | Bookmark/URI; same values no-op; wrong concept; missing entity. | Strong data verb coverage. | Multi-writer conflict, auto-sync partial failure, durable no-op invariant. |
| `DATA-05` | Query a concept. | Empty/many/malformed schema; human/JSON; offline. | Strong data/schema coverage. | Huge output/broken pipe, concurrent write consistency, stable JSON compatibility. |
| `DATA-06` | Retract one field or an entire instance. | One/many cardinality; notation/dry-run/no-sync/quiet. | Strong data verb coverage. | Already retracted, concurrent assertion, push failure/retry, value-level limitation copy. |
| `DATA-07` | Evaluate notation from inline text, file, explicit `-`, or piped stdin. | Query/write/mixed; JSON/quiet/home/dry-run/no-sync. | Strong notation/site coverage. | Broken pipe, signal during eval/sync, huge input, file mutation mid-read. |
| `DATA-08` | Show schema, concept, entity, or view. | Missing/ambiguous names; JSON/notation/human. | Listing/schema tests. | Stable error/output matrix, huge values/broken pipe, concurrent schema change. |
| `DATA-09` | Render a route to stdout or a file. | Directory/detail/explicit view; zero/one/many matching views; mixed portal types; missing fields/view; output path errors. | Ten render integration tests cover ordered multi-view siblings and frame-wide portal mode. | Atomic output file, broken pipe, real-browser parity, concurrent source change. |
| `DATA-10` | Add/list/read a content-addressed blob. | Inferred/explicit type; dry-run; missing/corrupt file/blob; stdout pipe. | Ten blob integration tests. | Large stream/disk full, signal, content changed mid-read, broken pipe. |
| `DATA-11` | Export or import CSV. | stdout/file; branch; empty/malformed/duplicate rows; write flags. | Transfer integration tests. | Atomic output/import, partial row failure/retry, escaping/large file, concurrent branch. |

## CLI setup and maintenance

| ID | User journey and entry | Starting variants | Existing evidence | Missing or weak evidence |
| --- | --- | --- | --- | --- |
| `CLI-01` | Ask for index, command help, all commands, or guides. | TTY/pipe; known/unknown command/guide; dynamic concept help. | Parser/unit and process coverage. | Snapshot of every help route, width/color/broken pipe, hidden-command discoverability. |
| `CLI-02` | Receive a usage or runtime error. | Human/verbose; JSON-capable command; TTY/pipe. | Partial output tests. | One stable exit-code/stream taxonomy across every command family. |
| `CLI-03` | Add/list/select a remote and upstream. | Zero/one/many remotes; invalid URL/DID/name; existing upstream. | Remote/sync/invite tests. | Concurrent mutation, partial meta-branch write, wrong origin, JSON matrix. |
| `CLI-04` | Inspect or reset the local identity. | Missing/existing profile; spaces/accounts present; confirmation expectations. | Partial identity/profile tests. | Process-level destructive boundaries, recovery/redelegation guidance, read-only state. |
| `CLI-05` | Migrate a carry directory. | Found/not found; copy/move; cross-filesystem; destination exists. | Live migration tests. | Crash/restart, partial copy cleanup, permissions, concurrent writer. |
| `CLI-06` | Migrate legacy account delegations. | Empty/partial/already migrated; offline account; many spaces. | Authority migration tests. | Crash at each drain/retain stage, duplicate run, corrupt delegation, push failure. |
| `CLI-07` | Inspect/toggle telemetry. | default/env/DNT/persisted; on/off/status; write failure. | Three telemetry integration tests. | Complete precedence, malformed state, stable privacy output, concurrent write. |
| `CLI-08` | Update the binary or change background-check setting. | install script/npm/nix; newer/current; network/error/signature/swap failure. | Nine integration plus source unit tests. | Full process restart/swap rollback, interrupted download, platform/deployment smoke. |

## Browser shell and runtime

| ID | User journey and entry | Starting variants | Existing evidence | Missing or weak evidence |
| --- | --- | --- | --- | --- |
| `UI-01` | Boot the browser shell and mount the current route. | First install/return; SW controlled/not controlled/updating; offline/cache corrupt. | Partial deployment and broad E2E setup. | Explicit boot state machine, update/reload failure, accessibility, recovery from stale assets. |
| `UI-02` | Route `/settings*`, `/activate*`, or content through the correct top-level surface. | Canonical/legacy/unknown/deep route; query/hash. | Partial route unit + account browser test. | Real mount assertions for every family, malformed URL, history back/forward. |
| `UI-03` | Pass account gating and enter local Hub/content. | Root missing/provider-free/registered; customer states; revoked. | Partial `account_gate` test and broader browser flows. | Full state matrix, offline local-first guarantee, gate recovery without reload. |
| `UI-04` | Open a space home, explicit rendered route, or `/inspector`. | Blank/configured home; missing view/entity; local/remote; unauthorized; named-space/profile inspector. | Rendering/portal coverage, inspector renderer tests, some E2E paths, and structural coverage for the actionable Tonk edge wall on an absent-space route. | Top-level real-browser route matrix, configured-upstream probe, and interactive behavior beyond headless render. |
| `UI-05` | Recover from service-worker or deployment configuration failure. | Missing env, wrong origins/DIDs, worker exception, stale cache. | One deployment browser test plus build checks. | Visible failure/retry, no infinite boot, diagnostics without leaking secrets. |

## Coverage conclusion

The suite is large, but the distribution explains the reported hot-path bugs:
happy CLI operations have broad integration coverage while account session
transitions, activation, response-class mapping, restart points, concurrency,
and destructive boundaries are much thinner. The highest-return work is not
more undirected tests; it is filling the P0 state/event cells named here.

Source audit pinned to Tonk commit `a3f8670b1`. No catalog row has yet been
promoted to verified in this storybook.
Onboarding-account addendum pinned to Tonk commit `b564e83b1`.
