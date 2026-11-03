# Host and kernel speak UCAN

The host already mints UCANs (the device's link, a session's delegations)
and checks them (what opens the account). Rather than hide that, the
interface between a kernel and its host is invocations: what a kernel asks
its host for, it invokes, signed by its session credential, with the proofs
it found in its own store; the host verifies the invocation and performs it.
Both sides read UCANs through the same Dialog crates (`dialog-ucan`,
`dialog-capability`), so there is one implementation of the rules, not two.

This replaces `persona-authority` and the two lifecycle callbacks that lent
it (`lifecycle.authorize`, `lifecycle.merge`): a function cannot return a
`borrow`, which is why the key was lent through a call into the guest. With
the persona's operations invoked like anything else, nothing is lent.

## What a kernel invokes

Each is a command over a subject the host answers for. The invocation is
issued by the caller's session credential; its proofs reach back to the
subject. Its arguments are the call's, so a signed invocation cannot be
replayed for another call.

| Command | Subject | What the host does | Answers |
| --- | --- | --- | --- |
| `/be/persona/delegate` | the persona | the persona signs a delegation | ucan |
| `/be/persona/confide` | the persona | reseals a secret to a custodian | secret |
| `/be/persona/restore` | the persona | restores a key backed up to the persona | credential |
| `/be/persona/backup` | the persona | adds a recovery, such as a passkey | credential-backup |
| `/be/persona/unlink` | the persona | revokes the persona's delegation to a device | nothing |

`/be/persona/delegate` may be narrowed by policy over the commands it hands
on.

The device delegates only to sessions: the host signs the hop from the
device to a session when it boots a worker or renews its session, and that
is all the device ever signs. So no kernel invokes the device. The profile's
session stays a key of its own rather than the device's: a session's chain
expires and is renewed, so a session can go stale or be cut off without
unlinking the device, and the device never signs what a kernel built.

Nothing is exempt. The persona's own worker invokes the same way, through
the delegation the persona gave its device and the device gave its session;
its session credential is derived as every worker's is.

## Boot

A kernel's authority is the subject it is booted for, and nothing more. The
boot names that subject and hands the session its delegation over it:

```wit
record boot {
  /// The standing this worker acts under.
  session: session,
  /// What this worker is booted for: the space it serves, or the profile.
  /// Its layer of `memory:space` is this subject's.
  subject: did,
  /// The chain from `subject` down to this worker's session: everything
  /// the session may do.
  delegations: list<ucan>,
  stack: list<stacked>,
  landing: option<landing>,
}
```

The profile's worker can do more than a space's only because its subject is
the profile, whose chain reaches what the profile reaches; a space's reaches
the space. A kernel takes up a chain only where `subject` signed it all the
way down to its session, keeps it in its store, and proves with it.

Preparing a boot, the host signs the hop from the device to the new session,
and where it lacks the hop above (the space's to the persona) it asks the
persona's worker through `guest`. A session is handed what its subject
needs and no more: none gets `/be/persona/*` by default. One that holds a
delegation of them, because the person granted it, can invoke them like any
command. Today every kernel asks for its chain
itself (`representative.delegate` at boot, `tonk-kernel/src/lib.rs`); that
goes.

## The persona's key

The host opens it with one of the person's recoveries, such as a passkey,
when a `/be/persona/*` invocation needs it, keeps it for a short window
(one minute, or the end of a sign-in), and drops it. Invocations inside the
window cost one prompt; the first after it prompts again. The prompt says
what is asked and by whom: a space asking to restore a key reads differently
from the profile linking a device.

## The WIT

```wit
interface capability {
  use types.{did, ucan, error};
  use credential.{argument};
  use session.{session};

  /// An invocation a kernel built of `command` over `subject`, with the
  /// proofs it found for it: what Dialog's `invoke` answers before it is
  /// performed.
  resource invocation {
    constructor(subject: did, command: string, arguments: list<argument>, proofs: list<ucan>);
    /// The invocation signed by `session`'s credential, to hand on.
    sign: async func(session: borrow<session>) -> result<ucan, error>;
    /// Sign it with `session`'s credential and have the host perform it.
    /// Answers what it performed, as DAG-CBOR.
    perform: async func(session: borrow<session>) -> result<list<u8>, error>;
  }
}
```

The host's own effects get typed resources over the same shape, where WIT's
lack of generics would otherwise leave the kernel decoding CBOR:

```wit
resource restore {
  constructor(persona: did, backup: credential-backup, proofs: list<ucan>);
  perform: async func(session: borrow<session>) -> result<credential, error>;
}
// …and `confide`, `persona-delegate`, `add-recovery` and `unlink`, each
// answering its own result.
```

In a kernel, `tonk-kernel-world` implements Dialog's providers over these,
so kernel code stays Dialog's:

```rust
Subject::from(persona)
    .attenuate(Restore)
    .invoke(backup)
    .perform(&operator)
    .await?
```

Dialog `claim`s the chain in the kernel's store, the adapter builds the
invocation with it, and performs it with the session.

`session.wit`: one `session` resource for every worker: `credential()` and
`persona() -> did`. The device is no handle in the kernel: it signs only the
host's hops to sessions. The profile's worker and a space's differ only in
what they were delegated, and so in what they can prove.

`credential.wit`: the `representative` resource, `persona-authority`,
`provisional-persona` and `permanent-persona` go. Signing in stays the
host's:

```wit
/// Ask the person to sign in, or up, through the host: who they are, such
/// as an email, and a passkey. Answers the persona they signed in as, its
/// key held for the window, or none when they declined. The worker moves
/// what it holds for its provisional persona to it, invoking
/// `/be/persona/*` within that window.
sign-in: async func(session: borrow<session>) -> result<option<did>, error>;
```

`lifecycle.wit`: `merge` and `authorize` go.

`guest.wit` stays: the host asks the persona's worker for the hop it lacks
when it prepares another worker's boot.

## Order

1. WIT, and the native host verifying and performing invocations with the
   Dialog crates, with its tests. The boot names its subject and passes the
   chains the host holds.
2. `tonk-kernel-world`: Dialog providers over the invocation resources.
3. The kernel: `merge.rs` and `authorize.rs` move onto invocations, and
   `session::delegate` goes: the boot hands the chain. The custody context
   the kernel passed back goes.
4. The browser host (`kernel_boot.js`): the held key and its window, the
   page's prompt per invocation.
5. Grants: a space gets `/be/persona/*` delegations only where the person
   grants them.
