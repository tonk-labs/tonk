# CLI invitation sender attribution

Status: implemented and verified locally on 2026-10-20; extends the worker fix.

Use the existing CLI `site::member_did` resolver to stamp the leaf grant with
the shared `invitation.inviter` metadata. Prove the space to that exact account,
then prove that account to the device, before signing. The shared invite
constructor checks that the declared sender reaches the signer through identity
grants. Keep the original founder role separate and preserve legacy invitation
interpretation.

Verification checkpoints:

1. Extend the existing open-invitation mint/claim test to check the signed sender.
2. Exercise real CLI library mint, claim and sync against the local access
   service: Alice invites Bob, Bob invites Carol, Carol's replicated facts name
   Bob as inviter and Alice as founder. Reject the wrong account's redemption.
3. Run that flow for Bob's onboarding identity and for a linked account root.
   Check account DIDs differ from signing devices and from each other.

The integration tests call the CLI's production library entry points; they do
not exercise shell argument parsing, browser UI or hosted deployment. Use the
existing isolated target directory from the discovery spike and filter captured
synthetic activation links from runtime logs. Preserve unrelated local changes.

Results:

- Seven focused CLI tests passed in 15.84 seconds: both reshare scenarios,
  open mint/claim roster facts, open claim with real remote sync before account
  linking, two display-name cases, and minting without a separate relay.
- Both reshare scenarios verify the signed sender, exact recipient, absence of
  a bearer fragment, completed initial pull and roster publication, and Carol's
  persisted invitation/provenance records. Alice is the sole founder; Bob and
  Carol are members; Alice has no inviter. Mallory is refused without a joined
  site and is absent from the shared roster. Alice subsequently pulls the Bob
  to Carol invitation into her original replica.
- The unchanged worker integration binary was rerun and passed in 17.27 seconds:
  email lookup, address-claim conflict, wrong-account refusal, Bob as Carol's
  inviter and Alice as founder. Its pre-existing account-backup version mismatch
  warning remains; backup recovery is outside this proof.
- Workspace formatting and scoped whitespace checks passed. No dependency or
  lock-file changes were needed for this CLI increment.

The relay-less remote test intentionally uses an unreachable example endpoint
and reports its expected ticket-to-full-link fallback warning. Hosted services,
shell argument parsing, browser/WASM runtime, real email and cross-device
account recovery remain untested. Legacy links retain their old attribution.

Build and focused test command (local listeners required):

```sh
CARGO_TARGET_DIR=/private/tmp/tonk-identity-discovery-target RUST_MIN_STACK=33554432 \
  cargo test --offline --profile ci-test \
  --config 'profile.ci-test.opt-level=0' \
  --config 'profile.ci-test.package."*".opt-level=0' \
  -p tonk-cli --test cli --features integration-tests -- \
  invitation_attribution:: \
  site::when_recording_roster_facts::it_records_roster_facts_on_mint_and_claim \
  site::when_minting_an_invite \
  site::when_claiming_before_account_linking --test-threads=1
```

The first runtime attempt rejected the onboarding reshare and the existing open
invite test with `invitation inviter is not bound to the signing identity`.
Public-DID diagnostics showed that generic device proof search could select a
valid space-access path through a root different from `member_did`. The current
onboarding secret and its stored identity grant agreed with `member_did`.
Minting now explicitly proves through that account using the existing prefix
recovery helper, composes its identity grant, and derives the leaf's bounds from
that proof. No validator exception or replacement ownership edge is introduced.
Temporary diagnostics were removed. The test also invokes the production
founder-recording step, which the lower-level `TestSite::new` fixture omits.
After the proof-path fix, onboarding reshare and open invite passed. The linked
account fixture reported `descriptor remote is not canonical`; its URL was
normalized with a trailing slash, matching the existing account-authority tests.

## PR verification on current main

On 2026-10-20, transferred only the reviewed discovery and attribution changes
into an isolated worktree based on `3abfe85a8b`. The original checkout and its
unrelated bench/MCP work were not modified. Shared/worker changes are committed
as `f44c83761b`; the CLI increment is `40ae0a3769`.

Rebuilt native tests in a fresh `/private/tmp/tonk-discovery-pr-target`, using
the commands above with `--locked` and `--no-run`. No source adaptations were
needed for the newer base. Ran the resulting binaries with a 32 MiB test-thread
stack and local service listeners:

- 49 invitation-library tests and seven invitation-schema tests passed.
- Worker discovery/reshare plus seven join-failure tests passed (eight tests,
  73.86 seconds while the CLI build ran); the separate mint-refusal test passed.
- All seven focused CLI tests passed in 12.47 seconds, including both account
  variants of Alice → Bob → Carol and wrong-account refusal.
- Workspace formatting, staged/unstaged whitespace checks, Storybook build
  freshness and all 185 local Storybook links passed.

The worker account-backup warning and expected relay-less full-link fallback
warning remain as described above. This is local native evidence, not hosted
CI, browser rendering or deployment verification. The Storybook addendum covers
`COLLAB-01` and `COLLAB-02` without changing existing visual captures.
