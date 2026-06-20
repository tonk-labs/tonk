# Invitation

An invitation lets someone join a space. Clicking Share mints a fresh
one: a new membership principal, an access delegation to it, and a link
that carries both the public delegation and the private seed. The seed
never enters replicated storage — it lives only in the session overlay.

## Concepts

```yaml
concept!:
  this: tonk:credential
  description: A did:key principal — its private seed.
  with:
    seed:
      description: The ed25519 keypair seed for this principal.
      the: xyz.tonk.credential/seed
      as: text

concept!:
  this: tonk:authorization
  description: Access granted to a did:key principal.
  with:
    proof:
      description: Base58btc UCAN delegation chain for this audience.
      the: xyz.tonk.authorization/proof
      as: text

concept!:
  this: tonk:invitation
  description: An invitation that can be used to join a space.
  with:
    access:
      description: Base58btc UCAN delegation chain for this audience.
      the: xyz.tonk.authorization/proof
      as: text
    code:
      description: The ed25519 keypair seed for this principal.
      the: xyz.tonk.credential/seed
      as: text

command!:
  this: tonk:invite
  description: Issue an invitation.
  with:
    time:
      description: Time at which the invite was requested.
      the: dom.event/time-stamp
      as: unsigned-integer
```

`tonk:invitation` is the join of `tonk:authorization` (the public
`access` proof) and `tonk:credential` (the private `code` seed) on the
same principal entity.

## Storage split

- `tonk:authorization` is **persisted** in replicated storage. The
  delegation chain is public.
- `tonk:credential` is asserted into the **session overlay** only — an
  in-memory, per-branch layer the reactor folds into read queries but
  never writes to the branch tree and never replicates. The seed is
  secret, so it stays out of storage.

The UI query for `tonk:invitation` joins both; the overlay supplies
`code`, storage supplies `access`.

## Flow

1. Share button submits the `tonk:invite` command (carrying `time`,
   which makes each click a distinct request so the effect re-fires).
2. The `tonk:invite` effect handler (in the worker) reacts:
   1. generates a new membership keypair;
   2. delegates repo access from the profile to that keypair;
   3. asserts `tonk:authorization` (the serialized UCAN chain) into
      storage;
   4. retracts any existing `tonk:credential` and asserts the new one
      (the seed) into the session overlay — so there is always exactly
      one live credential, and the prior invite stops resolving;
   5. (the matching `tonk:authorization` can likewise be the single
      current one.)
3. The UI queries `tonk:invitation` and renders the share link.

## View

```yaml
view!:
  this: tonk:invitation/view
  model: tonk:invitation
  display: |
    <form>
      <wa-input>?access={access}#{code}</wa-input>
    </form>
```

The template assembles the link from the joined fields — public
`access` before the fragment, secret `code` after the `#`.

## Rotation

Each Share click re-runs the effect, which replaces the single
`tonk:credential` (and its authorization). Only the latest link
resolves; previous links no longer find their seed in the overlay. One
live invitation at a time.

## Dialog integration

The session overlay is dialog's `Changes` batch held per-branch. Two
touchpoints:

- **Read** — dialog's query is composable: `branch.query()` returns a
  layer onto which `.with(changes)` folds an in-memory overlay that
  surfaces alongside branch facts but is never written to the tree. The
  reactor's read-query path adds `.with(overlay.read().clone())` before
  `.select(..).perform(env)`, so every read sees the credential while
  storage stays clean.

- **Write** — the effect handler mutates that same `Changes` directly:
  `changes.retract(old_credential)` then `changes.assert(new_credential)`.
  For a cardinality-one attribute a re-assert overwrites the prior value
  in place, so "always one live credential" needs no bookkeeping beyond
  asserting the new one.

The commit/transaction path is deliberately left alone — dialog's
transaction query is non-composable (no `.with`), so the overlay is
invisible to anything running mid-commit. That is the property that keeps
an ephemeral seed from ever flowing into a durable write.

## Pieces to build

- **Schema**: the three concepts + the `tonk:invite` command;
  `xyz.tonk.credential/seed` and `xyz.tonk.authorization/proof`
  attributes.
- **Reactor**: a per-branch session overlay (`RwLock<Changes>` — reads
  must not serialize against each other, only the rare credential write
  takes the exclusive lock) that the read-query path folds in via
  `QueryLayer::with`; a write method the effect handler calls to
  retract+assert the credential.
- **Worker**: the `tonk:invite` effect handler (generate keypair,
  delegate, assert authorization to storage, assert credential to
  overlay).
- **Core**: the `tonk:invitation/view` and the Share button wiring.

## Gotcha: `time` is a Float, not an integer

`dom.event/time-stamp` reads `event.timeStamp`, a `DOMHighResTimeStamp` —
a **double**. So the command's `time` field is `f64` (`as: float`), not
`unsigned-integer`. Declaring it unsigned makes the committed transient
carry `Value::Float(..)`, which the `u64` field can't decode, so the
command silently never dispatches (the transient commits, `matches`
fails, no handler runs). `Invite` therefore derives only
`PartialEq, PartialOrd` (an `f64` field can't be `Eq`/`Ord`).

## Open notes

- The membership keypair is generated in the worker (so the seed can be
  written to the worker-side overlay). The seed reaches the browser only
  on the UI read that builds the link; it is never persisted.
- The overlay is in-memory: if the reactor evicts or reloads the branch
  state, the live credential is gone, so `tonk:invitation` no longer
  joins and the share view renders nothing — until the next Share click
  asserts a fresh credential and the query matches again. Already-shared
  links are unaffected; they carry their own seed in the URL.
