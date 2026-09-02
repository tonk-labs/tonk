# Custody minting in the worker

## What the page does

WebAuthn remains page-only: `navigator.credentials` is unavailable in a
service worker, and the assertion must begin while the page still owns the
person's activating tap. The page therefore creates or asserts the passkey and
reads the PRF extension results.

For custody, the authenticator returns two independent 32-byte PRF outputs. The
page constructs one byte-only handoff:

```text
{
  type: "custody",
  credentialId: <hex string>,
  key: Uint8Array(32),
  kek: Uint8Array(32),
  request: <custody intent>,
  holderCredentialId?: <hex string>,
  holderKey?: Uint8Array(32),
  holderKek?: Uint8Array(32)
}
```

`AddPasskey` includes the optional holder triplet so the worker can open the
account with the existing passkey and re-seal it under the new one. Every other
intent carries only the primary triplet.

The page posts fresh typed arrays through structured clone. It never puts these
bytes in JSON, text, storage, logs, analytics, or a URL, and fills its typed
arrays with zero immediately after `postMessage` returns or throws. The
original Rust values remain `Zeroizing<[u8; 32]>` until they drop.

## What the worker does

The worker validates the credential id, requires each present PRF field to be a
`Uint8Array` of exactly 32 bytes, copies it into zeroizing Rust storage, and
immediately clears the received typed array. A holder is either wholly absent
or wholly valid; partial holder fields are refused.

The worker then imports both outputs as non-extractable HKDF bases with
`deriveKey` and `deriveBits`. From those handles it derives the same custody
signer and AES-GCM KEK as the original byte path. It generates or opens the
account secret, seals it, mints custody consent and recovery invocations, links
the device, and persists the result. The account secret and custody minting
remain worker-owned.

The one-request `MessageChannel` carries the reply. The page bounds that wait;
a browser that silently drops the custody message produces retry/reload UI
rather than an indefinite spinner. A service-worker `messageerror` handler logs
only a fixed deserialization diagnostic and never inspects event data.

## Why typed bytes cross the boundary

The first implementation imported each PRF output in the page and posted two
non-extractable `CryptoKey` handles. Desktop browser tests showed that those
handles survived `structuredClone` and desktop worker messaging. That evidence
was insufficient: iOS Safari accepted the passkey assertion and PRF evaluation
but silently dropped the service-worker message containing the handles before
`onmessage` ran. The same envelope with ordinary values reached the worker.

Typed arrays are the compatible transport. This is a transport change, not a
custody or wire-format migration: the PRF outputs already existed in the page
realm because WebAuthn returned them there. The new boundary creates one
transient structured-clone copy in the worker, clears both JS copies promptly,
and retains only non-extractable handles for the custody operation. No custody
DID, KEK, sealed envelope, or stored account record changes.

## Why there are two outputs

The established derivation takes two independently salted PRF outputs:

```text
PRF(salt = CUSTODY_KEY_CONTEXT)
  -> HKDF(info = CUSTODY_KEY_CONTEXT)
  -> Ed25519 custody seed

PRF(salt = CUSTODY_KEK_CONTEXT)
  -> HKDF(info = CUSTODY_KEK_CONTEXT)
  -> AES-256-GCM KEK
```

Starting from one output and separating it twice would be simpler in a new
design, but it would derive a different custody DID and KEK. Existing custody
cells would no longer resolve or open. Preserving both outputs is therefore a
compatibility invariant, not an incidental implementation detail.

## What moves

| Operation | Page | Worker |
|---|---:|---:|
| Start WebAuthn inside the activating gesture | yes | no |
| Receive the two PRF outputs | yes | no |
| Post and clear transient PRF arrays | yes | receive and clear |
| Import non-extractable HKDF handles | no | yes |
| Derive custody signer and KEK | no | yes |
| Generate or open the account secret | no | yes |
| Seal the account secret | no | yes |
| Mint consent and recovery | no | yes |
| Link and persist account state | no | yes |

The page is a narrow browser-capability adapter. The worker remains the custody
module and the only place the account secret, signer, KEK usage, and durable
account mutation meet.

## Security properties and limits

- The passkey secret never leaves the authenticator.
- PRF outputs exist transiently in the page because the WebAuthn API returns
  them there; they also exist in one transient worker clone for import.
- PRF outputs are never serialized to text or persisted and both JS typed-array
  copies are explicitly cleared.
- Imported HKDF and KEK handles are non-extractable.
- The AES-GCM KEK is derived with the existing contexts and produces the
  existing envelope wire format.
- The Ed25519 seed still materializes briefly inside the worker because
  WebCrypto has no Ed25519 `deriveKey` target.
- The worker keeps the imported derivation capability only as long as the
  custody request or parked activation-gated login needs it.
