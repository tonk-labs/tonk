# tonk-access-service

A UCAN-authorizing proxy that turns verified UCAN invocations into pre-signed R2 (S3) requests.

This crate is a Cloudflare Worker. It sits in front of Cloudflare R2 storage and never proxies object bytes itself: a client sends a CBOR-encoded UCAN invocation container, the service verifies the delegation chain and invocation, and on success returns a pre-signed S3 request descriptor (URL, method, headers) that the client then uses to talk to R2 directly. The R2 credentials stay on the service; clients only ever hold UCANs. Verification and presigning are delegated to `dialog-remote-ucan-s3`'s `UcanAuthorizer` (over `dialog-remote-s3`).

## Endpoints

The Worker (`src/lib.rs`) routes:

- `POST /ucan/`: authorize a UCAN invocation container and return a pre-signed S3 request.
- `OPTIONS /ucan/`: CORS preflight (returns 204; `/ucan/` responses carry permissive CORS headers).
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

The handler reads the body, builds a `UcanAuthorizer` from the Worker environment, and calls `authorize(&body)`. On success it returns the `AuthorizedRequest` as CBOR (`Content-Type: application/cbor`) carrying:

- `url`: pre-signed S3 URL
- `method`: HTTP method (GET, PUT, DELETE)
- `headers`: headers to send with the request

On failure it returns a JSON error (`{ "error": { "code", "message" } }`). Error codes map verification outcomes to HTTP status (see `src/error.rs`): `INVALID_ARGUMENT` (400); `SIGNATURE_INVALID` / `AUDIENCE_MISMATCH` / `INVOCATION_EXPIRED` (401); `CHAIN_INVALID` / `COMMAND_MISMATCH` / `SUBJECT_NOT_ALLOWED` / `CREDENTIAL_REVOKED` (403); `INTERNAL_ERROR` (500); `REVOCATION_UNAVAILABLE` (503).

## Credential screening

Revocation is checked inside the chain walk: the authorizer carries a `RevocationChecker` backed by `REVOCATIONS_KV`, so each link is measured against the principals entitled to revoke that link. One screen remains outside it, for the question the walk does not ask: the window the chain claims.

### Time window

The chain verifier computes the intersection of every hop's time bounds and hands it back as a `TimeRange`, but `InvocationChain::verify` discards it — so nothing on this path ever compared it to the clock. `src/expiry.rs` closes that: `collect_window` carries the latest `not_before` and earliest `expiration` across the invocation and every delegation, and a presign outside that window returns `401 INVOCATION_EXPIRED`.

Unbounded chains are unaffected. A `root → device` grant carries no expiration, so its window is open and every check passes; only a chain that bounds itself can fall outside one. That is what makes this safe ahead of the clients that will start presenting short-lived session delegations — and it is the enforcement those sessions depend on, since an expiry nothing checks buys nothing.

### Revocation

A revocation is an ordinary `ucan/revoke` invocation, so it arrives at `POST /ucan/` like everything else and is answered before the presign path: it writes the index rather than reading it. The service verifies the artifact, refuses a subject it holds nothing for, and records one `REVOCATIONS_KV` key per `(revoked delegation, revoking subject)` pair. The key is the fact, so concurrent revokers cannot clobber each other the way a shared set value would.

A presign reads that index during verification rather than after it. `UcanAuthorizer::with_revocations` supplies the checker, and the chain walk asks per link: *did any principal entitled to revoke THIS link do so?* The candidates are the issuers at or above the link plus the link's own audience, who may always disclaim what it was given. Scoping matters — one flat set of issuers applied to every hop would let a principal revoke the grant its own authority rests on.

A match returns `403 CREDENTIAL_REVOKED`; clients accept the legacy `DEVICE_REVOKED` code during rollout. A failed index read is kept distinct from a denial and answers retryable `503`, since a store outage is the service's fault rather than the caller's. Mutable account rows and issuer DID strings are not enforcement inputs.

## Configuration

The authorizer is constructed per request from the Worker environment (`src/handlers/ucan.rs`):

- `R2_ACCOUNT_ID` (var): used to build the endpoint `https://{account_id}.r2.cloudflarestorage.com`
- `R2_BUCKET_NAME` (var): target bucket
- `R2_ACCESS_KEY_ID` (secret): R2 access key
- `R2_SECRET_ACCESS_KEY` (secret): R2 secret key
- `ACCOUNT_SERVICE_URL` (var): account provider returned by
  `/.well-known/tonk`.

The S3 address uses region `auto`, as R2 requires.

## Running

As a Cloudflare Worker, build and deploy with `worker-build` / `wrangler` like any `worker`-based crate (the `cdylib` target).

`wrangler.toml` at the repo root carries three environments. The top level is production on `tonk.network`, `[env.staging]` is `staging.tonk.xyz`, and `[env.preview]` is on no route: the deploy workflow uploads one version of it per pull request under a `pr-<number>` preview alias, reached at a workers.dev URL. Each has its own R2 buckets, because this Worker presigns writes into whichever bucket it is bound to and a preview must not be able to write into a real one.

`ACCOUNT_SERVICE_URL` is checked in for production and staging but overridden per pull request for preview, since the account worker's own alias URL is not known until its upload returns. The checked-in preview value points at an unresolvable host on purpose: a preview that failed to wire itself has to break rather than serve a real service through `/.well-known/tonk` to browsers that trust it. Bootstrapping the preview environment is described in `tonk-account-service`'s README.

For local development and integration tests, the `helpers` feature builds a native HTTP server that mirrors the Worker behavior without deploying to Cloudflare. The `tonk-access-local` binary (`src/bin/local.rs`, requires `--features helpers`) starts that server against a local backing S3 and prints its URL:

```sh
cargo run --bin tonk-access-local --features helpers
# ACCESS_SERVICE_URL=http://127.0.0.1:8080
```

The `helpers` module also exposes `AccessServiceAddress` (connection info usable as a test parameter on all platforms, including wasm32) and re-exports the native `access_service` server. The `integration-tests` feature gates tests that require these local servers.
