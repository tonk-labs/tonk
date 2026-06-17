# tonk-access-service

A UCAN-authorizing proxy that turns verified UCAN invocations into pre-signed R2 (S3) requests.

This crate is a Cloudflare Worker. It sits in front of Cloudflare R2 storage and never proxies object bytes itself: a client sends a CBOR-encoded UCAN invocation container, the service verifies the delegation chain and invocation, and on success returns a pre-signed S3 request descriptor (URL, method, headers) that the client then uses to talk to R2 directly. The R2 credentials stay on the service; clients only ever hold UCANs. Verification and presigning are delegated to `dialog-remote-ucan-s3`'s `UcanAuthorizer` (over `dialog-remote-s3`).

## Endpoints

The Worker (`src/lib.rs`) routes:

- `POST /ucan/`: authorize a UCAN invocation container and return a pre-signed S3 request.
- `OPTIONS /ucan/`: CORS preflight (returns 204; `/ucan/` responses carry permissive CORS headers).
- `GET /`: service info as JSON (`service`, `version`).
- `GET /health`: liveness check (`OK`).

### `POST /ucan/`

The request body is a CBOR-encoded UCAN container following the [UCAN Container spec](https://github.com/ucan-wg/container):

```text
{ "ctn-v1": [invocation_bytes, delegation_0_bytes, ..., delegation_n_bytes] }
```

The handler reads the body, builds a `UcanAuthorizer` from the Worker environment, and calls `authorize(&body)`. On success it returns the `AuthorizedRequest` as CBOR (`Content-Type: application/cbor`) carrying:

- `url`: pre-signed S3 URL
- `method`: HTTP method (GET, PUT, DELETE)
- `headers`: headers to send with the request

On failure it returns a JSON error (`{ "error": { "code", "message" } }`). Error codes map verification outcomes to HTTP status (see `src/error.rs`): `INVALID_ARGUMENT` (400); `SIGNATURE_INVALID` / `AUDIENCE_MISMATCH` / `INVOCATION_EXPIRED` (401); `CHAIN_INVALID` / `COMMAND_MISMATCH` / `SUBJECT_NOT_ALLOWED` (403); `INTERNAL_ERROR` (500).

## Configuration

The authorizer is constructed per request from the Worker environment (`src/handlers/ucan.rs`):

- `R2_ACCOUNT_ID` (var): used to build the endpoint `https://{account_id}.r2.cloudflarestorage.com`
- `R2_BUCKET_NAME` (var): target bucket
- `R2_ACCESS_KEY_ID` (secret): R2 access key
- `R2_SECRET_ACCESS_KEY` (secret): R2 secret key

The S3 address uses region `auto`, as R2 requires.

## Running

As a Cloudflare Worker, build and deploy with `worker-build` / `wrangler` like any `worker`-based crate (the `cdylib` target).

For local development and integration tests, the `helpers` feature builds a native HTTP server that mirrors the Worker behavior without deploying to Cloudflare. The `tonk-access-local` binary (`src/bin/local.rs`, requires `--features helpers`) starts that server against a local backing S3 and prints its URL:

```sh
cargo run --bin tonk-access-local --features helpers
# ACCESS_SERVICE_URL=http://127.0.0.1:8080
```

The `helpers` module also exposes `AccessServiceAddress` (connection info usable as a test parameter on all platforms, including wasm32) and re-exports the native `access_service` server. The `integration-tests` feature gates tests that require these local servers.
