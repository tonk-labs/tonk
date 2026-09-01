# Custody minting moves to the worker

## What the page does now

Five methods, each running a ceremony end to end: `createAccount`,
`enrollCustodyPasskey`, `unlockWithPasskey`, `publishEncryptionKey`,
`authorizeDevice`. Between them they derive keys, generate the account
secret, seal it, mint delegations, sign invocations, and hand the
results back as hex.

That is why enrollment carries four hex fields, and why the four
call sites that have no ceremony at hand cannot enroll at all: two of
those fields are signatures by the custody key, which exists only inside
a live passkey assertion in the page.

## What it does instead

Two calls. Both return handles; neither returns key material.

```
// A new passkey. Creates the credential, then evaluates it.
navigator.tonkIdentity.createPasskey({ label, accountDid })
  -> { credentialId, key: CryptoKey, kek: CryptoKey }

// An existing one. One assertion, one prompt.
navigator.tonkIdentity.usePasskey({ credentialId? })
  -> { credentialId, key: CryptoKey, kek: CryptoKey }
```

`key` and `kek` are the two PRF outputs, each imported as a
non-extractable HKDF base with `deriveKey` and `deriveBits`. The page
never sees bytes: `importKey` takes the PRF output and it is unreachable
from that moment.

The worker receives them by `postMessage` and derives what it needs —
the custody signer through `deriveBits` (Ed25519 has no `deriveKey`
target), the KEK through `deriveKey`. Everything else it already does.

Verified: `it_carries_a_derivation_handle_across_structured_clone`. A
non-extractable HKDF key survives structured clone, which is the
algorithm `postMessage` runs, and the clone derives the same seed the
byte path produces.

## Why two handles rather than one

One handle would be the better design, and it is not available.

The derivation today takes **two independent PRF outputs**, not one
output separated twice:

```
PRF(salt = KEY_CONTEXT) -> expand(KEY_CONTEXT) -> Ed25519 seed
PRF(salt = KEK_CONTEXT) -> expand(KEK_CONTEXT) -> KEK
```

Both are then HKDF-expanded at a matching context, which is redundant —
the salts already separate them. A design starting fresh would take one
PRF output and separate it once, in HKDF, and post one handle.

But `expand(evaluation.key, KEK_CONTEXT)` is not `expand(evaluation.kek,
KEK_CONTEXT)`. Collapsing to one output changes every KEK, and a changed
KEK does not open the envelope already published for that passkey. That
is a migration with a recovery story, not a simplification.

So: two handles, which costs one extra `CryptoKey` on a `postMessage`
and nothing else. Both are HKDF bases imported the same way; they differ
only in which PRF output they wrap, which `info` the derivation names,
and what it produces.

Collapsing to one is worth doing eventually, and the moment to do it is
when something else already forces a re-seal — a KEK rotation, or the
hardware-key transition. Doing it alone would strand every account whose
passkey is the only way back: the DID changes, so a new device resolves
a cell that is not there, and the KEK changes, so finding it would not
help. That is the failure this work exists to prevent, applied
deliberately.

## What moves

| | Page today | Page after |
|---|---|---|
| WebAuthn | yes | yes |
| PRF -> handles | no | yes |
| Derive signer, KEK | yes | no |
| Generate account secret | yes | no |
| Seal it | yes | no |
| Mint consent, recovery | yes | no |
| Build containers | yes | no |

`ceremony.rs`, `custody.rs`, `request.rs` and most of `envelope.rs`
stop being page code. Whether they stop *shipping* depends on splitting
the crate, which is separate work — `install()` pulls all of
`tonk-identity` in today.

## What it settles

**The four ceremony-less call sites.** Login-enroll, re-enroll from
settings, the resend fallback, and enroll-before-link have no custody
material and no way to get one. Today that is unfixable without a
passkey prompt on each. With the worker holding a derivation handle it
mints its own, and the question disappears rather than being answered.

**The account secret never exists in the page.** It is generated in the
worker, sealed there, and the page holds neither it nor the KEK that
wraps it.

**The PRF output stops being readable anywhere.** Today it sits in page
memory as bytes for the ceremony's duration.

## What it does not settle

The Ed25519 seed still materialises as bytes, because WebCrypto has no
`deriveKey` target for Ed25519 and X25519 derivation needs the seed
anyway. This moves those bytes from the page to the worker rather than
removing them.

And the worker holds derivation capability for as long as it keeps the
handles. Dropping them after each use is possible; keeping them is what
lets the ceremony-less paths work without a prompt. That is a choice,
not a consequence.

## Shape of the change

1. **`tonk-identity`: a `handles` module.** `derive_custody_base(prf)`
   importing an HKDF key with both usages, and the two derivations off
   it — the signer via `deriveBits`, the KEK via `deriveKey`. The KEK
   half already exists in `webcrypto_kek.rs`.

2. **`install.rs` shrinks to two methods.** `createPasskey` and
   `usePasskey`, both returning `{ credentialId, key, kek }`. The five
   current methods go.

3. **The worker gains a custody module.** Receives the handles, derives,
   and does what `ceremony.rs` does today — generate the secret, seal
   it, mint consent and recovery.

4. **`EnrollCustomer` loses four fields.** The command carries `email`
   and `deposits`; the worker already holds everything else. All five
   UI call sites become the same call.

5. **The handles reach the worker.** A `postMessage` carrying two
   `CryptoKey`s, received where the worker keeps its own signers.

## Order

Steps 1 and 3 first, behind the existing hex path, so the worker can
mint before anything depends on it. Then 2 and 4 together, since the
command shape and the page API change as one. Step 5 throughout.

Nothing here blocks the branch it sits on: enrollment already writes the
cell, and this changes who signs what, not what is written.
