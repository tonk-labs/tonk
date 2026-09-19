# tonk-access-service

A UCAN-authorizing gateway to R2: verified UCAN invocations become service-signed permits for one object operation each, redeemed at this same worker.

This crate is a Cloudflare Worker. A client sends a CBOR-encoded UCAN invocation container, the service verifies the delegation chain and invocation, and on success returns a permit (URL, method, headers) naming one operation on one object under the worker's own `/object/` path. The client presents that URL, and the worker performs the operation over its R2 binding and streams the bytes. Nothing S3 ever reaches a client, and no R2 credential is held anywhere: the bucket is a binding. Verification is delegated to `dialog-remote-ucan-s3`'s `UcanAuthorizer` (over `dialog-remote-s3`); the permit signing lives in `src/permit.rs`.

The worker is in the byte path on purpose. R2's S3 endpoint speaks HTTP/1.1, so a browser caps it at six connections per origin and a cold load of a few hundred blocks queues behind that cap; the worker's own origin is HTTP/2, where the cap does not exist.

## Endpoints

The Worker (`src/lib.rs`) routes:

- `POST /ucan/`: authorize a UCAN invocation container and return a signed object permit.
- `OPTIONS /ucan/`: CORS preflight (returns 204; `/ucan/` responses carry permissive CORS headers).
- `GET`, `PUT`, `DELETE /object/{key}`: perform the operation a permit names; the permit travels in the query string.
- `OPTIONS /object/{key}`: CORS preflight.
- `GET /.well-known/tonk`: same-origin browser deployment configuration.
- `GET /`: service info as JSON (`service`, `version`).
- `GET /health`: liveness check (`OK`).

`GET /.well-known/tonk` returns canonical camelCase JSON:

```json
{
  "accountServiceUrl": "https://accounts.tonk.xyz/"
}
```

The value is validated as an absolute URL. Missing or invalid configuration is
a 500; browser clients do not guess from the host or use a production default.

### `POST /ucan/`

The request body is a CBOR-encoded UCAN container following the [UCAN Container spec](https://github.com/ucan-wg/container):

```text
{ "ctn-v1": [invocation_bytes, delegation_0_bytes, ..., delegation_n_bytes] }
```

The handler reads the body, builds a `UcanAuthorizer` from the Worker environment, and calls `authorize(&body)`. The authorizer describes the one request the chain admits (method, object key, body checksum, precondition) against a placeholder address; that description is lifted off and reissued as a permit signed by the service (`src/permit.rs`). The answer is the `Permit` as CBOR (`Content-Type: application/cbor`) carrying:

- `url`: `{origin}/object/{key}?permit=…&signature=…`, at the origin the request arrived on
- `method`: HTTP method (GET, PUT, DELETE)
- `headers`: headers to send with the request (none; everything is in the URL)

On failure it returns a JSON error (`{ "error": { "code", "message" } }`). Error codes map verification outcomes to HTTP status (see `src/error.rs`): `INVALID_ARGUMENT` (400); `SIGNATURE_INVALID` / `AUDIENCE_MISMATCH` / `INVOCATION_EXPIRED` (401); `CHAIN_INVALID` / `COMMAND_MISMATCH` / `SUBJECT_NOT_ALLOWED` / `CREDENTIAL_REVOKED` (403); `INTERNAL_ERROR` (500); `REVOCATION_UNAVAILABLE` (503).

### `/object/{key}`

The `permit` parameter is the DAG-CBOR encoding of the claims — method, key, expiry (an hour from issue), the SHA-256 a write must hash to, and the precondition (`if-match` a version, or create-only) — and `signature` is an HMAC-SHA256 over exactly those bytes under a key derived from `SERVICE_SECRET_KEY`. The handler (`src/handlers/object.rs`) verifies the MAC before decoding anything, then checks the expiry and that the request's method and path are the ones the claims name. Nothing else about the operation is read from the request: a permit for one object is worth nothing against another, and a read permit is worth nothing as a write or a delete.

Answers keep the shape the client knew from S3: `200` with `ETag` on a read or write, `206` with `Content-Range` for a `Range` read, `404` for an absent object, `412` when the precondition did not hold, `400` for a body that does not hash to the bound checksum, and `401`/`403` for a permit that is not good here (lapsed, unsigned, or presented beyond what it names), which is what tells the client to redeem afresh. Nothing is buffered in either direction: reads are handed to the runtime as the binding's own stream, and a write flows chunk by chunk into the binding through a fixed-length stream (so it must declare a `Content-Length`; `411` otherwise), hashed on the way past. R2 verifies the bound checksum itself and never stores a mismatch; the hash taken in the worker only decides whether a refused write is answered as the client's fault (`400`) or the store's (`500`).

Conditional deletes are checked against the object's current version and then performed, since the binding has no conditional delete. The two steps are not atomic. Nothing in the client issues one today.

## Credential screening

Revocation is checked inside the chain walk: the authorizer carries a `RevocationChecker` backed by `REVOCATIONS_KV`, so each link is measured against the principals entitled to revoke that link. The walk also judges the validity window, intersecting every hop's bounds with the invocation's and comparing the result to the clock, and names the refusal `Expired` / `NotValidBefore` with the bound that failed.

### Revocation

A revocation is an ordinary `ucan/revoke` invocation, so it arrives at `POST /ucan/` like everything else and is answered before the presign path: it writes the index rather than reading it. The service verifies the artifact, refuses a subject it holds nothing for, and records one `REVOCATIONS_KV` key per `(revoked delegation, revoking subject)` pair. The key is the fact, so concurrent revokers cannot clobber each other the way a shared set value would.

A presign reads that index during verification rather than after it. `UcanAuthorizer::with_revocations` supplies the checker, and the chain walk asks per link: *did any principal entitled to revoke THIS link do so?* The candidates are the issuers at or above the link plus the link's own audience, who may always disclaim what it was given. Scoping matters — one flat set of issuers applied to every hop would let a principal revoke the grant its own authority rests on.

A match returns `403 CREDENTIAL_REVOKED`; clients accept the legacy `DEVICE_REVOKED` code during rollout. A failed index read is kept distinct from a denial and answers retryable `503`, since a store outage is the service's fault rather than the caller's. Mutable account rows and issuer DID strings are not enforcement inputs.

## Configuration

From the Worker environment (`src/handlers/ucan.rs`, `src/handlers/object.rs`):

- `BUCKET` (R2 binding): the bucket every object operation runs against, and that deletion purges.
- `SERVICE_SECRET_KEY` (secret): the service's 32-byte hex seed. Its ed25519 identity issues activation delegations, and the permit MAC key is derived from it (HKDF, `tonk/permit/v1`). Without it `/ucan/` cannot answer and `/object/` cannot verify, so a deployment missing it serves no data.
- `ACCOUNT_SERVICE_URL` (var): account provider returned by
  `/.well-known/tonk`.

No S3 credential is configured: the worker never presigns.

## Running

As a Cloudflare Worker, build and deploy with `worker-build` / `wrangler` like any `worker`-based crate (the `cdylib` target).

`wrangler.toml` at the repo root carries three environments. The top level is production on `tonk.network`, `[env.staging]` is `staging.tonk.xyz`, and `[env.preview]` is on no route: the deploy workflow uploads one version of it per pull request under a `pr-<number>` preview alias, reached at a workers.dev URL. Each has its own R2 buckets, because this Worker writes into whichever bucket it is bound to and a preview must not be able to write into a real one. A permit is redeemed at the origin that issued it, so each environment serves its own bytes.

`ACCOUNT_SERVICE_URL` is checked in for production and staging but overridden per pull request for preview, since the account worker's own alias URL is not known until its upload returns. The checked-in preview value points at an unresolvable host on purpose: a preview that failed to wire itself has to break rather than serve a real service through `/.well-known/tonk` to browsers that trust it. Bootstrapping the preview environment is described in `tonk-account-service`'s README.

For local development and integration tests, the `helpers` feature builds a native HTTP server that mirrors the Worker behavior without deploying to Cloudflare. It issues the same permits and serves `/object/` by forwarding each verified operation to a local backing S3 with a SigV4-signed request, standing in for the R2 binding. A write streams through it one chunk behind, hashed on the way: the last chunk is released only once the digest and length check out, so a mismatch ends the upstream write short of its declared length and the local store, which verifies no checksum of its own, never completes the object. The `tonk-access-local` binary (`src/bin/local.rs`, requires `--features helpers`) starts that server and prints its URL:

```sh
cargo run --bin tonk-access-local --features helpers
# ACCESS_SERVICE_URL=http://127.0.0.1:8080
```

The `helpers` module also exposes `AccessServiceAddress` (connection info usable as a test parameter on all platforms, including wasm32) and re-exports the native `access_service` server. The `integration-tests` feature gates tests that require these local servers.
