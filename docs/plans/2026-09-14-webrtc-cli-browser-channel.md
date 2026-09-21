# A WebRTC data channel between `tonk` and a browser tab

Status: paused as a draft implementation on 2026-09-21. The ordinary Network-page
connection and online account-authorized repository/blob sync work, but the MVP
release gates below are not all complete. The broader C1-C7 contract was approved
for implementation on 2026-09-20 in `feat/rtc-finish`, based on `4302dd9d`; it is
now retained as design context rather than the scope of this draft. See the
implementation checkpoint for current evidence. The investigation following it
records earlier designs and measurements, some of which have been superseded.

## Completion contract (approved 2026-09-20)

### Intended outcome and proposed scope

A running `tonk` can serve its local spaces as a sync remote for the browser.
On the same machine, this must work without internet after the application and
the required authority have been provisioned. It lets browser views and local
CLI work share repository data without requiring a hosted storage service for
each transfer. Connecting does not confer permission to read or write a space.

The architecture already chosen in this document remains:

```text
browser views / commands
        |
service worker: repository, signed Dialog effects, iroh endpoint
        | MessagePort datagrams
browser page: RTCPeerConnection + unreliable, unordered data channel
        | WebRTC carrying iroh/QUIC
tonk rtc serve: verify invocation -> mounted local repository
```

WebRTC supplies the browser's local network path; iroh supplies authenticated
connections and streams; Dialog supplies repository operations and capability
checks. The page is a carrier, and does not receive the worker's signing keys.
The public rendezvous certificate is reproducible by anyone and establishes
neither the intended CLI's identity nor permission to use its repositories.

**Proposed first completion milestone:** a supported browser can connect to an
explicitly identified CLI, attach an already-authorized space as a peer remote,
and synchronize edits and blobs in both directions offline. Closing a tab or
restarting the CLI must produce recovery or a bounded, retryable failure.
Pasting the CLI's peer URI once is acceptable for this milestone; executing
JavaScript manually is not.

LAN configuration, internet discovery/relays, native CLI-to-CLI dialing, and
serving browser-held repositories to other peers are subsequent milestones.
They are discussed historically below but are not established by the current
product wiring. In particular, both product endpoint builders currently use
`presets::Empty`; pkarr, mDNS, relay fallback and automatic remote discovery
must not be described as shipped. This scope split is a proposal for review,
not a claim that the wider design has been completed.

### MVP scope freeze (2026-09-21)

This draft is intentionally narrower than C1-C7. The MVP is one supported
Chromium browser tab connecting to a local `tonk rtc serve`, selecting an
already-authorized space, and synchronizing edits and blobs in both directions
without replacing its separately configured cloud remote. It is experimental,
opt-in, loopback-only, and may require an explicit reconnect after a transport
failure.

The MVP release gates are:

1. The ordinary Network UI connects to the explicitly identified CLI and stores
   the peer remote without granting new content authority.
2. Existing account authority is enforced for every operation. Ordinary local
   discovery may reveal mounted space names and DIDs, but discovery alone never
   grants content access.
3. Browser edits, CLI edits, and multi-megabyte blobs synchronize in both
   directions while preserving cloud configuration and device-local `meta`.
4. Previously verified authority remains usable after restarting both the CLI
   and browser worker with the authority service unavailable. Unknown evidence
   fails closed. Once a revocation is learned, later writes are denied and that
   decision survives restart.
5. An interrupted write cannot commit partial or corrupt content. A retry uses
   repository compare-and-swap/reconciliation semantics and reports an unresolved
   outcome rather than blindly replaying a possibly applied mutation.
6. Connections, pending acquisitions, streams, buffers, and idle/abandoned work
   have conservative bounds and are reclaimed so repeated failures cannot cause
   unbounded growth.
7. A focused real-profile end-to-end test covers the supported journey, including
   the offline restart and failure boundaries above. The draft PR must state any
   gate that remains incomplete and must not be represented as release-ready.

Deferred beyond the MVP:

- Firefox, WebKit, and real Safari support or compatibility guarantees.
- Seamless multi-tab failover, frozen-tab transfer recovery, and automatic
  reconnection beyond the behavior already implemented.
- LAN listening, discovery/relays, native CLI-to-CLI dialing, and serving
  browser-held repositories to other peers.
- The exhaustive browser/network matrix, performance tuning, and broader UI
  polish described by C5-C7.

At the pause point, gates 1-3 are demonstrated by the real account fixture and
the focused transport/product tests. Gate 4 still needs the complete offline
CLI-and-worker restart journey and a revocation/write-boundary test. Gate 5
still needs interrupted-transfer/CAS coverage. Gate 6 still needs native
connection/stream/request bounds and abandoned-transfer cleanup. Gate 7 remains
incomplete until those focused tests pass. This is the handoff boundary for the
draft PR; work outside these gates should not be pulled into the MVP.

### Baseline at `4302dd9d` (before completion work)

Paths below are relative to this document. Dependency observations refer to
Dialog commit `991ec542a6c503968a415a1601a4c4b5a543778a`, the revision in
`Cargo.lock`; that exact source was inspected from the local Dialog repository.

| Area | Observed implementation and remaining gap |
| --- | --- |
| Listener and protocol | [`rtc.rs`](../../rust/tonk-cli/src/rtc.rs) mounts the space registry and serves signed Dialog requests. The pinned `dialog-iroh-remote` includes archive/memory operations and streaming blobs; these do not need designing from scratch. The registry is a startup snapshot, and the command still opens a selected space before starting. |
| Browser carrier | [`rtc.mjs`](../../rust/tonk-ui/assets/rtc.mjs) implements direct dialing and `attachCarrier()`. No production caller opens that carrier; the only external call found is in the app test. Posting the port has no worker-ready acknowledgment. |
| Worker integration | [`router/cli.rs`](../../rust/tonk-worker/src/router/cli.rs) attaches carriers and supplies the operator's iroh site. `ReachPeer` asks for inventory, but publishes only reachable/unreachable. Names, counts, identity and failure detail are discarded from the overlay. |
| Network UI | [`profile.yaml`](../../rust/tonk-core/assets/library/profile.yaml) defines `/network` with a peer input and an ask action in its initial empty state. It does not connect a carrier or attach a space. Once a status row exists, its template has no retry form. `PeerIdentity` exists but is never asserted by the worker. |
| Addressing | The worker uses one phrase-derived listener route for every carrier. The real CLI advertises an encoded address and keys accepted carriers by ICE ufrag; the example listener uses phrase-derived tags instead. `reach()` parses a supplied URI but discards its route hints. Tests against the example therefore do not prove the real CLI's route integration. |
| Multiple processes and tabs | Sixteen candidate ports are raced in one ICE connection, which chooses one path rather than enumerating sixteen identities. The transport stores one carrier per route; a new attachment replaces the old one, and an old close can remove the replacement. [`dispatch.rs`](../../rust/tonk-rtc/src/dispatch.rs) has policy tests but no production caller. |
| Remote configuration | [`remote.rs`](../../rust/tonk-cli/src/remote.rs) accepts peer URIs. The browser's [`invite_configuration()`](../../rust/tonk-worker/src/router/join.rs) still constructs a `UcanAddress` for every invite remote. Parsing a peer URI is not yet a complete browser remote/join workflow. |
| Authorization | The CLI uses `Responder::new`, whose pinned dependency default is `UnverifiedRevocations` (every revocation query returns none). `Served::Spaces` returns the whole mounted inventory to any verified caller, while the UDP listener binds `0.0.0.0`. Signature verification alone does not establish inventory consent or enforce revocation. |
| Tests and packaging | The [app harness](../../rust/tonk-ui/tests/e2e/cli-status-app.mjs) injects the carrier, defaults to an example server, and calls removed `/api/cli/status` and `/api/cli/spaces` routes. Native transport tests cover a stream, a signed block put and a greeting, not a complete browser repository sync. The Nix CLI package omits the optional `rtc` feature. |

### Required behavior

#### C1. A complete connect action

The `/network` action must accept the peer URI printed by `tonk rtc serve`,
request a carrier from a live page, wait for the worker to acknowledge its
registration, then issue the signed probe through the existing command system.
An existing peer remote needing sync must be able to request the same mechanism.
Opening an unrelated page need not start a perpetual discovery loop.

Keep the connect/retry action available after every result. Render distinct
states for not checked, connecting, connected, unavailable, invalid identity or
address, and refused access. Provide useful failure detail and the permitted
inventory. A connected result identifies the authenticated endpoint; a greeting's
`subject` echoes the caller's subject and must not be presented as proof that
the CLI owns that subject.

Live status, inventory observations and carrier IDs belong in the session
overlay. Clear stale observations on disconnect, profile change and worker
replacement. Persist only intentional peer/remote configuration and authority.
Late results from an old attempt must not overwrite a newer result.

Acceptance: launch the real CLI, open the ordinary app, paste its URI and use
the action successfully without a console, injected carrier or fixture endpoint.

#### C2. Identity selects the peer; routes reach it

Preserve and validate the URI's route hints, including an explicit port or
private certificate. Define one versioned representation shared by the CLI,
page and worker, and ensure the carrier is attached under the address the
endpoint actually dials. Reject unsupported or malformed routes explicitly.

Pin the expected endpoint key before trusting replies. A successful rendezvous
only found a transport: when several CLIs occupy the port span, select or probe
individual routes until the requested identity is reached. Never silently accept
whichever listener won the ICE race. An identity change requires relinking,
while a route change must not orphan the existing remote credentials.

The CLI endpoint key must survive restarts per profile. Distinguish a missing
key from a corrupt/unreadable key; concurrent starts must not mint competing
identities for one profile. The browser remains a dialer for this milestone:
its current derivation can vary between sessions on some engines, and it must
not be advertised as a durable server identity.

Acceptance: two CLI profiles on one machine, an occupied first port, an explicit
non-default port, a private certificate, and a wrong endpoint key all select
the intended peer or return an actionable refusal.

#### C3. Authorization and disclosure have explicit policies

Keep per-invocation signature, subject, scope, expiry, payload-integrity and
revocation checks. Replace the production `UnverifiedRevocations` default with
an explicit policy using the project's revocation mechanisms. A known revoked
delegation must always fail. Offline verification must obtain and retain the
needed evidence under the cache decision below; an unavailable lookup may reuse
verified evidence but must not turn unknown status into approval. The acceptance fixture must
exercise the chosen policy, including any DID-resolution evidence it needs.

Decision (2026-09-20): retain ordinary peer discovery, without a separate browser
approval ceremony. Explicitly running `tonk rtc serve` discloses its mounted space
names and DIDs to connected local browsers. This is metadata disclosure, not
evidence of access to those spaces. The local milestone binds loopback only;
broader listening is out of scope and requires its own disclosure policy.
Loopback reachability alone is not account identity.

Decision (2026-09-20): previously verified revocation evidence may be cached
indefinitely for offline use. Unknown evidence is not an approval. On returning
online, refresh that evidence; once a revocation is learned, deny subsequent
operations (including writes), persist the revocation across restarts, and never
replace it with an older cached approval. A fully offline peer cannot detect a
revocation it has not learned about. This cache policy does not extend a token's
expiry, expand its scope, or bypass signature and payload-integrity checks.

Selecting an inventory row must reuse existing account/invite authority or
report that access is needed. It must never mint access merely because a peer
was found. Verify account switching, device logout and removal cannot leave a
carrier or mounted server holding authority from the previous account.

Acceptance: authorized reads/writes succeed; wrong-subject, insufficient-scope,
expired, tampered and revoked requests fail without changing data. Unavailable
verification is distinguishable from a disconnected transport. Test metadata
disclosure separately from content access.

#### C4. A space is usable through the peer remote

Provide a supported command/UI path to register an already-authorized space
against the selected peer and configure its branch upstream. Adding a peer
must not overwrite a separately configured cloud remote. Peer addresses must
retain their transport type through storage, reload and any supported invite
path; the browser must not reinterpret them as access-service URLs.

Prove repository pull and push, shared content metadata and blobs through the
real worker and real CLI storage. Keep the device-local `meta` branch (replica,
remotes and tracking configuration) local, matching ordinary cloud sync; it is
not a shared content branch. Preserve branch compare-and-swap behavior,
content verification and the repository's ordering rules during interruption.
Do not count a successful greeting or a block put as complete sync. Surface
permission errors and unresolved outcomes; retry using repository semantics
rather than treating every failed mutation as unapplied.

A missing peer may leave local edits available and pending sync. It must not
silently change upstream or claim that cloud fallback exists. Any fallback
must use a separately configured, authorized remote with documented semantics.

Acceptance: a browser edit reaches CLI storage, a CLI edit appears in the
browser after sync, a multi-megabyte blob round-trips with its digest intact,
and a restarted browser/CLI can use the saved remote configuration offline.

#### C5. Carrier recovery is part of the transport

Track carriers by peer, client and generation. Either retain multiple carriers
with a selection policy, or explicitly replace an active one. In either case,
a stale close/error cannot detach a newer carrier. Detect silent tab loss or
freezing as well as orderly data-channel closure. Prefer a live visible page
and request a replacement when the current carrier stops responding.

Reconnect after CLI restart, page reload, service-worker `controllerchange`,
and sleep/resume. Cancel obsolete dials and release peer connections, ports,
listeners and callbacks. Revalidate the current profile before reattachment.
Use bounded dial/probe deadlines and capped retries; a page with no running CLI
must settle into a useful state. Do not copy the old per-effect dispatcher into
the datagram path without reconciling it with iroh's connection/stream behavior.

Acceptance: close or freeze the active carrier tab during a transfer with a
second tab available; recover without corrupting data. Close the old carrier
after its replacement attaches; the replacement must remain usable. With no
carrier left, fail within the documented deadline and recover on a later retry.

#### C6. The long-running listener has bounded resource use

Bound pending dial announcements and remembered ufrags, expire failed or idle
handshakes, and reclaim routes when connections end. The current 32-connection
limit only removes `Closed` peers and does not bound the mux's remembered set
or announcement queue. Exercise failed handshakes as well as successful dials.
Account for browser `bufferedAmount` and MessagePort backlog alongside Rust's
bounded datagram queues. Define limits and cleanup for concurrent streams and
large/abandoned requests; dropping datagrams under load must remain recoverable.

For the first milestone, document the served registry as a startup snapshot;
report mount failures and require restart to include new registrations. Account
or authorization changes still require immediate invalidation. Start the server
without requiring an arbitrary selected space, and make empty-registry behavior
and graceful shutdown explicit.

Acceptance: repeated failed dials and tab churn do not permanently exhaust the
listener or grow retained state without bound; healthy clients still connect
after pressure subsides. Termination releases the listening port.

#### C7. Shipping and verification match the supported behavior

Replace the stale app harness with a test that launches `tonk rtc serve` against
isolated real profiles/spaces and uses the `/network` command/UI. Retain the
example listener only for transport isolation. Run the same product journey in
Chromium, Firefox and WebKit, and record a real Safari HTTPS-to-local-CLI check;
the historical engine measurements alone do not establish current app behavior.

Test offline after installing a complete app generation, with external network
access blocked and loopback still allowed. Check `rtc.mjs`, `rendezvous.der`,
worker code and required authority survive offline reload. The existing asset
stamper inventories copied files; verify this in the built product rather than
assuming that a source-tree certificate implies an offline-ready application.

Add explicit CI coverage for the `rtc` CLI feature and native transport tests,
the current browser workflow, and wasm compilation. Keep ordinary builds
covered too. Decide when release packaging enables `rtc`; do not advertise the
command while release artifacts omit it. Review and pin the companion Dialog
changes, update obsolete comments/help, and measure the actual worker bundle.

Acceptance evidence must name commands, artifact revisions, browser versions,
and which failures were exercised. A supported release requires C1–C6 plus the
offline/browser matrix above, rather than a green transport-only smoke test.

### Delivery order and decisions to settle

1. Agree the local-first milestone, inventory disclosure policy and offline
   revocation rules. These determine the meaning of a successful connection.
2. Unify peer/carrier addressing and wire the real connect action (C1–C3), with
   a real CLI/browser test proving that boundary.
3. Complete peer-remote registration and authorized repository/blob sync (C4).
4. Exercise interruption, multiple tabs/processes and resource cleanup (C5–C6).
5. Run the supported browser/offline matrix and finish dependency/release work
   (C7), then consider the wider networking milestones.

The first milestone is local-only. Inventory disclosure and indefinite offline
caching were resolved above by the user on 2026-09-20. The implementation must
make these policies explicit rather than accidentally inheriting transport or
verification defaults.

### Verification performed for this review

- `node --test rust/tonk-ui/tests/rtc.test.mjs`: **16 passed** on 2026-09-20.
  These test encoding, SDP construction and rendezvous helpers, not the product
  connection or sync workflow.
- `cargo test --offline --locked -p tonk-rtc --features iroh`: could not start;
  the locked Dialog revision is absent from Cargo's checkout cache. Source
  inspection of that revision was possible in the separate local Dialog repo.
  No Rust pass/fail claim is made about the branch itself.
- Browser end-to-end tests and full builds were not run in this review. The
  checked-in app harness's removed-route and fixture-server mismatches were
  established by source inspection. Historical successful measurements below
  have not been repeated at this revision.

### Implementation checkpoint (2026-09-21)

Work is isolated in `/Users/goblin/Workspace/tonk-rtc-finish`, branch
`feat/rtc-finish`, with uncommitted changes on `4302dd9d`. The original `feat/rtc`
worktree is unchanged. This is a working connection foundation, **not completion
of C1–C7 or a claim that authorized offline sync ships**.

Implemented:

- The ordinary `/network` command requests a carrier from a controlled top-level
  page over a private reply port, attaches its exact route, acknowledges readiness,
  and probes the real CLI through signed Dialog effects. The UI retains retry,
  shows the authenticated endpoint and permitted inventory, and clears stale
  identity/inventory on failure. The event binding now reads `.timeStamp` rather
  than trying to coerce the literal `Date.now()` into a number.
- Shared versioned route validation preserves the advertised port and certificate
  fingerprint. The worker accepts local routes only and pins the requested iroh
  endpoint. CLI endpoint-key creation is serialized and corrupt keys are refused,
  not replaced. Explicit installation/test profile directories also isolate
  private certificates.
- Route leases prevent a stale close, queued datagram, or old observation from
  affecting a replacement. Page/controller changes cancel carriers; MessagePort
  heartbeats detect silent loss. Dial, registration and probe deadlines are
  bounded. Recovery currently requires an explicit retry, not automatic failover.
- Listener announcements, remembered ufrags, accepted channels, MessagePort
  traffic and SCTP buffers are bounded. Failed/stalled handshakes expire; listener
  drop releases the socket. QUIC datagrams preserve segmentation and oversized
  packets are dropped instead of truncated.
- The app test launches the actual CLI with isolated storage and uses the normal
  form. It no longer injects a carrier or calls the removed status routes.
- The CLI now replaces `UnverifiedRevocations` with a persistent checker. A
  bounded, versioned `POST /ucan/revocations` lookup reads the same access-service
  index used by normal invocation verification. Services come from local account
  configuration and mounted spaces' UCAN remotes, never from an incoming request.
  Every configured service must establish approval. HTTPS is required except
  for development loopback; redirects, malformed replies, unsupported services
  and corrupt caches fail closed.
- Verified revocation evidence has no age expiry. Unavailable services may use
  exact previously verified evidence or absence from a verified complete target
  snapshot; unknown evidence is not approval. Learned
  revocations dominate later approvals across cache reopenings and service
  sources. Atomic replacement and a cross-process lock merge concurrent updates.
  Positive entries are capacity-bounded (4,096); evicting one fails closed until
  refreshed. Negative entries are never evicted. Normal signature, scope and
  token-expiration checks remain mandatory.
- Revocation replies optionally carry the service's complete set of target CIDs,
  without principals or delegation bytes. This is shared revocation evidence,
  not an exception for session grants: newly minted session proofs can establish
  absence against the last verified service state while offline. Snapshots are
  monotonic locally; a newly observed target invalidates old exact approvals
  until its relevant revoker is checked. Old responses cannot restore absence.
  Snapshot work/output is bounded (8,192 targets/facts, bounded KV pages, 2 MiB
  response); an incomplete or over-limit inventory is omitted, never truncated
  into false absence. Exact checks remain available when no snapshot is supplied.
- The CLI now persists validated `did:web`/`did:plc` documents for offline
  resolution. It refreshes online, refuses redirects and oversized documents,
  and durably invalidates old keys after a current refusal or invalid document.
  Key/controller/fragment validation and signature verification remain Dialog's.
  The cache is bounded (128 entries, 8 MiB); unknown/corrupt evidence fails closed.
  Refreshes are serialized across processes and bounded within one process.
- Public discovery uses a self-signed profile invocation for that profile's own
  subject, rather than a delegated operator session. It passes the same strict
  UCAN verifier without requiring revocation evidence merely to list names/IDs;
  it does not confer content authority or special-case content verification.
- Serving is bound to the account attachment mounted at startup. Existing
  account-session locks guard effects, each blob chunk and the final blob commit.
  Logout or replacement refuses further operations from those old mounts without
  waiting for an idle peer to finish a stream. Restarting mounts the new account.
- CLI remote enumeration, upstream metadata and browser invite/reload paths now
  preserve peer addresses as `Iroh`, including their route hints. Setting a peer
  upstream retains a separately registered cloud remote.
- The Network inventory now offers `tonk:attach-peer` (profile-only), selecting
  a peer upstream for an already-authorized space. It requires the latest observed
  URI/subject and existing proof chains for content access; missing access is an
  overlay refusal, not a new delegation. An authorized but unmounted directory
  space uses ordinary verifier-only adoption, after the proof check. The command
  bypasses cloud provisioning and preserves separately configured cloud remotes,
  their execution metadata and the saved root prefix. Peer remote names derive
  from endpoint identity; route updates use the normal cell CAS and refuse an
  identity/subject conflict. Effective configuration is mirrored using the normal
  directory convention so adoption does not silently restore the cloud upstream.
  A directory-write failure is reported as a partial configuration requiring
  retry, not success. Actual authorized content/blob sync now has the focused
  account fixture described below; offline account restart remains unverified.
- The real account fixture exposed an incorrect initial assumption: `meta` is
  this device's replica/remote/tracking state, not a shared metadata branch.
  Peer selection now configures `main` only, matching ordinary cloud setup;
  device-local `meta` is not replicated. Shared content metadata stays on the
  content branch. The contract and attachment regressions reflect this correction.
- Saved peer remotes acquire a carrier at channel-open time, not only through
  Network's connect button. Profile-bound contexts avoid reacquiring AppState
  during repository I/O. Dials coalesce per endpoint; at most four peers,
  64 pending acquisitions and 64 open transfers are retained. Acquisition is
  bounded at 30 seconds, stream I/O at 30 seconds per operation. Errors retire
  the exact carrier generation; possibly applied mutations are never replayed
  by the transport. Profile changes invalidate I/O before and after each await.
- Carrier selection tries up to three current-profile controlled pages (visible,
  initiating, focused preferred), with an eight-second deadline per candidate.
  Superseded attempts cancel even before ICE completes. Page heartbeats detect
  tab loss; the Network observation also checks the CLI with a bounded signed
  read-only hello, since an alive page alone does not prove the CLI is alive.
- Offline scheduling now checks the selected branch's stored transport. Explicit
  loopback peer and local upstreams remain eligible; cloud/account sweeps wait
  for internet. Missing or malformed configuration fails closed, no upstream is
  switched implicitly, and unsynced cloud edits stay queued.
- Serving now opens a profile-wide context without a selected repository. Empty
  installations answer discovery with an empty inventory, without manufacturing
  a space or requiring an account. Broken mounts are reported and skipped;
  startup output explicitly says that new registrations require a restart.
- SDP fingerprint octets are normalized to uppercase at the browser boundary
  (Firefox rejects the native lowercase spelling). The saved route is unchanged.
  Fresh ICE credentials use the ICE/base64 alphabet, not base64url.
- Private certificate creation is now locked across concurrent callers,
  owner-only and atomically persisted before advertising a route. Corrupt or
  unreadable certificates are preserved and refused, not silently regenerated;
  persistence failure no longer leaves an ephemeral private route running.

Verification so far (all against the worktree, locked Dialog revision
`991ec542a6c503968a415a1601a4c4b5a543778a`):

- `cargo test --locked -p tonk-rtc --features iroh`: **68 passed** (65 unit,
  two Dialog-over-WebRTC, one iroh stream).
- `cargo test --locked -p tonk-worker --test standard_library`: **32 passed**,
  including the peer-form timestamp regression.
- Worker library tests: **159 passed**; peer schema tests: **2 passed**;
  latest native worker `router::cli::` regressions: **5 passed**, including stable
  remote naming, stale/undisclosed inventory rejection, route updates and conflict
  refusal while preserving the cloud remote. The invite peer-type reload
  regression also passes. **Seven browser service-worker CLI tests pass**:
  peer configuration persists after reload/reconciliation, existing cloud and
  authority remain, authorized unmounted directory spaces are adopted, and
  undisclosed, unjoined, content-origin and verifier-only replicas cannot register
  a peer remote. Demand-context generation, bounded capacity and transfer
  invalidation are covered too. Two transport-aware offline selection tests pass,
  including exact upstream selection without implicit cloud fallback. Three session-renewal and three profile
  browser regressions also pass. Browser-test compilation exposed a native-only
  access-service fixture linked on wasm (conflicting `wasm-streams` exports) and
  stale RTC state/session test constructors; target-scoping that fixture and
  updating the constructors makes the wasm tests build without dependency changes.
- `cargo test --locked -p tonk-cli --features rtc --lib`: **236 passed**, including
  **12 RTC**, **5 revocation**, **5 DID-resolution** and **5 remote-address** tests.
  `cargo test --locked -p tonk-cli --no-default-features --lib`: **223 passed**.
  `cargo check --locked -p tonk-cli --no-default-features` also passes.
  A real signed responder/storage test
  verifies online writes, offline cached writes, refusal with no evidence, and
  refusal without storage changes after learning revocation and reopening the
  cache, including new operator keys and delegation CIDs created while offline.
  Account lifecycle coverage includes logout with an already-open blob
  and replacement with a new attachment of the same root.
  The new combined evidence test uses real signatures and storage with a
  `did:web` invocation signer. Both caches are reopened for each request; fresh
  proof CIDs still work offline. Online and offline variants reject wrong
  subject, read-only scope, expired invocation, expired delegation, invalid
  signature and substituted payload, checking neither signed nor substituted
  content reached storage. The DID response provider is isolated test data;
  this is not yet the browser account-handoff/repository-sync fixture.
- Identity evidence tests: **7 passed**; access-service revocation tests:
  **17 passed** (`--features helpers`, required for the native test runtime).
  The production access-service WebAssembly target also passes `cargo check`.
- `node --test rust/tonk-ui/tests/rtc.test.mjs rust/tonk-ui/tests/boot-script.test.mjs`:
  **40 passed**, including cancellation, registration acknowledgment, stale
  controller rejection, timeout cleanup, heartbeat expiry and queue pressure.
- `cargo build --locked -p tonk-cli --features rtc --bin tonk` and the full Trunk
  app build succeed. The local wasm build requires the Nix LLVM compiler **and
  archiver**, not Apple's default archiver. Latest app build: `e94a7b5a3c447b98`;
  `worker_bg.wasm`: **33,494,323 bytes** (development artifact, not release-size
  evidence). The built manifest contains `rtc-carrier.mjs`, `rtc.mjs`, and
  `rendezvous.der`, and their SHA-256 hashes match the copied files. Runtime
  offline-reload evidence is recorded separately below.
- `node rust/tonk-ui/tests/e2e/cli-status-app.mjs` in Chromium **147.0.7727.15**:
  the actual Network form connects, lists the fixture, rejects invalid input,
  retries, detects CLI shutdown, and reconnects after restart with the same URI.
  Both the public certificate and `RTC_PRIVATE=1` runs reject a wrong endpoint
  key on the otherwise correct route, and recover by explicit retry in a second
  tab after closing the carrier tab. Both were repeated successfully with the
  strict revocation checker and self-signed discovery enabled. The fixture uses
  isolated device-root authority; it is **not** an account-authorized sync test.
- Build `e94a7b5a3c447b98` passes the private-certificate, two-CLI Chromium journey
  with the new inventory button: selecting an unjoined offered space reports
  `access needed`. Its worker SHA-256 is
  `24b89cbb84783e69ae205fd17b5da4bdc8b66fd01391c5827922136f60e32a34`.
  The same build passes WebKit 26.5 with two CLIs and the offline-reload journey
  (`RTC_OFFLINE=1`), including the new inventory refusal action before isolation.
- The latest build also passes the complete connection/retry/restart/tab-close
  journey in **WebKit 26.5** with a real registered space, and in Chromium with
  `RTC_EMPTY=1`: no account, root override, selected space or registered space.
  The harness uses a disposable persistent browser installation. WebKit's
  ephemeral context failed to open OPFS; the persisted context works.
- The expanded two-CLI fixture also passes in Chromium with **two private
  certificates**, and in WebKit with public rendezvous certificates. It checks
  different profile endpoint IDs and ports, selects each through the normal
  Network action, and confirms the inventory switches to the requested peer
  without retaining the other peer's rows. A success assertion now waits for
  the requested identity, not a previous peer's still-rendered status.
- `RTC_OFFLINE=1` passes the entire journey in Chromium with both public and
  private certificates and in **WebKit 26.5** with public certificates,
  including two CLI profiles, cached app reload, invalid
  input, wrong endpoint, retry, CLI restart and carrier-tab replacement. Before
  disconnecting, it waits for the exact build's **adopted** generation marker
  and checks all three RTC assets plus worker Wasm are retained. The app origin
  is stopped and external browser HTTP is refused; reload makes zero requests
  to the asset server. This is not OS-wide network isolation, does not force
  `navigator.onLine=false`, and does not exercise account-authorized sync.
  Playwright's generic offline emulation disrupted loopback dialing and WebKit
  navigation; it is not used to simulate the local-first network boundary.
  The fixture also permits local `blob:` modules: WebKit routes them through
  the request interceptor even though they use transferred cached bytes.
- **Firefox 153.0 does not pass.** After fixing its SDP fingerprint parse error,
  it accepts the SDP but produces no ICE candidate pair for the loopback route.
  A separate disposable direct-dial/UDP-observation fixture received zero
  packets; browser stats and ICE logs confirmed an empty checklist. Granting
  local-network permission did not resolve it. Temporary loopback/host-address
  diagnostic preferences also did not resolve it. No preference override or
  additional instrumentation was added to the product, and the listener was
  not broadened to LAN to make the test pass. This remains a C7 compatibility
  failure, not a passing or silently skipped test.
- `node --test --test-timeout=90000 rust/tonk-ui/tests/*.test.mjs`:
  **160 passed**, zero failures/cancellations, in approximately 60 seconds.
  The earlier 30-second cap prematurely cancelled two service-worker test files;
  both pass unchanged with sufficient time. The isolated service-worker suite
  also passes all **72 tests**.

- Latest product build: **`cb08eca7e2cfe82a`**, worker Wasm **33,616,309 bytes**,
  SHA-256 `6247bf7ee06319ffe7dbf94b3e7feff6e30f2d2230fd77684a17ed02e151d20d`.
  The ordinary CLI workflow passes on Chromium **147.0.7727.15** with private
  certificates and offline reload, and WebKit **26.5** with public certificates
  and offline reload. Both include two CLIs, wrong identity, retry, disconnect,
  CLI restart and carrier-tab replacement. A regression caught detached status
  writes not flushing subscription polls; the fix is included in this build.
- The real account fixture `it_syncs_an_authorized_space_through_the_local_cli`
  passes on Chrome **151.0.7922.176** through the harness's local HTTPS origin.
  Ordinary signup, account activation and browser approval link the CLI; ordinary
  account-space pull registers its replica. The actual Network UI selects the
  peer, retaining cloud configuration and keeping device-local meta unsynced.
  Browser-created schema/content reaches CLI storage, CLI-only edits return,
  **3 MiB blobs transfer in both directions** (every byte checked on the CLI,
  SHA-256 checked in the browser), and reload redials the saved upstream without
  another connect action. Initial reconciliation uses `/sync`, not a blind push
  with an unknown peer CAS head. Run with an RTC-enabled CLI and
  `cargo test --locked -p tonk-ui --features rtc-integration-tests --lib
  it_syncs_an_authorized_space_through_the_local_cli`. This is not yet the
  account-authorized offline restart, revocation-race or interrupted-blob test.

Still required, in priority order:

1. **C3:** exercise the new complete-snapshot and durable DID evidence together
   in the real account-authorized browser/CLI sync fixture, including worker and
   CLI restarts offline. Native tests now cover fresh offline session proofs and
   DID cache reopening/rotation/refusal, and now their combined signed-request
   acceptance matrix; those are not yet proof of the whole account journey.
   Verify revocation-refresh races with already-open
   content streams, in addition to the tested refusal of subsequent invocations.
   Account logout/replacement stream gating is implemented, but is not a
   substitute for that revocation test.
2. **C4:** extend the passing real account content/blob roundtrip to offline
   browser/CLI process restarts and interruption/CAS reconciliation. Authorized
   unmounted adoption, transport-aware offline scheduling and on-demand carrier
   requests have worker coverage; the real account fixture proves online
   content/blob sync and carrier recovery after page reload.
3. **C5/C6:** exercise automatic page selection/failover and reconnect during
   multi-page transfers, and implement native concurrent-stream/request limits.
   Client acquisition/stream limits now have worker coverage. The profile-wide registry context is
   implemented and tested; registry contents remain a documented startup snapshot.
4. **C7:** Firefox's loopback ICE failure, real Safari HTTPS verification, stronger
   network-isolation/actual offline-state coverage and release-feature/CI/dependency
   review. Chromium and WebKit now pass the installed-app reload journey with
   the asset origin stopped and external browser HTTP blocked; this does not
   complete the required account-authorized restart/sync matrix.
   Roll out the new lookup endpoint before expecting delegated content access:
   an older access service returning 404 is a refusal, not permission to bypass
   revocation checks. No deployment has been made by this worktree task.

Cleanup audit: no `archive_reachability` source/build artifacts, temporary disk
report, or diagnostic CLI dependencies remain. The single `Cargo.lock` addition
is `gloo-timers` in `tonk-rtc`, required by the worker carrier heartbeat.

## Historical investigation

The sections below preserve the development history. In particular, the early
chat-only status, space-based signaling, immutable per-machine certificate,
durable browser peer identity, per-effect page dispatcher and unconditionally
available fallback are not the current completion contract.

## What exists

`tonk rtc connect` (built with `--features rtc`) negotiates a WebRTC data
channel with a browser tab and relays lines of text in both directions.
Type in the terminal, it appears in the page; type in the page, it
appears in the terminal. That is the whole behaviour — the point is the
channel, not what rides on it.

Three pieces:

| Where | What |
| --- | --- |
| `rust/tonk-rtc/src/signal.rs` | The session-description envelope both halves agree on, and its URL-safe encoding. Target-agnostic, plain serde data. |
| `rust/tonk-rtc/src/peer.rs` | The native peer (webrtc-rs 0.17 — see below for why not 0.20). Offers, then talks. |
| `rust/tonk-rtc/src/loopback.rs` | Same-machine signalling: a loopback listener the browser navigates back to. |
| `rust/tonk-ui/assets/rtc.mjs` | The browser half, on the bare `/rtc` route. |
| `rust/tonk-cli/src/rtc.rs` | The `tonk rtc connect` command and the stdin/stdout relay. |

## Why WebRTC and not the loopback server the CLI already has

Because Safari will not let a page on `https://tonk.network` open a
`fetch` or a WebSocket to `http://127.0.0.1`: that is mixed content, and
now a Local Network Access prompt besides. A *navigation* to loopback is
still permitted — which is exactly why the account-login callback works
— but a navigation is one-shot and destroys the page that made it.

So for a live, bidirectional channel between a browser page and a local
process, WebRTC is not an optimisation. It is the only portable option.
NAT traversal, the usual reason to reach for it, is a bonus that matters
later.

## The ceremony

```
  tonk                                        browser
  ----                                        -------
  bind loopback listener
  create offer, gather ICE
  open <page>/rtc#offer=…&callback=…  ─────>  answer the offer
  receive answer on loopback          <─────  POST it from a hidden iframe
  set remote description
  ═════════════════ data channel ═════════════════
```

The offer travels in a URL **fragment**, which is never sent over the
network, so no SDP reaches a server log.

### Delivering the answer is the whole problem

The page must reach a listener on `http://127.0.0.1` while STAYING
ALIVE — it owns the `RTCPeerConnection`, so navigating it away destroys
the channel it just negotiated. That rules out the obvious approaches:

| | |
| --- | --- |
| `fetch` / `sendBeacon` to loopback | Mixed content in Safari; Local Network Access in Chrome. This is the reason the whole feature is WebRTC and not a local HTTP server. |
| Navigate this document | Destroys the peer connection. |
| **Hidden iframe form POST** | A nested-context navigation. Needs no user gesture and carries a body, so no fragment and no bridge page. Faces mixed content (loopback is "potentially trustworthy" per Secure Contexts, so it should be exempt) and Chrome's Local Network Access (which does cover nested-context navigations). |
| **Popup** | A top-level navigation, exempt from both, and the same mechanism `tonk account login` already relies on. Costs a user gesture, hence a button. A popup navigation is a GET, so the answer rides in the fragment and the listener's bridge page re-posts it same-origin. |

The page tries the **iframe first** and falls back to the popup. It
cannot read a cross-origin iframe, so it cannot observe that path
failing — but it does not need to: no data channel within six seconds
means the answer did not arrive.

Measured in Chromium from a genuine secure context (`isSecureContext ==
true`, self-signed cert): both `iframe.src` and a form POST targeting an
iframe reached the loopback listener, with no mixed-content or Local
Network Access complaint. **Safari is unverified** — there is no Safari
on the machine this was built on, and WebKit is historically stricter
about `http://localhost` than the spec requires. The fallback exists so
that answer does not have to be known to ship.

A popup opened after an `await` has lost the click's transient
activation and browsers block it, so the fallback path opens
`about:blank` synchronously in the handler and points it at the callback
once the answer is ready.

## Constraints discovered along the way

**`RTCPeerConnection` is `[Exposed=Window]`.** It does not exist in a
service worker. Tonk's replica and sync engine live in a service worker
(`tonk-worker`), so a WebRTC sync transport can never be a drop-in
sibling of the S3 one: the peer connection must live in the page, and
every invocation has to cross a page↔worker `MessagePort`. That hop —
not the transport — is the hard part of "add the CLI as a remote".
`tonk-worker/src/router/bridge.rs` already implements that shape for the
sealed iframe, and is the thing to generalise.

The channel's lifetime problem comes with it: the service worker
outlives any page, but a channel owned by a tab dies with the tab.

**`PeerConnectionBuilder` discards mDNS candidates by default.**
`webrtc`'s builder defaults to `MulticastDnsMode::Disabled`, which drops
*remote* mDNS candidates — and Chrome and Safari emit only mDNS host
candidates (`<uuid>.local`) by default, to avoid leaking private IPs to
a page. Against that default a browser's answer arrives with every
candidate dropped and no connection can form, on any machine. The peer
therefore passes a `SettingEngine::default()`, whose own default is
`QueryOnly`. The consequence to remember: a same-machine connection
depends on mDNS resolution working on the host. It does on an ordinary
desktop; it does not inside a container with no multicast.

**ICE is not trickled.** The signalling channel here carries one message
each way and then closes, so the peer waits for gathering to complete
and ships a single description with every candidate in it. On loopback
that wait is milliseconds. This is a property of the *channel*, not of
the peer — a channel that stays open makes `on_ice_candidate` the thing
to implement and the gather-wait the thing to delete.

**The remote surface is small.** Dialog's whole remote protocol is five
effects — `archive::{Get,Put,Import}` and `memory::{Resolve,Publish}` —
dispatched through `dialog_network::Network`, a `#[derive(Site)]` table
of `{ s3, ucan, fs }`. Adding an `rtc` field there is the shape of "CLI
as a remote". Small surface; the difficulty is all in the page↔worker
hop above.

## What does not survive contact with a second machine

Loopback signalling. It works only because the browser and the CLI share
a host, and no amount of care changes that.

The intended replacement is **signalling over the space itself**:
descriptions written as facts, read by whichever peer is listening. It
bootstraps over the existing remote, needs no new infrastructure, and
works between peers that have never shared a host. WebRTC then becomes
the direct fast path negotiated over the slow shared one.

Nothing in `tonk-cli/src/rtc.rs` reaches into how the bytes travel — it
depends on bind / hand out an offer / wait for an answer — so this is a
second implementation, not a rewrite.

### Shape

An SDP offer is not an address. It is one half of a stateful, pairwise
handshake carrying a DTLS fingerprint, ICE credentials and candidates.
If both peers can choose all three up front, each can construct the
other's SDP locally and skip the round trip entirely.

They can. A browser **does** accept a local offer whose `ice-ufrag` and
`ice-pwd` have been rewritten, and uses them on the wire — measured in
Chromium, and what `libp2p-webrtc-websys` ships. See "Dialling with no
answer at all" below; that is the design to build, and it needs neither
of the two fact shapes described next.

The offer/answer path is still needed where direct dialling cannot
reach — across NAT, where the CLI has no UDP address a browser can send
to. There, two kinds of fact:

- **Presence, cardinality one per peer** — `peer/<did> dialable
  { fingerprint, candidates, at }`. Superseding is exactly right: a new
  tab or a restarted CLI replaces the stale record. Note what is *not*
  here: ICE credentials. This says "I exist and here is how to reach my
  host", not "here is a connection".
- **Negotiation, per pair** — `session/<dialer>/<dialee>` with `offer`
  and `answer`, each cardinality one, written by the respective side.
  Superseding the offer *is* reconnect-or-ICE-restart, so "the current
  attempt wins" comes for free.

### Open questions

- **Presence is a lie by default.** A browser cannot be dialed
  passively; a tab has to be open to answer. A cardinality-one
  `dialable` fact from a tab that closed thirty seconds ago still reads
  as current. Needs a heartbeat or a TTL on `at`, and dialers must
  tolerate dialing a corpse and retrying when the record is superseded.
  This is the hard part, not the SDP plumbing.
- **Bootstrap latency.** Each connection costs a sync round trip through
  the S3 remote. Fine for establishing a long-lived channel; not fine
  for fast reconnects.
- **Browser candidates are mDNS-obfuscated.** A browser's `dialable`
  record carries `<uuid>.local`, which resolves only on the same LAN.
  Anything wider needs STUN in the record so there are `srflx`
  candidates too.
- **Who may offer.** Today a fragment-delivered offer is an
  unauthenticated request to open a channel into someone's browser.
  Harmless for a chat echo; not harmless once the channel speaks dialog
  effects. Signalling through the space fixes this incidentally — only
  peers who can already write the space can offer — which is a reason to
  get there before the payload grows.

Signalling through the space also removes the delivery problem above
entirely: no loopback hop means no iframe, no popup, no gesture and no
button.

## Dialling with no answer at all

Yes, and it is the design to build. Measured, not reasoned about — see
the `direct_dial` spike (removed once this crate moved off 0.20, since
it measured that version specifically; recovered from git history if the
raw instrument is ever wanted again).

The idea is libp2p's WebRTC-Direct: browsers refuse to let you munge
your own local SDP, but `setRemoteDescription` accepts any well-formed
SDP. So a browser can synthesise the CLI's half of the handshake from a
published address record, build its own offer locally, and dial — with
nothing travelling back. For that the CLI must stop needing the three
things an answer would carry.

What the measurements found, on `webrtc` 0.20.5:

| The CLI would need | Verdict |
| --- | --- |
| The peer's DTLS fingerprint | **Not needed.** `disable_certificate_fingerprint_verification(true)` works. The browser still checks the CLI against the published certhash, so the connection is one-way authenticated. |
| The peer's ICE candidates | **Not needed.** A peer-reflexive candidate is formed from the source address of the first check. |
| The peer's `ice-ufrag` | **Needed.** Username validation checks both halves; a placeholder is rejected outright. |
| The peer's `ice-pwd` | **Needed — because `set_lite(true)` does not suppress outgoing checks.** Despite lite mode the agent pings, signing with the remote password; a wrong one draws a `401` and the pair never validates. This contradicts the setting's own docs and is worth reporting upstream. |

Two supporting facts. Chromium dials a `127.0.0.1` remote candidate
without complaint, from `http` and `https` origins alike — that doubt
turned out to be nothing. And there is no UDP mux
(`SettingEngine::set_udp_network` is commented out as `/*todo:*/`), so
libp2p's trick of reading the peer's ufrag off the first STUN packet
before building a peer connection is not available without upstream
work.

Those last two rows say the CLI needs the peer's ICE credentials. It
does — but it does not need to *receive* them, because **the dialer
chooses them for both sides**.

`libp2p-webrtc-websys` munges its own `createOffer` output, replacing
`a=ice-ufrag` and `a=ice-pwd` with one random string, and synthesises
the peer's SDP with that same string in both fields. Measured in
Chromium: `setLocalDescription` accepts the rewrite, `localDescription`
reflects it, and the STUN `USERNAME` on the wire reads
`<ufrag>:<ufrag>`. One shared secret, symmetric, nothing to exchange.

The listener then reads that ufrag out of the first STUN packet and
builds a peer connection with `set_ice_credentials(ufrag, ufrag)`. That
needs a UDP mux — which the **0.17 line of the same `webrtc` crate
has** (`SettingEngine::set_udp_network(UDPNetwork::Muxed(..))`, a live
function there, and what `libp2p-webrtc` is built on). Its absence in
0.20.5 is a regression in the rewrite, not a property of the ecosystem.

So the address record collapses to what the CLI alone knows:

```
peer/<did>  dialable  { host, port, certhash }   ← CLI, on start
```

Zero round trips. No callback, no iframe, no popup, no button, and
nothing for the dialer to publish.

### Built, and verified

`tonk rtc listen` publishes an address; the browser dials it. Measured
end to end against real Chromium: the channel opens and relays both ways
with **nothing travelling back to the CLI** — no callback, no iframe, no
popup, no button, and no signalling channel of any kind.

```
[cli]  listening on 127.0.0.1:46660, 192.0.2.2:41445
[dial] address published; the CLI now receives nothing until the channel is live
[cli]  connected.
[dial] PASS  channel open with zero round trips
```

One setting turned out to be load-bearing and is easy to miss:
`set_include_loopback_candidate(true)`. Without it webrtc-rs publishes
no `127.0.0.1` candidate at all, and the same-machine case — the one
this exists for first — has nothing to dial.

### One address, many dials

The first version served exactly one connection ever. Measured, because
it was worth checking rather than assuming:

```
tab 1: CONNECTED
tab 2 (concurrent):        FAILED — stuck at "dialing…"
tab 3 (after both closed): FAILED — stuck at "dialing…"
```

ICE separates peers by ufrag, so a fixed credential in the address
means one peer, and the single peer connection was consumed by the
first dialer and never released — not even a sequential redial worked.

The fix is a UDP mux, and it *simplifies* the record: the dialer picks
its own ufrag, so the credential leaves the address entirely.

Rather than reimplement `UDPMuxDefault` to add ufrag discovery
(libp2p's own is some six hundred lines), `src/mux.rs` wraps the socket
the mux reads from. It is an ordinary `Conn` that passes every packet
through untouched and, on the way past, notices binding requests
carrying a ufrag it has not reported. The listener builds a peer
connection for that ufrag; ICE retransmission means the next packet
lands on a route that now exists. A few dropped packets at the start of
a dial cost nothing — ICE is built to expect loss. About a hundred
lines instead of six hundred.

```
tab 1: CONNECTED
tab 2 (concurrent):        CONNECTED
tab 3 (after both closed): CONNECTED
```

Two consequences.

**The certificate must be persisted, and fixed across dials.** Each
per-dial peer connection would otherwise mint its own, so every dialer
after the first would check the published fingerprint against a
different certificate and refuse. `src/identity.rs` holds it; the CLI
stores the PEM beside its other local state, `0600`.

**A reachable port is dialable.** Nothing at this layer gates who may
connect, which is deliberate: authorization is per invocation, where
every request carries a signed UCAN and is verified before any work is
done. Reachability is not permission. What an open port *does* cost is
resources — anyone can make this side run a DTLS handshake — so
concurrent dials are capped and closed connections are shed first.

Still outstanding: the **application-layer handshake** that the one-way
DTLS authentication requires, and **framing/chunking** before dialog
effects ride the channel (data channel messages cap at 64 KiB).

### What it costs

- ~~Move to `webrtc` 0.17 and port `peer.rs` to its callback API.~~
  **Done.** The port compiled first try and the browser end-to-end
  passes unchanged on it. 0.17 also re-exports `webrtc::ice`, so the ICE
  types are nameable without a version-locked second dependency, and its
  `MulticastDnsMode` default is already `QueryOnly` — set explicitly
  anyway, since a default is exactly what a version bump changes
  underneath you.
- A UDP mux keyed on the STUN `USERNAME` (libp2p's is ~350 lines and
  `pub(crate)`, so it is a reimplementation, not a dependency).
- A persisted certificate, so the published certhash survives a restart.
- An application-layer handshake. DTLS is one-way authenticated here —
  the dialer verifies the CLI against the published certhash, the CLI
  verifies nothing — so possession of the address becomes the
  capability unless something on top checks the peer. libp2p uses Noise;
  tonk has DIDs and UCAN delegations already.

### WebKit: measured, and it works

This was the outstanding risk — the munge had only been measured in
Chromium, and libp2p documents browser WebRTC-Direct for Chrome and
Firefox. WebKit refusing a rewritten local `ice-ufrag` would have killed
this path for the browser the whole project exists to serve.

Playwright's WebKit build (`AppleWebKit/605.1.15 Version/26.0`, Safari's
engine — not Safari itself, but the same engine) says otherwise:

```
[munge] setLocalDescription accepted: true
[munge] localDescription now says ice-ufrag: tonkdial+v1/…
[munge] STUN USERNAME on the wire:  tonkdial+v1/…:tonkdial+v1/…
```

Byte-identical to Chromium. And the full ceremony end to end, not just
STUN inspection:

```
[cli]  listening on 127.0.0.1:51247, 192.0.2.2:51247
[dial] PASS  channel open with zero round trips
[dial] PASS  browser -> CLI
[dial] PASS  CLI -> browser
```

Caveat worth keeping: Playwright's WebKit is not Safari. It shares the
engine but not the embedding, and Safari adds its own policy on top —
Local Network Access prompts among it. This clears the *engine*
question, which was the one that could have invalidated the design. A
run on real Safari is still worth ten minutes before anyone relies on
it.

## What iroh costs in a wasm bundle

Measured against a deliberately non-empty baseline, `opt-level = "z"`,
LTO, stripped, referencing `Endpoint::builder(..).bind()` so the linker
retains the QUIC machinery rather than discarding it:

| | raw | gzipped |
| --- | --- | --- |
| baseline | 27.9 KB | 9.3 KB |
| + iroh, `presets::Empty` | 768 KB | 223 KB |
| + iroh, `presets::N0` | 1,367 KB | 446 KB |

So iroh costs about **+214 KB gzipped** at minimum, and **+427 KB
gzipped** with the n0 relay and discovery — roughly half the weight is
the relay and discovery half, which an offline-first design may not
need on the browser side.

Read these as a floor rather than a figure. The probe references the
builder without awaiting it, so some connection and stream paths are
still absent; it carries no `wasm-bindgen` glue, no custom transport of
ours, and no `wasm-opt` pass (which would claw back some). Against a
bundle that was deliberately slimmed in #931, +214 KB gzipped is a real
number to weigh rather than an obvious yes or no.

### The risk that remains

It also does not survive NAT — the CLI must be reachable by UDP at the
published address, and the dialer cannot help it hole-punch without a
round trip. Same machine and same LAN, yes; anything wider needs the
offer/answer path as a fallback. One address record, two dial methods.


## Why the DB carries neither descriptions nor addresses

An earlier draft argued the space should carry long-lived *addresses*
while never carrying *descriptions*. Half of that reasoning survives.
The conclusion does not: the space carries neither, and is not part of
this design at all.

**What rules out descriptions is timing.** ICE gives up on failed checks
after roughly thirty seconds, so a handshake that waits on sync bets on
a number it does not control. And the CLI's sync is worse than "not real
time": `tonk-cli/src/auto_sync.rs` pulls before and pushes after a
*mutating* `tonk eval` — command-triggered, with no background poller.
An idle CLI never learns anything from the remote, so a per-dial fact
would sit in S3 until someone happened to run a command. That latency is
unbounded, not merely long.

**What rules out addresses is circularity.** To read a peer's address
out of the space you must first have synced the space — and when that
peer *is* the sync remote, syncing required connecting to it. The loop
only opens if some third always-reachable remote mediates, at which
point you are online anyway and a discovery service does the job better.

**And the case it was meant to cover is empty.** If the DB cannot reach
a mediating remote there is no internet; with no internet a remote peer
is unreachable whatever the DB says. The DB could never have helped,
because syncing it needs the same network the dial needs.

So the space is not a rendezvous. Addressing is iroh's job, and the
sections below are how — including offline, where the answer turns out
not to need a rendezvous either.

## Reaching a peer on another network

Direct dial needs the CLI reachable by UDP at a published address. How
far that goes without any real-time coordination:

1. **Same machine or LAN** — host candidates. No coordination. This is
   what the current work covers.
2. **An explicit port mapping (UPnP / NAT-PMP / PCP), or a genuinely
   public host** — the CLI creates a real inbound rule and is reachable
   like a server. Direct dial across networks with zero coordination.
   The strongest unassisted option, and why `iroh` carries a
   `portmapper` dependency.
3. **Everything else** — where it stops, for a structural reason rather
   than a latency one.

Publishing a STUN-derived reflexive address is not the general fix it
looks like:

- **Full-cone NAT** — works unassisted, given keepalives to hold the
  mapping. Uncommon.
- **Restricted-cone / port-restricted** — the inbound packet is dropped
  unless the CLI has already sent outbound *to that particular dialer*,
  so the CLI must learn the dialer's address first.
- **Symmetric NAT** — requires both sides to transmit simultaneously.
  That is inherently real time; a store-and-forward channel cannot do it
  at any latency.

So hole punching needs a real-time channel or a relay, and no amount of
tuning makes the space into one. That is exactly why `iroh` has relays
rather than being clever about it.

An earlier draft proposed building that rung in the worker / access
service: a small signalling endpoint, used only when a dial fails. That
is no longer the plan. Taking iroh means taking its relays and its
address lookup, which is that rung already built, maintained, and with
NAT traversal we would otherwise be writing ourselves. The cost is a
dependency on n0's infrastructure for the online case; the offline case
does not touch it.

## Addressing a remote by its key

The published address is `{ candidates, fingerprint }`, and keeping it
stable took work: persist the certificate so the fingerprint does not
move, pin the port so it does not either. Both are now done, and a peer
that reads the record once can cache it. But the record is still a
*location*, and locations churn — a new network, a different machine, a
port already taken.

The alternative is to address a remote by its **identity** and let
something else find it. That is iroh's model: a node is an ed25519
public key, and discovery resolves it to wherever it currently is.

This fits tonk unusually well, because the key already exists. The root
identity is an `Ed25519Signer` and dialog's credentials are ed25519 —
the same curve iroh uses for a `SecretKey`. A DID and an iroh
`EndpointId` would be two encodings of one public key, so `tonk remote
add <did>` could name a remote that is dialable forever, with no record
to publish and nothing to keep in step. That also matches how remotes
already work, rather than inventing a parallel notion.

Worth separating two things that arrive together:

- **The addressing model** — a remote is a DID, configured once. This
  stands on its own and can be adopted without iroh.
- **The transport** — given a DID, how is it reached. Direct WebRTC
  (built) when there is a local address record or the peers share a
  LAN; something with discovery and NAT traversal when they do not.

That split is now the design. A remote is an `EndpointId`, stored once;
routes are resolved rather than recorded. And the remote case needs no
bootstrap channel to carry SDP in band, because iroh's own address
lookup carries the WebRTC route directly — see below.

## Offline is the primary case, so the tiers are

Browser-to-CLI must work with no internet — same machine, no relay
reachable — which settles the earlier question. The direct dial stays,
and anything with discovery sits above it rather than replacing it.

| | How the route is found | What carries the data |
| --- | --- | --- |
| Browser ↔ CLI, same machine, offline | derived: fixed port on loopback | direct WebRTC — built |
| Browser ↔ CLI, same LAN, offline | configured by the operator | direct WebRTC |
| Browser ↔ CLI, remote | pkarr over HTTPS | WebRTC data channel |
| CLI ↔ CLI, same LAN, offline | mDNS | plain iroh, no WebRTC |
| CLI ↔ CLI, remote | pkarr, DNS | plain iroh, no WebRTC |

The last two rows are where iroh is unambiguously right: both peers are
native, both can open UDP sockets, real hole punching, no relay in the
data path. WebRTC is in this design only because browsers cannot do
that.

Row one is the primary case, and what it needs is **no per-dial
signalling** — not "no address at all", which an earlier version of this
paragraph claimed and which is wrong in a way worth spelling out,
because it cost a day.

Of the three things a dialer needs, two are derivable and one is not:

| | Derivable by the browser? |
| --- | --- |
| The port | Yes — it is fixed. |
| The candidates | Yes — loopback, for the same-machine case. |
| The CLI's DTLS fingerprint | **No.** |

The fingerprint is not derivable and cannot be skipped. `webrtc-rs` has
`disable_certificate_fingerprint_verification`, which is what lets the
*CLI* accept a browser certificate minted per page load; a browser has
no such knob. It verifies the remote certificate against the
`a=fingerprint` line in whatever SDP it was handed, and the SDP here is
one it fabricated, so a placeholder fails the DTLS handshake outright.
`decodeAddress` in `rtc.mjs` refuses an address without one, and
`the_published_address_carries_everything_a_dialer_needs` pins that the
listener publishes it.

"The fingerprint stopped being a security boundary" is true and is a
different claim: iroh's TLS authenticates by endpoint key, so reaching
the port grants nothing and an unrelated process there cannot complete
a connection. The fingerprint is still load-bearing for the *handshake*
even once it is no longer load-bearing for *authentication*.

So the address is published once and cached forever — the certificate
is persisted precisely so it survives a restart — and nothing travels
per dial. That is the property the design actually has, and it is
enough: discovery tolerates unbounded latency, the handshake involves
no sync, and a `did:key?route=custom:…` carries the whole record.

Row two is configuration, not discovery — the operator supplies an
address, exactly as they would for a non-default port. Browsers cannot
speak mDNS, so there is nothing to automate here, and pretending
otherwise would mean building the rendezvous this design just removed.

## How a route is found

iroh resolves an `EndpointId` through *address lookups*, several at
once, and the interesting part is that they do not all carry the same
thing. Measured against iroh 1.2 and `iroh-mdns-address-lookup` 0.5:

| lookup | works in a browser | carries `TransportAddr::Custom` |
| --- | --- | --- |
| pkarr over HTTPS | yes | yes |
| DNS | no — native only | yes |
| mDNS (separate crate) | no — native only | **no** |
| `MemoryLookup` | yes | yes — the application supplies it |

Two consequences.

**The WebRTC route is publishable.** `iroh-dns` encodes custom
addresses into the pkarr record —
`TransportAddr::Custom(addr) => attrs.push((IrohAttr::Addr, addr.to_string()))`
— with round-trip tests for Bluetooth and Tor transports. So the CLI
publishes its WebRTC route the same way it publishes a relay URL, the
browser resolves it over HTTPS, and the remote case needs nothing
bespoke. Publishing is world-readable, though: a signed pkarr record
keyed by endpoint id discloses the machine's addresses to anyone who
knows the id. That is a different question from whether a dial is
*authorized* — every invocation is UCAN-verified regardless.
`iroh-dns` exposes an `AddrFilter` as the knob; its shape is unread.

**mDNS does not help the browser, twice over.** It is native-only, and
its publisher writes only a relay-URL attribute, the IP list and user
data — there is no custom-address attribute, so it would not advertise a
WebRTC route even if a browser could listen. It belongs to the CLI ↔ CLI
row and nowhere else.

**The local route is registered, not special-cased.** `MemoryLookup`
"allows application to add and remove out-of-band addressing
information" and is not gated out of wasm. So the derived loopback route
and any operator-configured address go in there, and iroh resolves them
beside pkarr like any other route. Preferring them is
`Builder::path_selector`, whose own documentation names this case:

> Pass a custom `PathSelector` here to override that policy — for
> example, **to make a custom transport always win over IP**.

So "prefer the local dial" is two supported APIs rather than a branch,
and `endpoint.connect(id, ALPN)` still has no code path in it.

## The custom transport is the design

An earlier draft of this note argued that iroh need only be a
signalling channel, because `tonk-rtc::Session` was already "the channel
abstraction". That was wrong, and worth recording as wrong.

`Session` is 64 KiB messages with no framing. iroh's abstraction is
QUIC: multiplexed concurrent streams, flow control and backpressure,
cancelling one stream without killing the connection, timeouts and
keepalives — and **streams rather than messages, which removes the
chunking problem entirely** rather than leaving it as work. Building
that well is a great deal of subtle effort, and a session's worth of
proof of concept is not a substitute for it.

So the shape is the one `iroh-webrtc-transport` uses: keep iroh's
`Connection` and stream API, and change what carries the bytes. Locally
a WebRTC data channel, remotely iroh's own QUIC over relay or a punched
hole. One abstraction, several transports, all addressed by the same
key.

**A browser can do this.** In `iroh` 1.2's `socket/transports.rs` the IP
transports are `#[cfg(not(wasm_browser))]` while `mod custom` is
ungated, as is `custom: Vec<Box<dyn CustomEndpoint>>` on `Transports`.
So a browser endpoint has no UDP but *can* carry a custom transport —
which is exactly the offline case, with no relay in it.

The extension point is three traits — `CustomTransport` (a factory),
`CustomEndpoint` (`bind`, `poll_recv`, local addresses) and
`CustomSender` (`poll_send`) — behind
`Builder::add_custom_transport`. The interface is datagram-shaped:
fill `bufs`, `metas` and `recv_infos`, essentially "be a UDP socket".

Two details that are easy to get wrong and expensive to debug:

- **The data channel must be unreliable and unordered** —
  `{ ordered: false, maxRetransmits: 0 }`. QUIC supplies its own
  reliability and ordering; carrying it over a reliable ordered channel
  produces head-of-line blocking and retransmission fighting
  retransmission. The channel this crate opens today is the default
  reliable, ordered one, which would work in a quiet test and come
  apart under loss.

- **`CustomTransport` is `Send + Sync + 'static`, an `RTCDataChannel`
  in wasm is neither.** `SendWrapper` is the established answer here —
  `tonk-worker/src/router/bridge.rs` already does it for `MessagePort`,
  on the same single-threaded reasoning.

The risk to carry: `unstable-custom-transports` is unstable by name, so
pin the iroh version exactly and expect to follow it.

### The local dial is a route, not a code path

The tiering earlier in this note — local here, remote there — reads as
though an application has to choose. It does not. In `iroh-base`:

```rust
pub struct EndpointAddr {
    pub id: EndpointId,
    pub addrs: BTreeSet<TransportAddr>,   // one peer, many paths at once
}

pub enum TransportAddr { Relay(RelayUrl), Ip(SocketAddr), Custom(CustomAddr) }

pub struct CustomAddr { id: u64, data: CustomAddrBytes }
```

`Custom` is a peer of `Relay` and `Ip`, and `addrs` is a set, so one
endpoint address carries a relay URL, an IP and a WebRTC route at the
same time. iroh's path selection (there is a `biased_rtt_path_selector`)
picks among them and migrates, so a local path at sub-millisecond RTT
wins over a relay without anyone deciding, and fails over when it dies.

Application code is `endpoint.connect(id, ALPN)`. There is no branch.
Offline, the custom path is simply the only viable one.

`CustomAddr.data` is **opaque bytes this transport defines**, which
means the `Address { candidates, fingerprint }` already built is the
address payload — not a parallel scheme to be reconciled with iroh's,
but the content of `CustomAddr` for transport id *n*. `CustomAddr`
implements `Display` and `FromStr` as `{id:x}_{hex}`, so it goes into
config, a fact, or a URL as it stands.

So `tonk remote add <did>` records an `EndpointAddr` holding the custom
route beside whatever relay and IP information exists, and it keeps
working with no network because the custom route never needed one.

### What this retires

**The fingerprint stops being a security boundary.** iroh's TLS
authenticates end to end by endpoint key, so the WebRTC layer beneath is
a pipe. The fingerprint stays in `CustomAddr` for addressing — and
persisting the certificate still keeps that stable — but it carries no
trust. The one-way DTLS authentication caveat threaded through this
whole note dissolves, and with it the argument that the application
handshake is the *only* thing standing between a reachable port and a
served request: iroh will not complete a connection with a peer that
cannot prove the key.

**The 64 KiB message cap stops existing.** iroh gives streams.

### Where the work already done fits

None of it is wasted, but its role changes. The direct dial — address
record, fixed port, persisted identity, the mux — stops being the
transport and becomes the **local dial**: how two peers on one machine
find each other and open a data channel with no network at all. That
channel is then handed to the custom transport, and everything above it
is iroh.

`dispatch.rs` was claimed here to be unaffected, on the grounds that the
worker cannot hold a connection. That reasoning was wrong, and what
replaces it is below under *Which side of the worker boundary iroh sits
on*.

### Measured: iroh runs over a browser data channel

The native end-to-end test proved the transport; it did not prove the
browser, and the wasm port only proved that the browser half compiles.
Those are different claims, so the browser one was measured separately.

Two `RTCPeerConnection`s in a single page, wired to each other, with one
`ordered: false, maxRetransmits: 0` data channel between them. Both ends
of that channel go into a wasm module that builds two
`Endpoint::builder(presets::Empty)` endpoints over them, connects by
`TransportAddr::Custom`, opens a bi-stream and echoes. `presets::Empty`
matters: no relay, no discovery, and in a browser no IP transport to
fall back on either. If the exchange completes, it completed over the
data channel, because there was nothing else.

It completes. **Chromium, WebKit and Firefox all pass**, first run, no
per-engine accommodation — three independent SCTP implementations under
the same code.

`presets::Empty` is the *test's* preset, not the product's: a shipped
browser endpoint uses `presets::N0` so pkarr can resolve routes. The
test must keep `Empty`, though — under `N0` a green run would no longer
prove the data channel carried anything, because a relay could have.

Browser-to-browser rather than browser-to-CLI on purpose: it isolates
transport from signalling. With this, the remaining work between a
browser and the CLI is finding the peer, not carrying the bytes.

Two things this does *not* claim. It is Playwright's engine builds, not
shipped Safari or shipped Chrome. And both peers were in one page, so
the data channel never crossed a process — the native test is what
covers a channel that does, and the direct-dial spike is what covers a
browser channel reaching a CLI.

The harness is `scratchpad/e2e/wasmproof.mjs` plus a small cdylib; like
the rest of the browser harnesses it is not checked in, for the same
reason — CI has no browser runner for this crate.

## Evaluated: `iroh-webrtc-transport`

The crate bootstraps a WebRTC session over an *iroh* stream (ALPN
`noop/iroh/webrtc/bootstrap/1`), exchanges SDP over it, then promotes
the resulting `RTCDataChannel` into an iroh custom transport so callers
see ordinary iroh connections. `BootstrapTransportIntent` is
`IrohRelay | WebRtcPreferred | WebRtcOnly`, and TURN is deliberately
refused — the fallback is iroh's own relay. Architecturally it is the
same bootstrap-then-upgrade shape described above, with iroh's relay
where this design has the space.

Read against the addressing model above, this is not an alternative to
the direct dial but the **remote-case bootstrap for it**: connect over
iroh, exchange SDP on that stream, upgrade to a direct data channel.
Nothing needs publishing, because the description travels in band.

What does not change is that **a browser's iroh connection is always
relayed** — browsers cannot open UDP sockets — so two processes on one
laptop could not connect without reaching the internet. The obvious
escape fails too: a local iroh relay would have the browser open a
WebSocket to `http://127.0.0.1`, the Safari-blocked case this whole
feature exists to avoid.

So it does not replace the direct path; it sits above it. **The question
that decides how much of iroh to take is whether browser-to-CLI must
work with no internet at all.** If it must, the direct dial stays and
iroh is the remote tier. If it need not, iroh alone is simpler and the
direct dial is an optimisation.

The "second identity system" objection does not survive contact with
the profile keypair: both are ed25519, so one key can serve as both DID
and `EndpointId`. What remains: pinned to
`iroh ^0.98.2` while iroh is at 1.2.0; `0.1.0-alpha.2` from an
individual's repository rather than n0's, whose README still describes
itself as `publish = false`; shipped tests cover only the native path;
`browser-main-thread` only, so the page/worker bridge is still needed;
and iroh + noq + webrtc added to a wasm bundle that was recently
slimmed.

Worth keeping for two things. Its `src/native.rs` (webrtc 0.17) and
`src/browser/rtc.rs` (web-sys) are a reference implementation of exactly
the cross-target session layer the 0.17 port needs. And if CLI-to-CLI
mesh with discovery ever becomes a product feature rather than a
workaround, this is how a browser joins that mesh.

## Where this lives: `dialog-iroh-remote`

dialog-db gets a new site, and the point of the shape below is that it
holds no opinion about browsers, workers or tabs.

**The address is `EndpointAddr`**, which already derives `Serialize,
Deserialize, Clone, PartialEq, Eq, Hash, Ord` — so it satisfies
`SiteAddress` as it stands. A ticket from the CLI is the transfer
format; what gets *stored* is smaller, because iroh's own guidance is:

> Tickets can go stale: the dialing information in a ticket (especially
> IP addresses) can become outdated as network conditions change. For
> long-lived connections, prefer caching `EndpointID`s and letting iroh
> resolve current dialing details.

So `SiteId` — the credential-store key — is the `EndpointId` alone, not
the whole address. Routes churn; identity does not, and keying on the
whole thing would orphan a credential every time a route moved.

**The effect surface is closed**: `archive::{Get, Put, Import}`,
`blob::{Read, Import}`, `memory::{Resolve, Publish, Retract}` — seven,
the same set `S3` and `Fs` implement. If they reduce to a request shape
the way `UcanSite` does, one blanket impl covers all seven instead of
seven impls; `UcanSite` is the precedent.

**It needs both halves.** A client site that turns effects into iroh
streams, and the responder that accepts a connection and performs them
against a local repository — the CLI is the server here. One wire
protocol, so one crate.

**Cross-target is nearly free.** `iroh::Endpoint` is already
cross-target and the transport core is already target-agnostic, so this
crate needs almost no `cfg`.

Two constraints worth knowing before writing it:

- **`add_custom_transport` is a builder method.** There is no post-bind
  registration anywhere in the endpoint API, so the transport set is
  fixed at `bind()` — and in a browser the transport is tonk's. The
  crate therefore does not build its own endpoint; it takes one,
  registered once at process init the way `http_client()` is. That is
  one-time platform setup, not per-remote configuration, so it never
  reaches the address or the site.
- **`#[derive(Site)]` rejects `#[cfg]`-gated fields** outright
  (`rust/dialog-macros/src/site.rs`), because the generated impls need
  per-variant bounds in a `where` clause and attributes there are still
  unstable (rust#115590). Feature-gating the variant means declaring
  `Network` twice under `#[cfg]`. Cheap in lines, but it splits
  `NetworkAddress`'s variant set per build, and addresses are persisted
  — so an address written by one build will not load in the other.

A `Dynamic` variant wrapping `dyn Site` was considered and is not
expressible: `Site` has a generic associated type (`type Fork<Fx:
Effect>`), which is not object-safe, and `Address: DeserializeOwned`
cannot be produced through a trait object without a registry.

## Which side of the worker boundary iroh sits on

`RTCPeerConnection` is `[Exposed=Window]`, so a *peer connection* cannot
live in the worker. It does not follow that the *iroh endpoint* cannot,
and that distinction decides how much of the section after this one is
needed at all.

**(a) iroh in the page.** The worker hands a tab an effect invocation;
the tab runs the endpoint and the peer connection and hands back a
result. The boundary carries dialog effects, so it needs request/response
bracketing, ack deadlines, failover and session pinning — everything
`dispatch.rs` does — and dialog-db would have to know the ceremony
exists.

**(b) iroh in the worker, WebRTC in the page.** The boundary carries
datagrams. The page owns an `RTCPeerConnection` and relays bytes; it
links neither iroh nor dialog-db.

**(b) is the intent**, and it is the shape the transport was already
built for: `WebRtcTransport` holds no data channel, it hands out a
`Port { outbound, inbound }` and platform glue owns the carrier.
`native.rs` and `web.rs` are two pieces of that glue; a `MessagePort`
relay is a third, and the page-side shim that moves `ArrayBuffer`s
between a port and a data channel is the fourth. None of it reaches
dialog-db.

Two things follow that (a) does not give. `index.html` builds `ui` and
`worker` as separate wasm binaries, so under (b) iroh's ~214 KB gzipped
lands only in the worker bundle, not the page one that was recently
slimmed. And QUIC absorbs most of what `dispatch.rs` hand-rolls: session
pinning exists because a half-finished push through a dying tab left a
remote in an unsafe state, but an interrupted QUIC stream is only an
interrupted stream, and the two deadlines are loss detection. What
survives is "pick a live tab, pick another when it dies".

### Measured: the port is not the bottleneck

The objection to (b) was the datagram rate: every QUIC packet is one
`postMessage`, plausibly thousands per second, where (a) sends one
message per effect. That was the one thing that could have overturned
this, so it was measured rather than argued.

Two dedicated workers, each holding an iroh endpoint and a transport
with no carrier of its own. The page holds both peer connections and
relays `ArrayBuffer`s between each worker's `MessagePort` and its data
channel, transferred rather than cloned. Against a baseline of the same
exchange with both endpoints in the page and no port in the path. 8 MiB
over one QUIC stream, same browser, same run:

| engine | relayed | direct | datagrams/s relayed |
| --- | --- | --- | --- |
| Chromium | 12.4–13.1 MB/s | 10.0–11.1 MB/s | ~14,000 |
| WebKit | 5.9 MB/s | 4.8 MB/s | ~6,600 |
| Firefox | 1.6 MB/s | 0.9 MB/s | ~1,800 |

**The relay is faster, on every engine, by 15–72%.** Not "acceptable
overhead" — a win, and consistent across three runs on Chromium and one
each on the others. The reading: moving QUIC off the main thread buys
more than the port costs. In the baseline both endpoints share one event
loop with the data channel; relayed, each endpoint gets its own thread.

Stated honestly, that comparison is not an isolated postMessage cost —
it is (a)-shaped single-threading against (b)-shaped multi-threading,
which is the confound *and* the reason it is the decision-relevant
number. What it rules out is the failure mode that mattered: the port
does not throttle the data path.

Firefox's absolute throughput is a separate worry. 1.6 MB/s relayed is
low for bulk sync, and it is low in the baseline too, so it is not the
relay. Whether that is wasm, SCTP, or the unreliable-unordered channel
is unexamined.

The section that follows is written for (a). It is kept because the
policy in it is real and tested, and because a page still has to be
chosen even when all it carries is bytes.

### The decision it was written for

The replica and the sync engine are in the worker, the channels are in
the pages, and every operation crosses worker → page → CLI and back. The
worker has to pick a page.

`rust/tonk-rtc/src/dispatch.rs` is that decision and nothing else — no
worker, no `postMessage`, no WebRTC — so every failure path is reachable
in a test without a browser. It compiles for wasm32, which is what lets
the worker use it.

### The answer does not arrive everywhere

Each page holds its own `RTCPeerConnection`: separate DTLS session,
separate SCTP association, separate data channel. Point-to-point, no
broadcast. So the CLI's answer comes back on the channel that carried
the request, to the page that sent it — pairing is automatic, and the
failure to handle is narrow: a page that dies after sending loses the
answer with its channel, and the request must be re-issued elsewhere.

That is safe because the remote effects are idempotent: `archive::Put`
of a content-addressed block is a no-op on repeat and `memory::Publish`
is a compare-and-swap. Worth stating because it is load-bearing — add a
non-idempotent effect and retrying silently stops being safe.

### Ranked by visibility, because frozen tabs do not announce it

Browsers freeze and throttle background tabs. A frozen tab does not say
so; it stops answering. The visible tab is the one the browser
guarantees is running, so "most recently visible" is a liveness
heuristic rather than an arbitrary tiebreak.

### Two deadlines, because they detect different things

An unacknowledged request means the page is not running, and should be
abandoned in about a second. An acknowledged request with no answer
means the page is alive and the CLI is working, which may legitimately
take much longer. A single deadline would have to be either so short it
sheds healthy slow work or so long a frozen tab stalls sync behind it.

The split pays for itself a second time. When a page fails to
acknowledge it is **demoted**, not merely skipped for that request —
otherwise it stays the most-recently-visible candidate and every queued
operation pays the ack deadline again before reaching the same
conclusion. Ten queued operations, ten wasted deadlines, serially. A
page that *did* acknowledge keeps its standing, because the timeout
says nothing about the page.

### A whole session is pinned to one page

Dialog's push writes blocks in reference order, children before parents,
so that every prefix of an interrupted push leaves the remote
closure-complete. `dialog-repository`'s own note is emphatic that this
is a protocol invariant rather than a nicety: another pusher's existence
probes prune a whole subtree on one positive answer, which is sound only
if a block's presence implies the presence of everything it references.

Spreading one push across pages breaks it — two pages write over
separate SCTP associations with no ordering between them — and the
damage is not self-contained, because it makes a *different* pusher's
probes unsound. So a session is pinned to a page for its whole life,
even when a better candidate appears mid-session.

The same invariant is what makes failover safe: an interrupted push left
the remote closure-complete, so the session simply restarts elsewhere
with nothing to undo.

### Giving up is a normal outcome

When every page is frozen — the person switched to another application
— there is no live carrier and waiting is not the answer. The dispatcher
reports `Abandon` and the caller falls back to the ordinary remote.
WebRTC here is an accelerator, never the only path.

Note this is *not* the same as "no pages at all": with no clients there
is no service worker either (absent Web Push or periodic background
sync, which tonk has neither), so nothing is running to need a fallback.
If either is ever added, a worker can wake with no clients and
`Abandon { NoPage }` becomes a live path rather than a guard.

## Running it

Against a dev server:

```sh
nix develop            # then, in one shell:
dev:web                # trunk on http://127.0.0.1:8080

cargo run -p tonk-cli --features rtc -- rtc connect \
  --via http://127.0.0.1:8080/rtc
```

Against production, `--via` defaults to `https://tonk.network/rtc`, so
`tonk rtc connect` is enough. `--no-open` prints the URL instead of
launching a browser.

## Tests

- `cargo test -p tonk-rtc` — envelope round-trips and rejections, the
  offer actually describing a data channel, host candidates gathering
  without STUN, and the loopback listener's full POST round trip.
- `cargo test -p tonk-cli --features rtc` — `--via` validation (scheme,
  credentials, a caller-supplied fragment that must not collide with
  ours).
- `node --test rust/tonk-ui/tests/rtc.test.mjs` — `/rtc` registering no
  service worker, envelope round-trips in the browser's encoding, and
  the loopback-only check on the callback URL.

The full ceremony was additionally driven end to end against headless
Chromium during development (real `RTCPeerConnection` against real
webrtc-rs, popup and all). That harness is not checked in: CI has no
browser runner for this crate, and standing one up is its own piece of
work.

## Cost

The `rtc` feature is off by default. `webrtc` pulls an ICE/DTLS/SCTP
stack that roughly doubles `tonk-cli`'s dependency graph, and a shipped
`tonk` has no use for it until the channel carries sync.
