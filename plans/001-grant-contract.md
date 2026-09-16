# Plan 001 ordinary grant contract

This is the additive envelope and native import contract. The local build/sync
and CLI import gates have direct tests described below; browser issuance is a
separate milestone. Legacy `Invite`, `claim` and `visit` keep their semantics.

## Wire and identity

Dialog is pinned to `4c16de9e345d2b2d888d1008c5d3f0ca990c4807`.
`DelegationPayload` identifies the `dlg` wire format as `1.0.0-rc.1`;
`InvocationPayload` uses the corresponding `1.0.0-rc.1` invocation format.
The envelope reuses Dialog's `DelegationChain::to_bytes` container serialization.
It does not assume a newer UCAN specification is wire compatible.

Flow A uses `<browser-route>#tonk-agent-v1=<base58(DAG-CBOR)>`. The exact map is:

- `version`: integer `1`.
- `seed`: 32-byte Ed25519 seed generated fresh for this invitation.
- `grants`: list of byte strings, each a complete ordinary delegation chain.

Carriers and service URLs require HTTPS (HTTP is allowed only on literal loopback
IPs or `localhost`), and reject userinfo, query strings and endpoint fragments.
No query parameters are accepted. Unknown fields, versions, absent keys, malformed
chains and over-one-MiB fragments fail closed. The private seed and all grant
bytes stay in the fragment; HTTP and shortcut-server requests must never receive
it. A shorter link cannot reduce the fragment itself. Delivery and application
code must avoid logging the bearer or retaining it in resume metadata.
`AgentInvite` has a redacted `Debug` implementation and explicit `secret_seed()`
for credential-store import. Reusing the same link retains the same key and grant
CIDs and does not issue another delegation.

Flow B uses `SpaceGrantBundle::validate` for the public grant set and the exact
CLI recipient DID. Its correlation/delivery envelope is still a later milestone;
it must not contain the CLI seed. Neither flow derives identity from account
state or performs redemption. Constructors accept an issuer-supplied seed so the
browser must generate it once per explicit new invite, not each render.

## Validation and routes

`connection::SpaceGrantBundle::validate` receives the expected recipient, exact
space-specific scopes, independently trusted access-service URL, and explicit
clock instant. All chains must name that space and recipient. Dialog checks
rooting, principal linkage and the full ancestral time intersection. The recipient must differ from every ancestor issuer, preventing root or browser
private-key export disguised as a scoped self-delegation. Every hop's
signature is verified with the existing `DidKeyResolver`. Unsupported issuer DID
methods are refused; no implicit network fallback expands trust.

Every expected scope must match exactly one leaf command and equality-policy
vector. Duplicates, omissions and extra grants fail. Every ancestor command must
be a prefix of that leaf command and every ancestor policy predicate must occur
in the leaf's expected equality predicates. This is deliberately conservative:
other logically compatible shared-space policies are not inferred equivalent.
They return `connection_unsupported_ancestor_scope`; future support needs focused
policy evidence. Each leaf must have an explicit expiry no later than any
ancestor. Executors still validate each concrete invocation and standard UCAN
revocations; offline import is not a cached authorization receipt.

Every leaf must sign `home.address` equal to the caller's independently trusted
URL. A loose query URL or nearest unsigned/descendant metadata cannot choose the
route. The caller must resolve and verify the service identity with the existing
trusted service mechanism before passing the URL; the envelope parser does not
perform service discovery. The invocation audience remains the verified service
`did:key`, not an assumed `did:web` spelling. This API does not claim a caller
supplied URL becomes trustworthy merely by passing it to validation.

Stable error categories are `connection_invalid_url`,
`connection_unsupported_version`, `connection_invalid_envelope`,
`connection_envelope_too_large`, `connection_missing_key`, `connection_invalid_key`,
`connection_subject_mismatch`, `connection_recipient_mismatch`,
`connection_recipient_not_fresh`,
`connection_scope_mismatch`, `connection_invalid_chain`,
`connection_invalid_signature`, `connection_unsupported_ancestor_scope`,
`connection_missing_expiry`, `connection_expiry_limited`, and
`connection_untrusted_route`. Errors never include the full URL or secret seed.
Service failures such as revocation remain the existing service error contract.

## Lifetime

The default requested lifetime for both flows is 90 days (7,776,000 seconds), with
an absolute signed deadline normalized to Unix seconds, the UCAN wire precision. `require_grant_deadline` rejects a requested deadline
past any ancestor limit and reports the actual limiting Unix timestamp for UI.
The issuer must surface the limit and obtain suitable durable authority; it must
not silently shorten to an operator's one-hour lifetime or substitute another
identity. Import also rejects a child explicitly outliving an ancestor.

This is a grant duration, independent of approval correlation timeouts and the
separate S3 transport ceiling. Operator rotation cannot extend it. Browser
issuance still needs an end-to-end demonstration that its retained authority can
support this duration; this contract is not evidence that existing operators can.

## Durable browser authority source

The existing browser issuance path in
`rust/tonk-worker/src/router/create_invite.rs` calls
`profile.access().claim(capability).delegate(audience).perform(&operator)`.
Pinned Dialog's `dialog-identity/src/profile/access.rs` implements this as:

1. `Claim::perform` constructs `access::Prove<Ucan>` with principal equal to the
   profile DID and propagates the requested `TimeRange`.
2. `Delegate::perform` claims that proof with the profile's signing credential,
   applies `.expires(...)`, then signs the child delegation.
3. `dialog-operator/src/operator/access.rs::resolve` adds the session suffix only
   when the requested principal is the operator DID. A profile-DID request uses
   `walk`, whose `.during(claim.duration)` requires the retained upstream proof
   to cover the requested window.

Therefore `.claim(...).expires(deadline).delegate(...)` is the high-level durable
issuance path; dynamic exact `Scope` values can use the same public
`access::Prove<Ucan>` effect, then `proof.claim(profile.signer().signer().clone())`.
Passing the operator as executor does not put its short-lived key in that grant
chain. Shared-space upstream rights still bound the profile's proof. The browser
must retain the actual public proof path and use this path, not an operator-DID
claim, before producing the candidate leaf grants.

`tests/browser_connection.rs` exercises an actual volatile `Profile` and operator
with a one-hour session, retaining an external owner's 90-day shared-space grant.
It requests each candidate scope for the profile, signs each child with the
profile credential, checks that the chain contains no operator DID, then validates
the bearer. It also requests authority beyond the retained deadline through the
same high-level API used by browser issuance and requires refusal. A second
read-only shared space cannot supply a write leaf, even with the broad operator
session. This is local
library-path evidence; browser storage, production shared-account chains, Wasm
execution and UI expiry display still need their own journey tests.

## Rights preset and local integration evidence

`candidate_build_scopes` retains its original feasibility-test name and now supplies
the six scopes used by the dedicated main-only connection importer. Real CLI
schema, data, view and blob operations have passed native remote sync/readback:

| Commands | Equality policies |
| --- | --- |
| `/use/get/memory/cell`, `/use/put/memory/cell` | `space=branch/main`, `cell=revision` |
| `/use/get/archive/block`, `/use/put/archive/block` | `catalog=index` |
| `/use/get/archive/blob`, `/use/put/archive/blob` | None |

These leaf grants authorize ordinary UCAN redelegation within their bounds. No
connection marker, management record, confirmation or activation status supplies
data authority. Distinct invites must have independent keys and leaf CIDs so
standard leaf revocation preserves siblings and affects all copied holders.

The legacy CLI remote setup tracks `meta` alongside `main` (`remote.rs`), while
`meta` contains membership, roles and invitation management. The dedicated
connection importer bypasses that helper and configures only `origin/main`,
refusing management upstreams on reopen. Native CLI coverage proves that its
schema, data, view and blob operations can use the six leaf grants without a
broader `/use` target-space grant. Existing account/legacy remote paths retain
their own behavior. Browser production issuance and hosted compatibility still
require their separate gates.

## Verification

The focused `agent_connection` tests cover retained identity across independent
imports, original grant CID retention, missing/wrong keys, wrong subject, wrong
route, missing/duplicate/broadened scopes, expiry, explicit ancestor lifetime
limits, unknown versions, tampered proof bytes and CLI-owned public-only grants.
Actual ordinary remote reads/writes and revocation are owned by access-service
integration tests. Fresh native validation on 2026-09-16: `cargo test --offline -p tonk-invite`
passed 27 existing unit tests, 10 new envelope tests, 1 real Profile/Operator
durable-issuance test and 2 historical primitive checks after the final source
changes (40 total). The first online test attempt failed
on sandbox DNS while the shared vendor dependency transition was in progress;
the final offline run above supersedes it. The browser fixture initially needed
the `DeriveOperator` trait import and deadline normalization to wire-format Unix
seconds; both corrections are included in the passing final run. Fresh
`cargo check --offline -p tonk-invite --target wasm32-unknown-unknown` passed
after the final source changes. This is a library compile check, not Wasm/browser
execution of the tests.

Owned-file `rustfmt --check` and `git diff --check` also passed. No browser,
production import, service discovery, issuance lifetime or hosted compatibility
evidence is implied by these offline tests.

## Native import boundary (milestone 2)

`tonk-cli::connections` provides a two-step inspection/discovery/validation path.
`inspect_link` verifies signatures and the fixed six rights before exposing the
signed claimed route; the CLI then runs trusted service discovery and calls
`validate_link`. `ValidatedConnection::with_directory` attaches the original
requested final working-directory binding before any credential checkpoint.

`import_at` installs the exact seed through Dialog's native credential provider
inside a private isolated profile. The outer directory contains public
`connection.json` and directory-intent metadata; private keys live only under
`credentials/invitation`, and replica data lives under `data/main`. A nested
`.connection-data` marker prevents generic opens even if the outer marker is lost.
The stable binding hashes public subject, recipient and sorted leaf grant CIDs.
The same invite resumes; distinct invite grants use distinct bindings/replicas.

The importer locks each destination separately from the short registry lock.
Unrecognized nonempty directories and symlinks/special files are rejected before
permission changes or credential access. It creates private directories with
0700 mode, hardens native secret files to 0600, and fsyncs files, directory
contents and newly created parent entries before exposing a credential checkpoint.
Checkpoints are preparing, credentials, mounted and ready. Retained credentials
can finish incomplete mounts through `open_bound` without the bearer. Missing
ready credentials, mismatched markers, contradictory outer legacy data, changed
recipient or grant set all fail without regeneration or account fallback.

Reopen cryptographically rechecks the original bundle at its saved import instant
so expired/revoked replicas remain usable for offline reads and edits. Every
remote dispatch instead requires a current proof whose prefix exactly equals one
of the retained original chains, followed by the selected profile's operator
suffix. The scoped wrapper never initializes or reads canonical account-session
state. A loaded space credential must remain verifier-only. Main must track
exactly the trusted `origin/main`; management `meta` must have no upstream. This
also prevents a local-only upstream from producing a false remote confirmation.

The current defensive symlink/special-file check recursively scans the managed
connection tree on reopen. Its cost grows with retained files; no performance
claim or premature optimization is implied. Same-user processes remain outside
the application-level isolation boundary described by the main plan.

The six focused native connection tests passed after the final importer changes,
including real scoped schema/data/view/blob sync, private native persistence,
identity/tree retention on restart, independent same-subject invitations,
URL-free saved-checkpoint recovery and corruption/legacy-open refusal. The first
local service run hit sandbox listener `Operation not permitted`; the unchanged
focused run with local-network access passed. Actual CLI subprocess checkpoint,
revocation and old-binary compatibility evidence is maintained in the main plan.

The focused retained-expiry library test also passed: a new current-time import of
an expired link is refused, while a historical retained credential can reopen,
commit an offline schema edit and reopen again with the same tree. Its new remote
push is denied without initializing the ambient account store. An initial test
fixture DSL description needed quoting; that fixture correction is included in
the final passing result.
