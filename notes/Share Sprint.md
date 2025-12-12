# Share Sprint

## Glossary

### Principal

Principal is either an `issuer` or an `audience` of the UCAN delegation identified by a [decentralized identifier (DID)][DID]

### Authority

Authority is a [`did:key`][] [principal][] deterministically derived using uses [HKDF][] key derivation via WebAuth API with [PRF extension][]

> ℹ️ Derived [Ed25519][] keypair can be stored in non-extractible form and helps avoid repeated web authorization flows involving finger scans

### Secret Vault

Users may and are encouraged to have ==Secret== vaults, principals that can be used to recover access in scenarios when [authority][] can not be regained through WebAuth API.

> ℹ️ _Ideally these would be hardware keys, but paper keys buried underground or somewhere in the safe deposit box would suffice also._

### Profile

Users may have multiple profiles each derived from the same [authority][] concealing relation to a same user. Connection can be selectively disclosed by the user, yet choosing privacy as default.

Derivation combines signatures from the authority using [HKDF][] to avoid WebAuthn prompts when switching profiles.

Created profiles SHOULD delegate full authority to the [Secret Vault][] to enable recovery.

### Operator

Operator is [principal][] representing a program acting on user behalf.

> We used to call them agents before LLMs co-opted the term.

### Session

Authorization session or session for short represents a UCAN delegation issued by the user [profile][] to authorizing [operator][]  with access to set of capabilities for some duration of time.

> ℹ️ Typically on activation application will generate an [operator][] key and use it to requests from runtime an authorization for desired set capabilities.
>
> Runtime prompts user and if they approve, issues delegation from active user [profile][] to an [operator][] allowing program to invoke them for the duration of the session.

### Space

Space is a [principal][] representing a collaboration unit & an access control boundary. Space is a `subject` and a root `issuer` in delegation chains.

Once [space][] keypair is generated full access MUST be delegated to all the [profile][]s that represent it's [owner][]s, after which private key MUST be discarded _(This ensures that space creator has no more authority over the space than owners do)_.

> ℹ️ It is worth calling out that there is no way to enforce that creator discards a key or that they don't use deterministic key generation.
>
> ⚠️ Once key is discarded no more owners could be added to the space. However forking space should be something we could do and make it very easy.

### Owner

Owner is a role of the [principal][] that has full access to a [space][] delegated to by the [space][] [`did:key`][].

Direct delegation ensures that no intermediary principal (in delegation chain) can revoke access to the [space][].

> ⚠️ This does imply that space creator is technically able to revoke access of anyone if they have not discarded keys or if they can regenerate it.
>
> To address this limitation we could adopt [BLS](https://en.wikipedia.org/wiki/BLS_digital_signature) keys in the future enabling true co-ownership.

### Invitation

Invitation is a UCAN delegation issued by the member of the space to a `did:mailto` identified audience.

The invitation is stored in the [space][] that is the `subject` of the delegation. The hash of the delegation serves as unique invitation ID.

Signature of the invitation is used as a part of the two-factor derivation scheme used to produce [membership][] key.

### Membership

Membership is a [principal][] representing an individual's ongoing access to a [space][]. The membership private key is cryptographically derived by combining:

- The signature from the [invitation][] delegation (stored in space)
- An invite code (sent via email)

This two-factor derivation ensures that neither access to space alone nor email access after expiry can recreate the membership key.

### Invite Code

An invite code is a short, user-friendly secret derived by the inviter signing the invitation hash. Typically the last 5 characters are used to make it easy for recipient to enter them on keyboard.

The invite code serves as one half of the secret material needed to derive the [membership][] key, with the invitation signature providing the other half.

### Authorization Space

Authorization space is a regular [space][] that stores UCAN delegations that have `audience` of the authorization space.

Principals like [profile][] or [membership][] have an implicit authorization space for holding all the delegations they can recover on any device with access to the [profile][] / [membership][].

## Goals

### Invite collaborator via email

> This takes core building blocks covered in the glossary to facilitate desired experience

1. If [authority][] does not yet exist it is derived using web-auth and [PRF extension][] with [HKDF][] as described in the [Authority](#authority) section, and stored in non-extractable form.

   > ⚠️ It should be stored in separate "system" origin so that applications can not gain unauthorized access to it.

   Full authority is delegated from created [authority][] to the [secret vault][] to make recovery possible. Delegation must be stored in the [authorization space][] of the [secret vault][].

2. If [profile][] does not exist, a new one named `"default"` is derived from the [authority][] using the signature-based [HKDF][] approach and stored in non-extractable form.

   > ⚠️ It should be stored in separate "system" origin so that applications can not gain unauthorized access to it.

3. The [space][] is created locally using randomly generated keypair by the system ensuring that:

   1. The [space][] delegates complete authority to an active [profile][] making them an [owner][] of the space.
   2. Delegation is stored in the [authorization space][] of the [profile][] so that access to it can be recovered by the user across devices.
   3. The [space][] delegates complete authority to all the [profile][]s that are intended to be co-[owner][]s.

4. Inviter creates an [invitation][]:

   1. Creates UCAN delegation: `Space → did:mailto:invitee@example.com` with desired capabilities
   2. Signs delegation with inviter's key
   3. Computes invitation ID: `invitation_id = hash(delegation)`
   4. Stores delegation in space: `/access/${delegation.audience}/${hash(delegation)}`

5. Inviter derives [invite code][]:

   1. Signs invitation ID with inviter's key: `full_secret = inviter.sign(invitation_id)`
   2. Extracts user-friendly code: `invite_code = full_secret.slice(-5)`

6. Inviter derives [membership][] key and creates permanent delegation:

   1. Derives membership key: `membership_key = HKDF(delegation.signature, invite_code)`
   2. Creates delegation: `Space → Membership` with same capabilities
   3. Stores delegation in space: `/access/${membership.did()}/${hash(delegation)}`

7. Email is sent to the invitee containing:

   1. Invitation ID (hash of the invitation)
   2. Invite code (short secret for manual entry)
   3. Link with embedded time-limited UCAN invocation expiring in 7 days allowing read of `/access/${delegation.audience}/${hash(delegation)}`

8. Recipient accepts invitation:

   1. Clicks link, which provides temporary read access via UCAN invocation
   2. Reads invitation delegation from `/access/${delegation.audience}/${hash(delegation)}`
   3. Extracts delegation signature
   4. Prompts user to enter invite code from the email
   5. Derives membership key: `membership_key = HKDF(delegation.signature, invite_code)`
   6. Creates delegation: `Membership → Profile` to grant their profile access
   7. Stores delegation under `/access/${membership.did()}/${hash(delegation)}`

> ℹ️ The two-factor derivation (delegation signature + invite code) ensures security: access to space alone cannot recreate the membership key (missing invite code), and email access after the temporary UCAN expires cannot recreate it either (cannot read delegation signature).

### Invite someone from my addressbook

## Tasks


### Tonk CLI

1. `tonk account create`
   - generates authority key (no webauth here).
   - Maybe store it in a file or give back mnemonics to write down
   - In the future we'll replace this with webauthn

2. `tonk use default`
  - switches to the "default" profile derives one if does not exist yet
  - `tonk profile create --name [NAME]`
  - `tonk profile set-default [NAME]` - 🚲🏠 I like explicit language better

1. `tonk space create --name NAME [--profile PROFILE]` - creates new space, delegates ownership to creator

1. `tonk space invite --space [NAME] alice@web.mail`
  - creates an invite for `did:mailto:alice@web.mail`
  - writes the invitation into space VFS
  - generates membership keypair
  - create delegation to membership DID
  - returns file that contains
    - invite code
    - cid of the invitation
    - space DID
    - delegation to membership DID
    - invocation to access space

1. `tonk space join --invite [INVITE_FILE_PATH] --profile [PROFILE]`
   - On other device you can import produced file in previous step to accept invite and join
   - Decode file
   - retreive invitation from space
   - extract secret from invitation
   - derive membership by combining secret with invite code
   - redelegate from membership to own profile
   - store memebrship delegation in the space
   - store redeleation membership -> profile in the space


1. We can use S3 bucket to write things into


### WebAuth Integration

- `tonk login`
  - generates local [operator][] keypair.
  - open browser with url `https://auth.tonk.xyz?as=did:key:zAlice&cmd=/&sub=null&callback=http://locahost:8089`
    - query parameters tell auth endpoint to request [powerline access](https://github.com/ucan-wg/delegation?tab=readme-ov-file#powerline) to an account.
    - ~~poll delegations for `did:key:zAlice` until found or times out~~
    - ~~if delegation is discovered use it as [session] caching it locally.~~
    - once request on callback URL lands extract UCAN and sore it locally closing a window

Most of the work here happens on the `auth.tonk.xyz` static page with some JS logic which is as follows:

1. Auth using webauth or create credentials if they don't yet exist
  2. Use [PRF extension] to derive [Authority] from credentials.
  3. Disply confirmation prompt to user to accept a request
    - Probably show expiry maybe 1 month allowing user to adjust
    - Ideally (in the future) we'd also show spaces user can grant access to.
  5. Generate UCAN delegation from derived [Authority] key to audience in the query parameter.
  6. ~~Store generated delegation under `/did:web:tonk.xyz/shared/did:key:zAlice/did:key:auhority/${hash(delegation)}` in S3~~
  7. Redirect to `callback` URL adding `?approve=${encode(ucan)}` as parameter

> 🤔 Would be nice if write something if request is denied also so CLI can react

> ℹ️ There is [prior art in regards to WebAuth that we can probably borrow from](https://github.com/commontoolsinc/labs/tree/main/packages/identity)
>
> There is also alot we can borrow from https://github.com/storacha in regards to the login UI and such, asking friends there to point me where we can find relevant bits


## Appendix

### Key Derivation

All key derivation use [HKDF (RFC 5869)](https://tools.ietf.org/html/rfc5869) algorithm.

>  ℹ️ Code examples use JavaScript/WebCrypto API for illustration.

### Deriving Authority from WebAuthn PRF

```javascript
// WebAuthn credential creation with PRF extension
const credential = await navigator.credentials.create({
  publicKey: {
    challenge: new Uint8Array(32),
    rp: { name: "Share Sprint" },
    user: {
      id: new Uint8Array(16),
      name: "user@example.com",
      displayName: "User"
    },
    pubKeyCredParams: [{ alg: -8, type: "public-key" }], // Ed25519
    extensions: {
      prf: {
        eval: {
          first: new TextEncoder().encode("share-sprint-authority-v1")
        }
      }
    }
  }
});

// Get PRF output
const prfOutput = credential.getClientExtensionResults().prf.results.first;

// Import PRF output for HKDF
const prfKey = await crypto.subtle.importKey(
  "raw",
  prfOutput,
  "HKDF",
  false,
  ["deriveBits"]
);

// Derive Ed25519 seed
const authoritySeed = await crypto.subtle.deriveBits(
  {
    name: "HKDF",
    hash: "SHA-256",
    salt: new TextEncoder().encode("share-sprint-authority-v1"),
    info: new TextEncoder().encode("ed25519")
  },
  prfKey,
  256 // 32 bytes
);
```

### Deriving Profile from Authority

```javascript
// Authority signs the profile derivation context
const message = new TextEncoder().encode(
  `share-sprint-profile-v1:${profileName}`
);

const signature = await crypto.subtle.sign(
  "Ed25519",
  authorityPrivateKey, // stored in non-extractable form
  message
);

// Import signature for HKDF
const sigKey = await crypto.subtle.importKey(
  "raw",
  signature,
  "HKDF",
  false,
  ["deriveBits"]
);

// Derive profile seed
const profileSeed = await crypto.subtle.deriveBits(
  {
    name: "HKDF",
    hash: "SHA-256",
    salt: new TextEncoder().encode("share-sprint-profile-v1"),
    info: new TextEncoder().encode(profileName)
  },
  sigKey,
  256
);
```




[Operator]:#operator
[Authority]:#authority
[profile]:#profile
[principal]:#principal
[invitation]:#invitation
[membership]:#membership
[invite code]:#invite-code
[owner]:#owner
[session]:#session
[Secret Vault]:#secret-vault
[authorization space]:#authorization-space
[UCAN]:https://github.com/ucan-wg/spec/
[`did:key`]:https://w3c-ccg.github.io/did-key-spec
[space]:#space
[invite]:#invite
[Public Key Credential]:https://developer.mozilla.org/en-US/docs/Web/API/PublicKeyCredential
[Ed25519]:https://ed25519.cr.yp.to/
[DID]:https://w3.org/tr/did-core
[prf extension]:https://developer.mozilla.org/en-US/docs/Web/API/Web_Authentication_API/WebAuthn_extensions#prf
[@noble/ed25519]:https://github.com/paulmillr/noble-ed25519
[@stablelib/ed25519]:https://github.com/StableLib/stablelib/tree/master/packages/ed25519
[libsodium.js]:https://github.com/jedisct1/libsodium.js
[HKDF]:https://en.wikipedia.org/wiki/HKDF
