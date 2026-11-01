# The persona's operations are capabilities

What a persona's key does (delegate, confide a secret, restore a backup, add
a recovery, unlink a representative) becomes methods on the persona itself,
callable from any session. Each takes a signed UCAN invocation of the
operation: issued by the caller's session credential, carrying the chain
that delegates the command to it. The persona's own worker invokes the same
way, through the delegation the persona gave its device and the device gave
the session; nothing is exempt. Its session credential is derived as every
worker's is, so it signs its invocations like any other. The host verifies the invocation, opens the
key with a passkey if it does not hold it, keeps it for a short window, and
drops it. Several operations
in one window cost one passkey prompt; one after the window costs another.

This replaces the two lifecycle callbacks (`lifecycle.authorize`,
`lifecycle.merge`) and `persona-authority`. A function cannot return a
`borrow`, which is why the key used to be lent through a call into the
guest; with the methods on the persona there is nothing to lend.

## The capabilities

Each is a command over the persona's DID (`sub`). The invocation is issued
by the caller's session credential, and its proofs reach back to the
persona:

| Method | Command |
| --- | --- |
| `delegate` | `/be/persona/delegate` |
| `confide` | `/be/persona/confide` |
| `restore` | `/be/persona/restore` |
| `add-recovery` | `/be/persona/backup` |
| `unlink` | `/be/persona/unlink` |

The invocation's arguments are the call's (the audience and access for
`delegate`, the custodian for `confide`, and so on), so a signed invocation
cannot be replayed for another call. `delegate` hands on what the persona
has, so its delegation may narrow which commands (`policy` over
`access.command`).

## The WIT

`credential.wit`:

```wit
/// A person's persona, whichever worker holds it. Its key is the
/// embedder's: the embedder opens it with one of the person's recoveries,
/// such as a passkey, keeps it for a short while, and drops it. Each method
/// takes `invocation`, the caller's signed invocation of it, whose proofs
/// reach back to this persona; the embedder refuses one it cannot verify,
/// or whose arguments are not the call's.
resource persona {
  did: func() -> did;
  /// Whether the person made themselves known: a provisional persona's
  /// key is the embedder's own, and opens with no recovery.
  permanent: func() -> bool;
  /// A delegation of `access` from this persona to `audience`.
  delegate: async func(invocation: ucan, audience: did, access: access) -> result<ucan, error>;
  /// `secret`, concealed to this persona in `context`, confided to `to`.
  confide: async func(invocation: ucan, context: list<u8>, secret: secret, to: custodian) -> result<secret, error>;
  /// The key `backup` backs up to this persona.
  restore: async func(invocation: ucan, backup: credential-backup) -> result<credential, error>;
  /// Another way to recover this persona, such as a passkey, kept by the
  /// embedder, answered as this persona's key backed up to it.
  add-recovery: async func(invocation: ucan, recovery: recovery) -> result<credential-backup, error>;
  /// Revoke this persona's delegation to `representative`.
  unlink: async func(invocation: ucan, representative: did) -> result<_, error>;
}

/// Ask the person to sign in, or up, through the embedder: who they are,
/// such as an email, and a passkey. Answers the permanent persona they
/// signed in as, its key held as one of its operations would hold it, or
/// none when they declined. The worker moves what it holds for its
/// provisional persona to it, and links its representative.
sign-in: async func() -> result<option<persona>, error>;
```

`custodian.persona` takes `borrow<persona>`. `provisional-persona`,
`permanent-persona`, `persona-authority` and the `persona` variant go.

`session.wit`: one `session` resource for every worker:
`credential()`, `representative()`, `persona() -> persona`. The persona's
own worker and a space's differ only in what they can prove.

`lifecycle.wit`: `merge` and `authorize` go.

`guest.wit` stays: the embedder still asks the persona's worker to
delegate for another worker's `representative.delegate`.

## Where the host draws the line

- The window: one minute from the passkey, or the end of the sign-in. Each
  operation inside it uses the held key; the first after it prompts again.
- The prompt says what is asked, by the operation and the caller: a space
  asking to restore a key reads differently from the profile linking a
  device.
- A space shown at its DID that has no account calls `sign-in` through the
  page that shows it; the host's panel is the one the profile uses.

## Order

1. WIT and the native host (`tonk-kernel-host`), with its tests.
2. The kernel: `merge.rs` and `authorize.rs` move onto `sign-in` and the
   persona's methods; the custody context the kernel passed back goes.
3. The browser host (`kernel_boot.js`): the held key and its window, the
   page's prompt per operation.
4. Invocations: each worker signs its own with its session credential. The
   persona's worker proves through its device's delegation; a space gets
   `/be/persona/*` delegations only where the person grants them.
