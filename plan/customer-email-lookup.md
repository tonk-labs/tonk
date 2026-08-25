# Customer Lookup by Email Address

Status: implemented
Scope: the access service worker, its control database, and the email normalization the two services already disagree about

## 1. Problem

Given an email address, there is no way to find the `did:key` it belongs to. Inviting someone by address, or checking whether an address is already known before offering to invite it, both need that direction and neither has it.

The mapping exists. It is `customer.email → customer.did` in the access service's control database, written at `/customer/enroll`. Nothing exposes it, and nothing indexes it.

## 2. Where the mapping lives

Two databases hold an email-to-DID mapping, and the choice between them is not arbitrary.

`tonk-accounts` (account service) holds `accounts.email → accounts.root_did`, the passkey identity registry. `tonk-access-control` (access service) holds `customer.email → customer.did`, the registration and billing state. The two DIDs are the same value: `customer.did` is the account root, as `rust/tonk-worker/src/router/customer.rs:35` states and enrollment enforces by invoking on the account link's issuer.

The lookup reads the access service, for three reasons.

It is already the `did:web` host. `/.well-known/did.json` is served there (`rust/tonk-access-service/src/service.rs:30`, routed at `lib.rs:79`), so a `did:web:tonk.network:...` name resolves to the same worker with no new deployment and no new domain.

It is the only side that knows whether an address is confirmed. `customer.status` is `Registered | Active | Suspended`, and `verified` carries the activation timestamp. The `accounts` table has no equivalent, so a lookup rooted there could not distinguish an address someone claimed from one they proved control of.

It already exposes customer state by DID at `GET /customer/:did` (`handlers/registration.rs:104`), so this is a second view of a registry that is public by design, not a new disclosure.

An account that has a passkey but never enrolled has an `accounts` row and no `customer` row. Such an address resolves as unknown. That is correct: it has not been confirmed, so there is nothing to attest.

## 3. The DID

The address is encoded into the DID rather than hashed, following the split that `did:mailto` uses ([spec](https://github.com/storacha/specs/blob/main/did-mailto.md)) but under our own `did:web` name:

```
did:web:tonk.network:customer:example.com:jsmith
```

Domain first, then the local part percent-encoded per the `did:mailto` `idchar` rule: alphanumerics, `.`, `-`, and `_` pass through, everything else becomes `%XX`. So `tag+alice@web.mail` is `did:web:tonk.network:customer:web.mail:tag%2Balice`.

An earlier draft hashed the address with blake3 to keep it out of the URL. That is abandoned. The input space is enumerable, so the hash bought no real secrecy, and it forced a stored hash column, a backfill script D1 could not express in SQL, and a nullable column where a missed row would be silently unfindable rather than loudly broken. Encoding the address instead makes the DID self-describing and the lookup a plain indexed equality match.

The consequence is that the address is plaintext wherever one of these DIDs is written down: a delegation, an invite record, a log line, a browser history entry. This is accepted, and it is what makes the design honest, but it should be understood before these DIDs start appearing in stored UCAN chains.

## 4. Resolution and the path

`did:web` resolution maps the DID to `https://tonk.network/customer/example.com/jsmith/did.json`, percent-decoding each path segment as it builds the URL. The local part's own percent-encoding is decoded by that same step, so `tag%2Balice` arrives at the worker as the path segment `tag+alice`.

A `+` in a path segment is legal and means `+`; it only means space in a query string. The handler must therefore read the raw path segment and must not run it through form decoding. A test pins an address containing `+`, end to end through the service.

The route is `/customer/:domain/:local/did.json`. It does not collide with the existing single-segment `/customer/:did` probe, and `run_worker_first`'s `/customer/*` entry in `wrangler.toml` already covers it.

## 5. One address, one customer

`customer.email` gains a unique index. Registration has always been one
account per address in practice, and the constraint is what lets the
lookup answer with a single DID rather than a set.

Normalization can create a duplicate where none was visible before, by
merging two spellings of one address. The service has no production
deployment, so the migration drops duplicates rather than reconciling
them: the highest `rowid`, the most recently inserted, is kept, and the
self-provided `consumer` rows of the dropped customers go with them,
deleted first while the customers they name still exist.

## 6. Normalization

The address is the lookup key, so the two sides must agree on its spelling exactly. Today they do not.

The account service normalizes to `trim().to_lowercase()` at `core/accounts.rs:93`, `core/codes.rs:43`, and `core/deletion.rs:32`. The access service only trims (`registration.rs:214`), storing whatever case the client sent. That is the form the codebase has otherwise settled on, so the access service adopts it.

`customer.email` stores the normalized address. Not a raw address with a normalized key beside it: the column itself holds the normalized form, and the invariant is unconditional. `registration.rs:214` becomes `.trim().to_lowercase()`, and that value flows into both `enroll_customer` and `update_registered_email`. Existing rows are normalized by `UPDATE customer SET email = lower(trim(email))`, which is plain SQL and runs as part of the migration.

Normalization is extracted as one shared helper so the writer and the lookup cannot drift.

Two things follow. The comparison at `registration.rs:251` becomes normalized-against-normalized, which fixes a latent bug where re-enrolling as `Alice@x.com` after `alice@x.com` triggers a pointless `update_registered_email` write. And the address as originally typed is lost. This does not affect delivery, since `email.rs` sends to the stored value and no mail server in practice treats the local part as case-sensitive, but it means an outgoing `To:` header reads lowercase.

Incoming path segments are normalized the same way before the query, so `Jsmith` and `jsmith` resolve identically despite being different URLs.

## 7. Responses

| Case | Status | Body |
| --- | --- | --- |
| `status = 'Active'` | 200 | DID document |
| `status = 'Registered'` | 202 | DID document, plus `"status": "Registered"` |
| `status = 'Suspended'` | 410 | DID document, plus `"deactivated": true` |
| no row | 404 | error |
| malformed DID path | 404 | error |

`Registered` answers 202 rather than 200 because the address is claimed but not confirmed: the DID is real and worth returning, but a caller about to act on it should know the confirmation has not happened.

`Suspended` answers 410 Gone rather than 403 or 404. The resource existed and is not currently available, which is what a suspension is, and unlike 403 it does not read as a permission failure on the caller's part. The document still comes back with `"deactivated": true`, matching DID Core deactivation semantics: a suspension is reversible, and discarding the key mapping would force every caller to re-resolve from scratch when it lifts. A suspended customer is therefore distinguishable from an unknown address, which is consistent with the address being in the URL to begin with.

## 8. The document

The key is embedded under the `did:web` name rather than referenced, so
the document verifies standalone without a resolver having to dereference
a second DID:

```json
{
  "@context": [
    "https://www.w3.org/ns/did/v1",
    "https://w3id.org/security/multikey/v1"
  ],
  "id": "did:web:tonk.network:customer:example.com:jsmith",
  "alsoKnownAs": ["did:key:z6Mk..."],
  "verificationMethod": [{
    "id": "did:web:tonk.network:customer:example.com:jsmith#z6Mk...",
    "type": "Multikey",
    "controller": "did:web:tonk.network:customer:example.com:jsmith",
    "publicKeyMultibase": "z6Mk..."
  }],
  "authentication": ["...#z6Mk..."],
  "assertionMethod": ["...#z6Mk..."],
  "status": "Active"
}
```

`alsoKnownAs` carries the `did:key`, which is the form the rest of the
system uses, so a caller need not rebuild it from the multibase. A
`did:mailto` alias would assert an identity in a method we do not
otherwise use, so there is none.

`status` carries the registration state the status code already encodes,
so a caller reading the body does not have to infer it. A suspended
customer's document also carries `"deactivated": true`.

The builder is shared with the service's own `/.well-known/did.json`
document, so the two cannot drift in `@context` or verification-method
shape.

## 9. Changes

**Migration** `migrations/0005_customer_email.sql` normalizes existing
addresses, drops the duplicates that creates, and adds the unique index.
`store/sqlite.rs` applies it as `user_version` 5, so the native store and
D1 hold the same schema.

**`email.rs`** gains `normalize_email`, the one definition of the stored
spelling, which `registration.rs` now applies at enrollment.

**`store.rs`** gains `customer_by_email` on the `Store` trait and
`SELECT_CUSTOMER_BY_EMAIL`, implemented in both backends.

**`lookup.rs`** holds the DID encoding, the address/segment round trip,
and the resolution, generic over `Store`.

**`service.rs`** gains `customer_document`, sharing a private `document`
builder with `did_document`.

**`handlers/lookup.rs`** binds it to D1 and shapes the answer, registered
in `lib.rs` at `/customer/:domain/:local/did.json`, with a native twin in
`helpers/server.rs` matched before the single-segment probe.

**CORS** matches every other public route.

Two bugs surfaced while testing, both in test-only code. The native
server in `helpers/server.rs` built its host from `req.uri().authority()`,
which is absent for the origin-form request line every real client sends,
so it minted `did:web::...`; `request_host` now falls back to the `Host`
header. The deployed worker was never affected: it reads `req.url()`,
which reassembles the authority from that header, so `/.well-known/did.json`
has always served the right host in production. The same fix applies to
the native twin of that route, which had the bug. And the test harness
matched captured activation emails by verbatim address, which stopped
finding them once enrollment normalized before sending.

## 10. Cache and rate limiting

The two work together, and the order matters. A cached answer never
reaches the worker, so it never counts against the limit: the cache
absorbs repeat lookups of one address, and what reaches the limiter is
closer to the distinct-address rate, which is the signal enumeration
actually produces.

**Only a settled answer is cacheable.** A `200` is stored in the
Cloudflare cache and carries `Cache-Control: public, max-age=60` — long
enough to absorb a burst, short enough that a suspension is not missed. A
`202` and a `404` carry `no-store` and are stored nowhere. Both are about
to change, and the change is what a caller polling an address is waiting
on: an invite flow must not be told for a minute that the person it just
invited is still unregistered. Setting the header matters as much as
declining to store it, because the header is what browsers and
intermediaries obey.

**The limit is keyed by client IP** (`CF-Connecting-IP`), 60 per minute,
declared as a `[[ratelimits]]` binding in every environment. Keying by
address would not constrain walking a list of them, which is the thing
worth limiting on an endpoint whose URL carries the address. A throttled
caller gets `429` with `Retry-After: 60`.

A limiter that cannot answer is logged and the request proceeds. The
endpoint discloses nothing a caller could not already reach through the
registration probe, so failing closed would cost more than it protects.

## 11. Tests

`lookup.rs` unit tests cover the encoding: the `did:mailto` split, the
percent-encoding of a local part, normalization before encoding, the
last-`@` split, the segment round trip (over `+`, a literal `%`, a `/`,
non-ASCII, and the RFC-legal punctuation set), and the status each
registration state maps to.

Two encodings are correct but do not resolve, and the tests say so: an
`@` inside a local part, and a `/`, which `did:web` resolution decodes
back into a path separator. Both fail closed with a `404` rather than
reaching the wrong customer, and both are vanishingly rare.

`store/sqlite.rs` tests cover the schema: an address finds its customer,
an unregistered address finds nothing, a second customer on one address
is a conflict, the store matches exactly rather than normalizing, and a
suspended customer is still found so the lookup can answer 410 with the
key.

`tests/lookup.rs` covers the wired service: the 200 and 202 answers, the
move from one to the other on confirmation, 404 for an unknown address,
resolution whatever the casing, a `+` in the local part surviving the
path, the registration probe staying reachable beside the lookup, CORS,
a DID built by a caller resolving to the customer it names, and the
`Cache-Control` walk from `404` through `202` to `200`.

The 410 answer is covered at the store level only. Nothing writes
`Suspended` yet — no admin path exists — so the integration harness
cannot produce one.
