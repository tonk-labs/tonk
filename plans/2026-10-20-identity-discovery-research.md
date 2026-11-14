# Identity discovery and an address book

Status: investigation complete; [first integration spike](2026-10-20-identity-discovery-spike.md)
implemented and verified locally. Email targeting and wrong-account refusal
work. The spike exposed incorrect reshare attribution; the
[sender fix](2026-10-20-invitation-sender-attribution.md) now verifies that Carol
records Bob as inviter while Alice remains founder for new worker invitations.
The [CLI follow-up](2026-10-20-cli-invitation-sender-attribution.md) verifies the
same reshare flow for onboarding and linked accounts with matching attribution.
Inspected at `0df8e73610` on 2026-10-20.

## Recommendation

Build a contact book and verified aliases around Tonk's existing account root.
Start with the existing email resolver, then prove one externally hosted domain
binding. Add AT Protocol linking after those two flows share a small, typed
resolution contract.

Keep three separate records of fact:

1. A contact: this user's chosen name for a pinned Tonk root.
2. An alias binding: evidence associating an email, domain, or external DID
   with that root, including who verified it, when, and until when.
3. A capability: the existing signed authority to access a space.

Discovery returns a candidate identity; it never grants access. Names are not
unique and are not proof. An address changing hands must not silently replace
a saved root or transfer existing grants.

## Existing implementation

| Evidence | What it supplies |
| --- | --- |
| `rust/tonk-access-service/src/lookup.rs:1–28,66–73,112–162` | Email to account discovery already exists as `did:web:{service}:customer:{email-domain}:{encoded-local-part}`. Active is HTTP 200, pending 202, suspended 410, unknown 404. |
| `rust/tonk-access-service/src/service.rs:128–155` | The response includes the account `did:key`, embedded verification key, registration state, and a `TonkAccessService` sync endpoint. |
| `rust/tonk-access-service/src/handlers/lookup.rs:28–35,51–79,160–185` | Hosted lookup has a rate limiter and short-lived caching. This is a provider-backed lookup, not a universal email directory. |
| `plan/customer-email-lookup.md:1–40` | Existing design explicitly treats registration discovery as public and explains the plaintext email exposure. Do not report this as an accidental new disclosure. Historical deployment statements in this plan were not verified. |
| `rust/tonk-worker/src/router/email_status.rs:79–138` | Registration already consumes the lookup, currently projecting status and service rather than a reusable contact result. |
| `rust/tonk-worker-api/src/invite.rs:7–39`; `rust/tonk-worker/src/router/create_invite.rs:168–173` | `recipientRoot` already selects a root-targeted invitation; omission selects an open invitation. |
| `desktop/Sources/Tonk/ShareSpaceView.swift:21–49`; `rust/tonk-fab/src/logic.rs:2257–2275` | Inspected sharing controls do not provide an email/contact picker. Desktop person sharing creates an anyone-with-link invitation. |
| `rust/tonk-schema/src/identity.rs:11–43`; `rust/tonk-worker/src/router/profile_name.rs:30–41` | Display-name facts and a profile identity overlay exist. These are useful patterns, not an address book or verified identity. |
| `rust/tonk-schema/src/invitation.rs:35–56,74–102`; `rust/tonk-schema/src/membership.rs:73–95` | Invitation roots and membership provenance can be derived from delegation data and persisted as queryable facts. |
| `rust/tonk-fab/src/member_roster.rs:645–648` | The roster displays invitation relationships using member names, without email/domain verification evidence. |
| `docs/account-repository-provider-contract.md:19–21` | Account metadata storage is not end-to-end encrypted; provider access must be considered before syncing contact data. |

Sender-attribution follow-up: `Invitation::from_chain` now reads the leaf's
signed inviter and checks its identity delegation path to the signer. The
owner → member → new recipient spike verifies this distinction for new worker
and CLI invitations. Legacy leaves still use the first proof's audience and must not be
described as verified senders. Distinguish the original grantor, later delegator,
and whoever forwarded a bearer URL: a signed grant does not prove who sent a
message containing that URL.

## Protocol choices

### Email and did:mailto

Reuse Tonk's current verified registration mapping for the first increment.
Tonk's service is the attestor of the inbox-to-root relationship; publishing a
public key in a DID document does not make the service's email claim independently
self-authenticating.

The [Storacha did:mailto draft](https://github.com/storacha/specs/blob/main/did-mailto.md)
already defines the identifier spelling. It explicitly has no standards-body
standing, allows session-specific documents, and leaves proof discovery outside
the specification. Its proposed proof flow uses email/DKIM. It therefore does
not supply the global address-to-current-Tonk-key directory this product needs.
Do not equate an ordinary emailed verification link with full implementation of
that draft.

An additional connected email requires a new alias lifecycle and proof of both
the Tonk account and the inbox. Do not repurpose the one-email-per-customer
registration table into a multi-alias address book.

[WebFinger, RFC 7033](https://www.rfc-editor.org/rfc/rfc7033.html) is relevant to
later provider discovery, but depends on the identifier's domain participating.
It cannot make arbitrary consumer mail providers publish Tonk mappings.

### Domains, URLs, and did:web

[did:web](https://w3c-ccg.github.io/did-method-web/) resolves a DID to an HTTPS
`did.json` resource. A custom DNS TXT pointer is a separate naming convention,
not a requirement of did:web.

For a person who already has a Tonk root, first investigate
[DIF Well Known DID Configuration](https://identity.foundation/well-known-did-configuration/resources/did-configuration/):
it binds an existing DID to an HTTPS origin through a signed Domain Linkage
Credential at `/.well-known/did-configuration.json`. This is directly relevant
to associating an existing Tonk identity with a website.

Prototype one supported proof format, including whether the current root/device
signing path can produce it without exposing root secrets. If a custom compact
Tonk proof is simpler, document it as Tonk-specific and require both domain
publication and a signature authorized by the target root. A domain merely
publishing somebody else's public DID is insufficient.

A DNS alternative could point to the root/proof using a documented `_tonk`
record, but requires its own validation rules and account-side consent. DNS
control and HTTPS-origin control should have explicit verification labels.
Constrain the first URL flow to a controlled origin or explicit did:web path;
arbitrary profile URLs on third-party sites require another adapter.

Domain binding establishes control of a website, not legal identity. It also
does not enroll the account with a storage provider or grant access to spaces.
Discovery of provider/service metadata is a separate result.

### AT Protocol

Use AT Protocol as an optional external identity association:
handle → AT DID → explicit verified binding → Tonk root.

The [handle specification](https://atproto.com/specs/handle) supplies DNS TXT
and HTTPS resolution and requires a matching backlink from the DID document.
Use the stable AT DID as the external identifier and recheck mutable handles.
An AT account existing is not evidence it has joined Tonk.

The [OAuth specification](https://atproto.com/specs/oauth) supports authentication
with minimal `atproto` scope, but still requires the full identity/issuer checks,
PKCE, PAR, and DPoP. Authenticate the AT account while the user is authenticated
to their Tonk root; bind the two in one nonce-protected linking transaction.
Keep the Tonk passkey/root as the access authority.

Service-attested linking is cheaper than portable public proof. If independently
verifiable discovery is required, investigate publishing a Tonk binding record
in the AT repository and validating its repository proof and reciprocal Tonk
authorization. A profile bio or a public handle lookup alone is not sufficient.
AT Protocol does not provide a general public email-to-user directory.

## Proposed data and trust contract

Use ordinary Dialog concepts with account-scoped ownership:

- Contact: local contact entity, pinned root, local label, creation source.
- Binding: kind, canonical identifier, target root, evidence reference,
  verifier/issuer, checked time, expiry or refresh deadline, visibility, state.
- External account: external DID plus current verified handle and Tonk binding.

Keep aliases separate so one root can have multiple addresses and stale evidence
can be retained without remaining trusted. Keep the local contact label separate
from the person's self-asserted display name. Duplicate names are allowed.

Resolver states should distinguish verified, unverified/pending, not found,
unavailable, changed, and withdrawn. Only active confirmed email results qualify
for an email verification label. Do not interpret 202 or network failure as a
verified address or as a reason to fall back to an open invite.

New public aliases and a shareable identity card should be explicit user choices.
The existing public registration lookup is an established policy; opt-out would
be a separate compatibility decision because login currently uses it too.
Do not publish a user's address book or reverse-enumerate all their email
addresses. For an unknown incoming grantor, accept an intentionally disclosed
alias/card as a hint, verify it, and compare its root with validated authority.
For a known grantor, match the pinned contact root locally.

Initially keep sensitive contact data local. Before promising synced private
contacts, decide whether provider-readable account metadata is acceptable or
whether to add encryption. Encryption, recovery, and cross-device key lifecycle
are a separate workstream, not an incidental checkbox.

## Smallest useful checkpoints

1. **Characterization spike, 1–2 engineering days.**
   Use synthetic accounts to resolve a confirmed email, extract the root, mint
   an existing scoped invite, and prove only that root can claim. Exercise
   pending and missing addresses. Inspect owner and member resharing to settle
   sender attribution. No new DID method or directory service is required.
2. **Contact and sharing increment, roughly 1–2 weeks total including the spike.**
   Add contact save/list/rename/remove, a typed shared resolution result, email
   and contact selection in one client, verification details and root-change
   warnings. Saved contacts remain useful offline, with evidence freshness shown.
   Test persistence, account isolation, stale responses, and wrong-root rejection.
3. **Domain proof increment, another 1–2 weeks.**
   Verify one hosted origin-to-root proof, add/remove the alias, and show the
   evidence on an incoming invitation. Test mismatched roots/origins, expired
   proofs, domain reassignment, redirects, and safe public-network fetching.
4. **AT account association, another 1–3 weeks.**
   Implement verified handle resolution, linking and unlinking, OAuth account
   binding checks, and handle changes. Portable repository proof is additional
   scope if the first implementation uses a service attestation.

These are rough estimates for one engineer familiar with Tonk, including focused
tests and a basic usable UI. Allow roughly 4–8 weeks for a cohesive email/domain/AT
experience with web/Desktop integration and hardening. This excludes encrypted
contact sync, global name search, contact-provider imports, a federated email
directory, root-key migration, and replacing Tonk identity with AT identity.
The initial spike should replace these estimates with measured scope.

## Verification for implementation

Use repository-defined runners from the Nix dev environment. Commands below are
proposed gates, inspected against the current runner definitions, not results
from this investigation:

```sh
test:native:debug -p tonk-access-service -E 'test(lookup::)'
test:native:debug -p tonk-schema -E 'test(invitation::)'
test:native:debug -p tonk-invite
test:web:debug -p tonk-worker -E 'test(email_status)'
cargo fmt --all -- --check
git diff --check
```

Verify nonzero test counts. The lookup integration tests are modules in
`rust/tonk-access-service/tests/access_service/main.rs`; the old `--test lookup`
example in the module header is stale. Add regression coverage alongside those
tests and the existing schema/invitation tests; extend the actual browser share
journey and Desktop `RuntimeShareTests`/`ShareInvitationsTests` when each client
is connected. New worker routes must also appear in `router/route_table.rs`.

Required product proof: verified email → saved contact → scoped share → intended
recipient claims → wrong root rejected; matching incoming grantor is identified;
reassigned email/domain prompts review and never transfers old capabilities.
Outages and stale proof are visible without erasing saved contact identity.
Exercise local two-account flows first, then a deployed candidate with real
email delivery, HTTPS/DNS and external OAuth as applicable.

## Investigation boundary

The initial research pass inspected source and current protocol documentation.
During that pass, no application code,
accounts, external aliases, or remote services were changed. No runtime tests,
live email delivery, DNS setup, OAuth login, or deployment verification were run.
The subsequent implementation and local verification are recorded in the
linked spike and sender-attribution notes. Existing unrelated bench/MCP changes
were preserved.

This was a direction investigation of identity lookup, sharing, metadata and
protocol options, not a full security audit. Existing lookup/scoped-invite code
is reusable source evidence, not a claim that the deployed product currently
exposes this complete user journey.
