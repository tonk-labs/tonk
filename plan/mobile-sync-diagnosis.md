# iOS Safari joined-space sync diagnosis

Reported 2026-09-09 on staging, product-wip. Join and sync succeeded, then
FABB settings opened the email setup prompt. No email was submitted and no
account switch was reported. Within a couple of minutes the user returned
via Spaces and selected the space; the page reloaded and sync failed.

The pull fails at FetchRemoteBranch -> Resolve -> Authorization ->
UnprovenSubject. HTTP 502 is the worker fallback wrapper, not independent
proof of an upstream server outage. Device metadata confirms the claimed
principal is the operator (DID ending RXjA8c), the profile ends 2FoKYPw,
and the target space ends 6ti3cr. Repository GET returns 200 and main tracks
origin/main. No invitation credentials are recorded here.

Source inspection: session::open restores a fresh persisted session without
minting. session::rotate durably retains the bounded profile-to-operator
grant. perform_join retains both the account-to-device grant and claimed
invite chain before its first pull. Dialog's proof walk can also report
UnprovenSubject when candidate envelopes are unavailable or unverifiable.
Settings opens the registration UI for a provider-free profile; that alone
has not been shown to change authority.

Experiment: temporarily extended
session::tests::it_authorizes_a_presign_chain_bounded_by_the_session to call
session::open again and prove the same space through the rebuilt operator.
`cargo test -p tonk-worker session::tests::it_authorizes_a_presign_chain_bounded_by_the_session --lib --locked --offline`
passed (1 test). Temporary change removed. This checks a native operator
rebuild over the same storage pool and a simple space-to-profile chain;
it does not reproduce an iOS worker restart or the full claimed invite.

Next: query only issuer/audience/subject metadata in profile main on the
failing device. Establish whether the session grant and every account/invite
hop exist before investigating envelope reads, validity bounds, or storage
recovery. Root cause remains unconfirmed. No product fix applied.

## Device metadata and likely mechanism

The device query returned nine delegation records. Their issuer/audience
links connect the space to profile 2FoKYPw, but the profile-to-operator grant
(blob 6SLMMLoCayCxY2i6oPo1yf3DPvfLCeb2DMFZb9gGzq5Y) names audience
CeHJs86p, not the current operator t2RXjA8c. No returned grant names the
current operator. Presence of metadata does not verify envelope validity.

Pinned Dialog 805151c, dialog-operator/src/operator/builder.rs:214-249:
extractable keys derive from seed plus context; non-extractable browser
keys derive from a signature over a fixed context. Browser signing calls
WebCrypto Ed25519. Apple documents randomized CryptoKit Ed25519 signatures:
https://developer.apple.com/documentation/cryptokit/curve25519/signing/privatekey/signature(for:)
WebCrypto working-group issue specifically records WebKit's behavior:
https://github.com/WICG/webcrypto-secure-curves/issues/28

Likely mechanism: fresh signature changes derived operator on rebuild;
session::open trusts saved expiry and does not grant the new audience.
Native proof-reopen test uses seed derivation and cannot catch this.
Next smallest confirmation: sign the same message twice with one throwaway
non-extractable Ed25519 key on the affected Safari and compare bytes.
Likely correction must give browser operators stable persisted identity
without relying on signature determinism, including recovery for existing
session records. No implementation authorized or applied.

## Confirmation on affected Safari

The user ran the throwaway non-extractable Ed25519 test on the affected
Safari: signing identical message bytes twice with the same key returned
`identical signatures: false`. Together with the missing current-operator
grant and pinned derivation source, this confirms the signature-derived
operator identity defect as the explanation for this session mismatch.
The exact full UI sequence has not been reproduced locally.

Failure chain: randomized browser signature -> different derived operator
on reconstruction -> persisted fresh session reused without minting a grant
for that operator -> no proof from current operator to profile/space ->
UnprovenSubject -> worker 502 wrapper and failed sync indication.
Settings navigation exposes reconstruction; submitting email or completing
account setup is not required for this defect. The evidence does not
establish why that particular navigation caused the page reload.

Correction requirements: stable browser operator identity independent of
signature bytes; durable recovery of older session records so the restored
operator and its bounded grant agree; preserve non-extractable key handling,
local joined-space data, and membership; verify identity plus actual proof
resolution across storage reopen and worker restart on Safari. Root cause
is diagnosed; no product fix has been implemented or deployed.

## Disposable operator audit

Current worker join resolves `current_account` and claims the invite to that
account (router/join.rs:623-630). `current_account` uses the passkey root or
an onboarding account with an account-to-profile grant (router/account.rs:
150-169). Repository-wide `Invite::visit`/`.visit(` search finds only the
invite library tests as visit callers; no worker guest-join mode remains.
Session/renewal guest-replay commentary is stale. Production worker
operator-DID references found are session-grant creation and repository
metadata; no durable membership/invite audience depends on that DID.
Operators are replaceable in the current browser flow, provided their
bounded profile grants match their actual key. Stable identity across boots
is not a requirement; earlier correction wording was too restrictive.

Boot overhead from invoking current session::rotate each boot: operator
construction (also incurred by session reuse), bounded grant signing, grant
retention in profile main (blob/fact/history commit with refresh and possible
CAS retry), plus session credential metadata write. No account-service
round trip or passkey interaction is inherently required to mint the grant.
The retention path may need branch data; do not promise entirely read-free
or network-free startup in partially hydrated states. No Safari timings
measured. No automatic expired-session delegation pruning was found in the
worker; expiry rejects authority but does not itself remove stored records.

A candidate design is a disposable operator with an in-memory bounded
profile-to-operator grant, avoiding per-boot durable session churn. Dialog
already composes in-memory session grants, but its current .allow builder
mints unbounded grants, so it cannot be used unchanged for Tonk's TTL model.
This is design guidance, not an implemented or tested correction.
