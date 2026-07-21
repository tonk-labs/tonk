# tonk-account-service

The account registry for tonk passkey identities: verified email binds a root
DID, a device registry tracks which devices are delegated under that root,
and encrypted delegation chains are backed up so a lost device can be
replaced without losing custody of the account.

This crate is a Cloudflare Worker. It stores account and device rows in
Cloudflare D1 (`migrations/0001_init.sql` is the schema's single source of
truth) and backs up chain bytes, keyed by content address, in an R2 bucket.
Email codes are sent through Resend. Natively (off wasm32), only the
binding-free routes (`GET /`, `GET /health`) are registered; the D1/R2/Resend
routes are wasm-only adapters (see `src/store/d1.rs`, `src/chains/r2.rs`,
`src/email/resend.rs`).

## Auth model

Every endpoint outside the two bootstrap ceremonies is invoked as a signed
UCAN invocation container: the invocation subject is the account's root DID,
and the invocation issuer is a device that has been delegated under that
root and is not revoked. The service verifies the container cryptographically,
checks the command matches the endpoint being called, then resolves the
subject and issuer to a registered account and one of its active devices
before running the ceremony. The two bootstrap ceremonies — requesting a code
and creating the account — use email verification codes instead, because at
that point no delegation exists yet to authenticate against.

## RP ID invariants

The passkey RP ID is the root-key custody boundary: every origin under it can
derive any visiting user's root key from the PRF output. This service relies
on `rp.id` being pinned to the apex, `tonk.spot`, for every production host
under it (see `tonk-identity`'s `apex_rp_id`). That pin only holds if nothing
untrusted is ever served from the apex or a subdomain of it — a hostile page
on `*.tonk.spot` could otherwise derive root keys for the same credentials.
Staging is not yet on this apex; it is moving behind Tailscale precisely so
it can stay off-apex and mint disjoint, staging-only credentials rather than
sharing the production custody boundary. Once staging's env stanza is added
to `wrangler.account.toml`, its route must not live under `tonk.spot`.

## Endpoints

The Worker (`src/lib.rs`) routes, all under `accounts.tonk.spot`. Every
`POST` route also has a matching `OPTIONS` route for CORS preflight (204,
permissive CORS headers).

- `GET /`: service info as JSON (`service`, `version`).
- `GET /health`: liveness check (`OK`).
- `POST /codes`: request an email verification code. Body: `{ "email": string }`.
- `POST /accounts`: create an account and register its first device, consuming
  a verification code. Body: `{ "email": string, "code": string, "rootDid": string, "credentialId": string, "deviceDid": string, "deviceName": string, "delegationHex": string }`.
  Returns `201` with `{ "accountId": number }`.
- `POST /devices/list`: list the devices registered under the caller's
  account. Body: a UCAN invocation container (CBOR bytes) with command
  `["account", "device", "list"]`. Returns an array of device rows (`did`,
  `name`, `status`, `delegationCid`, `createdAt`).
- `POST /devices/register`: register a new device under the caller's
  account. Body: a UCAN invocation container with command
  `["account", "device", "register"]` and arguments `did`, `name`,
  `delegation` (the new device's DID, name, and hex-encoded `root → device`
  delegation).
- `POST /devices/revoke`: revoke a device under the caller's account. Body:
  a UCAN invocation container with command `["account", "device", "revoke"]`
  and argument `did` (the device DID to revoke).
- `POST /chains/put`: back up a delegation chain, keyed by content address.
  Body: a UCAN invocation container with command
  `["account", "chain", "put"]` and argument `chain` (hex-encoded chain
  bytes). Returns `{ "key": string }`.
- `POST /chains/list`: list the chain keys backed up under the caller's
  account. Body: a UCAN invocation container with command
  `["account", "chain", "list"]`. Returns an array of keys.
- `POST /chains/get`: fetch the bytes backed up under a chain key. Body: a
  UCAN invocation container with command `["account", "chain", "get"]` and
  argument `key`. Returns the raw bytes (`application/octet-stream`).

Errors are JSON, `{ "error": { "code", "message" } }`, with `code` one of
`INVALID_ARGUMENT` (400), `UNAUTHORIZED` (401), `FORBIDDEN` (403),
`NOT_FOUND` (404), `CONFLICT` (409), `RATE_LIMITED` (429), `INTERNAL_ERROR`
(500). See `src/error.rs`.

## Deploying

Config lives in `wrangler.account.toml` at the repo root, alongside the
`tonk-access-service` and `tonk-ui` config in `wrangler.toml`. Both build
through the same `nix build .#tonk-cloudflare-artifacts` target, which places
this crate's worker output at `result/tonk-account-service/worker/shim.mjs`.

First deploy, in order:

1. `wrangler d1 create tonk-accounts` and paste the returned database id into
   `database_id` in `wrangler.account.toml` (it ships empty on purpose —
   wrangler refuses to deploy without it, so there's no risk of silently
   deploying against the wrong database).
2. `wrangler d1 migrations apply tonk-accounts --remote -c wrangler.account.toml`
   to apply `migrations/0001_init.sql`.
3. `wrangler r2 bucket create tonk-account-chains`.
4. `wrangler secret put RESEND_API_KEY -c wrangler.account.toml`.
5. Add the DKIM records Resend requires for sending from `tonk.spot` in the
   Cloudflare dashboard (DNS for the zone), so `accounts@tonk.spot` mail is
   authenticated.
6. `wrangler deploy -c wrangler.account.toml`.

There is no staging env stanza yet: staging's apex is moving behind
Tailscale, and its env should be added to `wrangler.account.toml` once that
lands (see RP ID invariants above for why staging needs its own, off-apex
host rather than reusing production's).
