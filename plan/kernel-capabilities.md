# What a Tonk kernel needs from its host

Tonk is moving every space, and the person's profile with them, onto a
kernel: a component of the `sapling:kernel` world, built for `wasm32-wasip3`,
booted in the browser by an embedder on the Wasm Component Model Polyfill (see
`plan/space-kernel.md`). The kernel uses no web API. What it needs from where
it runs is a capability its host gives it through the world, so that a native
Sapling host and Tonk's web host can run the same kernels.

The world does not yet give everything Tonk's workers use. This lists each
capability they use, what in the world covers it, and, where nothing does, the
extension we propose. It is written to agree with Chris on the shape before
either side builds it.

The profile is a space like any other, on the same kernel. It also loads the
spaces nested in it and keeps track of which were added or removed.

## What the world covers today

The world is `sapling:kernel@0.1.0`, as Sapling defines it (PDD008):

- imports: `storage` (`layer`, `open`), `session` (`key`, `chain`,
  `sign-head`, `sign-revision-record`), `application`, `network`, and WASI's
  clocks, random and stdio;
- exports: `lifecycle` (`init`, `sync`, `shutdown`), `channel`, and
  `wasi:http/handler`.

It leaves out the file system, sockets and `wasi:http/client` (PDD008, "What
the World Leaves Out"), and a guest kernel cannot sign a UCAN (PDD007).

## Each capability

Covered means an interface of the world already carries it. P is the profile
only, S a space only, B both.

| Capability | Who | Covered by |
|---|---|---|
| Durable repository: blocks and cells | B | `storage.layer`. The web host keeps it in memory with a journal in IndexedDB; it has to grow into a real store. |
| Blobs: ranges and streaming (`/asset:`, uploads) | B | `storage.layer` `size`, `read`, `write` |
| Clock, timers, random | B | `wasi:clocks`, `wasi:random`. The gap is only a crate, `getrandom` 0.3, which has no `wasip3` backend; Tonk's one use of it is being removed. |
| Logging | B | `wasi:cli/stderr` |
| Signing heads and revision records | B | `session` |
| Watching a local cell | B | `storage.layer.watch` |
| Pulling from a peer | B | `network.open-remote` |
| Who the space is, and the delegation for it | S | `session.chain`, `storage.open(subject, chain)`, renewed by the host |
| Signing a UCAN: delegations, invites, revocations, presigns | P | **Not covered**: `credentials` (below) |
| Holding keys and secrets: profile and operator keys, site secrets | P | **Not covered**: `credentials` (below) |
| Custody: sealing and opening under a key that never leaves the host | P | **Not covered**: `credentials` (below) |
| Passkey ceremonies (WebAuthn with PRF), which need a gesture in a page | P | **Not covered** |
| Pushing to the space's remote (S3 through the access service), and its watch | B | **Not covered**: `network` pulls only |
| Asking the account service (registration, email, customer, activation) | P | **Not covered**: there is no HTTP client |
| The library a space installs (`/library/core.yaml`) | B | **Not covered** as such; can be a read-only stacked layer |
| Talking to the pages: which page asked, telling a page to navigate or what a worker is busy with, telling the other tabs of a commit | B | **Not covered**: a `wasi:http` request carries no client |
| Keeping the worker alive while it answers; retiring for a successor; forgetting the space | B | **Not covered** beyond `lifecycle.init` and `shutdown` |
| Visibility and connectivity of the device | B | **Not covered** |

## Proposed extensions

Each proposal is in the world's own style. The web host is Tonk's embedder;
the native host is Sapling's daemon.

### `credentials`: keys a kernel uses and never holds

Proposed to Sapling as PDD015 (dialog-db/sapling#5). Dialog DB already models
keys and secrets as the credentials of a subject (`dialog-effects`
`credential`): a key at an address, with `self` for the subject's own, and a
secret for each site. A credential signs, reveals what was sealed to it, derives
a key, and seals itself to another `did:key`, and its material never leaves
where it is kept (`dialog-credentials` `secret`).

A kernel reaches the credentials of a layer through the handle of that layer,
under a reach its host binds, as for cells:

- A space's kernel reaches its own keys, such as its session key, and the
  secret of the site it syncs with. It can never sign as the profile.
- The profile's kernel also reaches the profile's, the account's and the
  custody keys. A credential that names the persona acts only with consent.

The kernel signs invocations and delegations through
`dialog-credentials`' external key, whose signature comes from the host, so
`dialog-capability` builds them as it does now. Sealing to a DID needs only the
public key, so the kernel does it itself.

Two additions to Dialog DB come with it: an external key that also reveals and
derives through its host, and sealing a credential to a DID and adopting a
sealed one.

What it changes in Tonk: the account secret wrapped under a passkey's AES-GCM
KEK, and the space seeds sealed to the account's own X25519 key
(`tonk-identity` `envelope`, `sealed`, `webcrypto_kek`), become credentials
sealed to a DID.

**Migration.** The web host does it, lazily. The first time a kernel adopts or
loads a credential that the host still holds in Tonk's old form, the host opens
the old form itself, keeps the credential in the new one, and answers with the
credential. The kernel never sees the old form, a native Sapling host carries
none of it, and nothing needs a step of its own. This is Sapling's rule that the
embedder carries each adapter (PDD008, "Versioning of the World").

### `network`: a remote a space writes to

Sapling's `network` pulls from peers (PDD010). A Tonk space also pushes to its
remote, an S3 bucket behind the access service.

```wit
open-upstream: async func(remote: string, subject: did) -> result<upstream, error>;

resource upstream {
  // what `replica` reads, and:
  put: async func(catalog: string, key: hash, bytes: list<u8>) -> result<_, error>;
  publish: async func(namespace: string, cell: string, content: list<u8>,
                      when: option<version>) -> result<version, error>;
  write: async func(bytes: stream<u8>) -> result<hash, error>;
  watch: async func(namespace: string, cell: string) -> result<stream<edition>, error>;
}
```

`remote` is the descriptor already signed into the space's chain. The host
presigns and talks to S3, so the guest signs nothing.

- Web: `fetch` with streamed bodies, and a credit-based fallback where Safari
  cannot transfer streams.
- Native: the daemon dials the access service.

### `services`: the account service and other HTTP

Either:

- the profile's world imports `wasi:http/client`, and the host refuses every
  origin but the deployment's (the access service, the short-link host); or
- registration, email checks and activation are commands the host performs.

The second keeps the world without a client, as Sapling wants.

### `clients`: the pages a kernel answers

```wit
interface clients {
  type client = string;
  live: func() -> list<client>;
  notify: async func(to: client, message: list<u8>) -> result<_, error>;
  broadcast: func(topic: string, message: list<u8>);
}
```

- The host stamps each request with the client that made it (a header).
- `lifecycle` gains `clients-gone(list<client>)`, `visibility(bool)` and
  `connectivity(bool)`.
- Web: service-worker clients and `BroadcastChannel`.
- Native: the daemon's connections; `notify` writes to that connection's event
  stream.

### `ceremony`: what must happen in a page

```wit
interface ceremony {
  use credentials.{credential};
  use clients.{client};

  variant kind { register(list<u8>), assert(list<u8>) }
  record outcome { authenticator: list<u8>, custody: credential }

  request: async func(on: client, kind: kind) -> result<outcome, error>;
}
```

The host runs the ceremony in the page that holds the gesture, and makes what
PRF yields into a credential the guest cannot read. Today those bytes cross into
the worker; this keeps them out of it.

- Native: the platform authenticator, or a browser tab the daemon opens.

### `lifecycle`: holds, retiring and forgetting

- An import `hold: func() -> hold`: a resource whose drop lets the worker idle.
  Web: `waitUntil`. Native: the daemon's idle timer.
- Exports `retire` (a successor waits) and `forget` (the space is removed from
  the device).

### Between the profile and its spaces

Sapling's model fits:

- the delegation is a notice on the application channel, and the space reads
  it through `session.chain` and `storage.open`;
- invites, revocations and authority are commands of the profile, signed after
  consent;
- the web host relays the channel between the two origins over the port the
  page brokers;
- the profile stops forwarding `/api` requests to a space: a page asks the
  space's origin.

## What stays in the host

Never the kernel's to set, since a kernel is code the space holds and may
upgrade:

- each response's security policy (`content-security-policy`, `nosniff`,
  `worker-src`), forced by the bootloader;
- which load gets the shell and which the kernel answers;
- the caches of the host's own files, and the health log.

## Hardest first

1. **Who signs a UCAN.** Proposed as `credentials` (PDD015); everything the
   profile does with authority depends on Sapling agreeing to it.
2. **Pushing to the remote.** `network` is pull-only, and Tonk's remote is a
   writable hub.
3. **Passkey custody.** The gesture is in the top page of another origin, PRF
   output must stay out of the guest, and Safari's X25519 and Ed25519 quirks
   come with it.
4. **The relay between profile and space.** Ports die with either worker, and
   the seed and copy handshake has to become notices.
5. **The bootloader enforces policy.** The trusted-service-worker invariant
   (`plan/space-workers.md`) now rests on the host alone.
6. **Lifecycle under the polyfill.** Holds, retiring with open streams, and
   keeping the session overlay across restarts.
7. **The port of Tonk's crates.** About 550 web-gated lines across 49 files of
   `tonk-worker`, and Dialog's IndexedDB, OPFS and WebCrypto providers, need
   host-backed variants.
8. **HTTP for the account service.** A client or commands.
9. **Client identity.** Navigation, status, view bindings and the stale-client
   sweep all key on it.
10. **The host's store.** It has to match Dialog's IndexedDB and OPFS providers
    for speed, behind `get-many` and range reads.

Sapling's PDD007 (Daemon and Workers), PDD008 (The World), PDD009 (Commands
and Intents), PDD010 (Peers), PDD011 (Worker Authority) and PDD012 (The Boot
Protocol) are the designs these extend.
