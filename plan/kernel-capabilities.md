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
| Signing a UCAN: delegations, invites, revocations, presigns | P | **Not covered**, and excluded on purpose |
| Holding keys and secrets: profile and operator keys, credentials, retained chains | P | **Not covered** |
| Custody: X25519 agreement, HKDF, AES-GCM under a key that never leaves the host | P | **Not covered** |
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

### `keys`: keys and secrets that never enter the guest

```wit
interface keys {
  use types.{did, error};

  enum algorithm { ed25519, es256, x25519, hkdf, aes-gcm }

  resource key {
    did: func() -> option<did>;
    sign: async func(tag: string, payload: list<u8>) -> result<list<u8>, error>;
    agree: async func(peer: list<u8>) -> result<key, error>;
    derive: async func(info: list<u8>, into: algorithm) -> result<key, error>;
    seal: async func(plain: list<u8>, aad: list<u8>) -> result<list<u8>, error>;
    open: async func(sealed: list<u8>, aad: list<u8>) -> result<list<u8>, error>;
  }

  generate: async func(algorithm: algorithm, name: string) -> result<key, error>;
  load: async func(name: string) -> result<option<key>, error>;
  forget: async func(name: string) -> result<_, error>;
}
```

- Web: a `CryptoKey` kept in the origin's IndexedDB, as Dialog keeps one now.
- Native: the daemon's data directory or the OS keychain.
- Open question: Sapling keeps UCAN signing out of the guest on purpose. Tonk's
  profile mints delegations, invites and revocations, and the access service's
  presigns. Either the profile's world gets `keys` with `sign`, and only the
  profile's, or each of those becomes a command the host signs after consent
  (PDD009, PDD011). This decides the trust model, so it comes first.

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
  use keys.{key};
  use clients.{client};

  variant kind { register(list<u8>), assert(list<u8>) }
  record outcome { credential: list<u8>, prf: list<key> }

  request: async func(on: client, kind: kind) -> result<outcome, error>;
}
```

The host runs the ceremony in the page that holds the gesture, and imports
what PRF yields as `keys` the guest cannot read. Today those bytes cross into
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

1. **Who signs a UCAN.** Everything the profile does with authority depends on
   it.
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
