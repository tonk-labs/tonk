# tonk-account-service

The account registry for tonk passkey identities: verified email binds a root
DID, a device registry tracks which devices are delegated under that root,
and encrypted delegation chains are backed up so a lost device can be
replaced without losing custody of the account.

This crate is a Cloudflare Worker. It stores account and device rows in
Cloudflare D1 (`migrations/` is the schema's single source of truth) and backs
up chain bytes, keyed by content address, in an R2 bucket. It also relays self-certifying revocation artifacts into a separate immutable R2 bucket; the D1 device status is only an account UI projection.
Email codes are sent through Resend. Natively (off wasm32), only the
binding-free routes (`GET /`, `GET /health`) are registered; the D1/R2/Resend
routes are wasm-only adapters (see `src/store/d1.rs`, `src/chains/r2.rs`,
`src/email/resend.rs`).

## Auth model

Every registry endpoint is invoked as a signed UCAN invocation container. Most
requests are signed by an active device delegated under the account root. The
service verifies the container and command, requires exactly one proof whose
CID equals the active attachment's grant, then resolves its subject and issuer
to that account and device. Account creation and browser self-link are
instead signed directly by the passkey-derived root: successful verification,
with issuer equal to subject, proves root-key control before the service writes
the first account or another device. Account creation additionally consumes an
email verification code. `POST /codes` and creation of a pending CLI handoff
are unauthenticated and edge-rate-limited. Account preflight is authenticated
by the submitted email code and edge-rate-limited; resolving and consuming a
handoff requires its 256-bit bearer secret.

## RP ID invariants

The passkey RP ID is the root-key custody boundary: any origin allowed to use
it can derive any visiting user's root key from the PRF output. This service
relies on `rp.id` being pinned to exactly one origin, `tonk.network` (see
`tonk-identity`'s `apex_rp_id`). Every other host — `www.tonk.network`, staging,
and any wildcard hostname under the apex — is its own relying party with its
own disjoint credentials, so none of them can derive an apex root key.

Two things follow. Production account ceremonies run only on `tonk.network`;
`tonk-ui` refuses ceremonies on every other production-facing host rather than
writing a second, disjoint identity for the same person into this registry.
And staging runs off-apex on `staging.tonk.network` against its own registry,
minting staging-only credentials.

Widening the RP ID later is possible via Related Origin Requests. Narrowing it
is not: it orphans every credential minted under the wider boundary.

## Endpoints

The Worker (`src/lib.rs`) routes, all under `accounts.tonk.xyz`. Every
`POST` route also has a matching `OPTIONS` route for CORS preflight (204,
permissive CORS headers).

- `GET /`: service info as JSON (`service`, `version`).
- `GET /health`: liveness check (`OK`).
- `POST /codes`: request an email verification code. Body: `{ "email": string }`.
- `POST /accounts/preflight`: verify a submitted `{ "email", "code" }` and
  reject an already-registered address before WebAuthn. A successful check
  does not consume the code; `POST /accounts` remains authoritative.
- `POST /accounts`: create an account and register its first device, consuming
  a verification code. Body: a root-signed UCAN invocation container with
  command `["account", "create"]` and arguments `email`, `code`,
  `credentialId`, `deviceDid`, `deviceName`, `delegation` (the hex-encoded
  `root → device` delegation), and `repositoryDescriptor`.
  Returns `201` with `{ "accountId": number, "descriptorHex": string }`.
- `POST /account/repository/establish`: root-authorized set-if-absent
  descriptor establishment for an existing account. Returns the exact stored
  winner as `{ "descriptorHex": string, "created": boolean }`.
- `POST /devices/link`: register a device through a passkey self-link ceremony.
  Body: a root-signed UCAN invocation container with command
  `["account", "device", "link"]` and arguments `deviceDid`, `deviceName`,
  and `delegation`. Returns the account's exact stored `descriptorHex`; an
  old account without an established descriptor receives `409`.
- `POST /devices/list`: list the devices registered under the caller's
  account. Body: a UCAN invocation container (CBOR bytes) with command
  `["account", "device", "list"]`. Detached generations are omitted. Returns
  actionable device rows (`attachmentId`, `did`, `name`, `status`,
  `delegationCid`, `createdAt`).
- `POST /devices/register`: register a new device under the caller's
  account. Body: a UCAN invocation container with command
  `["account", "device", "register"]` and arguments `did`, `name`,
  `delegation` (the new device's DID, name, and hex-encoded `root → device`
  delegation).
- `POST /devices/detach`: validate a canonical device-signed JSON detach
  intent and detach only its exact `attachmentId` generation. It carries no
  reusable account delegation and returns a typed terminal outcome. Replays
  are idempotent; stale intents cannot affect newer attachments.
- `POST /devices/revoke`: publish the required witnessed, root-signed
  revocation artifact, then project the exact requested `attachmentId` row to
  revoked. R2
  publication happens before D1 projection. A successful response includes
  `targetDid`, `targetCid`, `published: true`, and `projection: "updated" |
  "stale"`. Compatibility fields `artifactCid`, `stored`, and `attestation`
  remain for one rollout. Projection failure does not turn canonical
  publication into failure.
- `POST /revocations`: unauthenticated relay for raw self-certifying revocation bytes. The service verifies authority and canonical CIDs, then stores immutably at `revocations/<target-cid>/<artifact-cid>`.
- `POST /links`: create a five-minute CLI handoff from `tokenHash`, `deviceDid`,
  and `deviceName`. The raw bearer secret is generated by and remains with the
  CLI; D1 receives only its BLAKE3 hash.
- `POST /links/resolve`: resolve pending device metadata with `{ "secret": … }`
  so the browser can display exactly what it will approve.
- `POST /links/complete`: durably record a fresh delegation, descriptor, and
  random attachment generation without activating it. Body: a root-signed
  invocation with command `["account", "link", "complete"]`, binding
  `tokenHash`, `deviceDid`, `deviceName`, and `delegation`.
- `POST /links/consume`: retrieve replayable completion material with
  `{ "secret": … }`. Returns `202` while pending; a successful `200` returns
  `attachmentId`, `delegationHex`, and `descriptorHex` for crash recovery.
- `POST /links/activate`: device-signed idempotent activation of the consumed
  generation. The exact proof, token hash, root, device DID, attachment ID,
  and delegation CID must match the completed handoff.
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

UCAN invocation containers and raw signed revocation artifacts are posted as
`application/cbor`. JSON is reserved for the unauthenticated code/link request
envelopes and JSON responses.

## Deploying

Config lives in `wrangler.account.toml` at the repo root, alongside the
`tonk-access-service` and `tonk-ui` config in `wrangler.toml`. Both build
through the same `nix build .#tonk-cloudflare-artifacts` target, which places
this crate's worker output at `result/tonk-account-service/worker/shim.mjs`.

For a compatible identity/sharing rollout, deploy the account worker first,
then the access worker, then the UI/service worker. Account acknowledgements
are response supersets and clients accept both `CREDENTIAL_REVOKED` and legacy
`DEVICE_REVOKED` during the mixed-version window.

The preflight route is a hard dependency of this account UI: deploy the account
worker containing `/accounts/preflight` before the UI that calls it. The UI
fails closed if the route is absent rather than creating a passkey before email
availability has been verified.

Releases are not manual. `.github/workflows/publish.yml` applies this
crate's migrations and deploys the worker on every push to `staging` and
`stable`, alongside the access worker, so the two never drift apart. The
steps below are the one-time bootstrap of the resources that deploy needs —
database, buckets, secrets, and WAF rules — not a release procedure.

First deploy, in order:

1. `wrangler d1 create tonk-accounts` and paste the returned database id into
   `database_id` in `wrangler.account.toml` (it ships empty on purpose —
   wrangler refuses to deploy without it, so there's no risk of silently
   deploying against the wrong database).
2. `wrangler d1 migrations apply tonk-accounts --remote -c wrangler.account.toml`
   to apply every pending migration, including
   `0003_account_repository_descriptor.sql` and `0004_normalize_devices.sql`.
3. `wrangler r2 bucket create tonk-account-chains`.
4. `wrangler r2 bucket create tonk-revocations`.
5. `wrangler secret put RESEND_API_KEY -c wrangler.account.toml`.
6. Add the DKIM records Resend requires for sending from `tonk.xyz` in the
   Cloudflare dashboard (DNS for the zone), so `accounts@tonk.xyz` mail is
   authenticated.
7. Add a WAF rate limiting rule for unauthenticated request creation (zone `tonk.xyz`,
   Security > WAF > Rate limiting rules): expression
   `(http.host eq "accounts.tonk.xyz" and http.request.method eq "POST" and
   http.request.uri.path in {"/codes" "/accounts/preflight" "/links"})`,
   counting by IP, 3 requests per 10 seconds, action Block. The service
   enforces a per-email cooldown but nothing per-IP, so without this rule an
   unauthenticated caller can fan out email sends or pending D1 rows.
8. `wrangler deploy -c wrangler.account.toml`.

### Staging

Staging is a wrangler environment in the same config, on `accounts-staging.tonk.network`
with its own database and bucket. Deploy it the same way, in order:

1. `wrangler d1 create tonk-accounts-staging` and paste the returned id into
   `database_id` under `[[env.staging.d1_databases]]`.
2. `wrangler d1 migrations apply tonk-accounts-staging --remote -c wrangler.account.toml --env staging`.
3. `wrangler r2 bucket create tonk-account-chains-staging`.
4. `wrangler r2 bucket create tonk-revocations-staging`.
5. `wrangler secret put RESEND_API_KEY -c wrangler.account.toml --env staging`.
   Secrets are per-environment, so this has to be set explicitly — with the
   same value as production's key, since staging sends from the same verified
   domain.
6. Add the WAF rate limiting rule for `accounts-staging.tonk.network`, matching
   the production rule above. Note the zone differs: staging is on the
   `tonk.network` zone, so the rule has to be created there rather than
   alongside production's on `tonk.xyz`. Staging shares the production Resend
   key, so it shares the email fan-out vector.
7. `wrangler deploy -c wrangler.account.toml --env staging`.

There is no DNS step: the route is a Custom Domain (`custom_domain = true`),
so wrangler provisions the record and certificate itself on deploy. Creating
`accounts-staging.tonk.network` by hand beforehand conflicts with that and blocks
the deploy.

Exercise the flow against staging by creating an account on
`https://staging.tonk.network/account`—the page reads the staging provider from
`GET /.well-known/tonk`—then linking the CLI:

```
tonk account link \
  --service-url https://accounts-staging.tonk.network \
  --account-url https://staging.tonk.network/account/link
```

### Preview

Preview is a third wrangler environment, on no route at all: `.github/workflows/publish.yml`
uploads one version of it per pull request under a `pr-<number>` preview alias
and reads back the workers.dev URL that alias resolves to. The access worker's
`ACCOUNT_SERVICE_URL` is then overridden with that URL, so a pull request's
browser reaches the same pull request's registry rather than a released one.

The environment exists so continuous integration can apply an unreviewed
migration to a real D1 database. That is exactly what must never happen to
staging, and it is why preview holds its own database and buckets rather than
borrowing staging's. Resetting or recreating any of them is safe, and is the
right first move whenever two open pull requests have left the schema in a
state neither of them expects.

Bootstrap covers both workers, because a preview is only testable as a pair.
Do it once, in order; after this, pull requests need no manual step.

State:

1. `wrangler d1 create tonk-accounts-preview` and paste the returned id into
   `database_id` under `[[env.preview.d1_databases]]`.
2. `wrangler r2 bucket create tonk-account-chains-preview`.
3. `wrangler r2 bucket create tonk-revocations-preview`. Both workers bind this
   one: this service publishes revocation artifacts into it and the access
   service reads them back, so a preview that split them would enforce nothing.
4. `wrangler r2 bucket create tonk-spaces-preview`, bound by the access service
   as `BUCKET`.

Secrets, which are per-environment and so have to be set explicitly:

5. `wrangler secret put RESEND_API_KEY -c wrangler.account.toml --env preview`.
   Preview sends from the same verified domain as staging and production, so it
   inherits the same email fan-out exposure and wants the matching rate limiting
   rule.
6. `wrangler secret put R2_ACCESS_KEY_ID -c wrangler.toml --env preview` and
   `wrangler secret put R2_SECRET_ACCESS_KEY -c wrangler.toml --env preview`,
   from an R2 API token granting object read and write on `tonk-spaces-preview`.
   That is the only bucket these credentials presign; `BUCKET` and `REVOCATIONS`
   are native bindings and need none.

Workers. `wrangler versions upload` versions an existing Worker and fails when
there is none, so each needs one deploy by hand before the workflow has anything
to attach its uploads to:

7. `wrangler deploy -c wrangler.account.toml --env preview`.
8. `wrangler deploy -c wrangler.toml --env preview`.

The API token the workflow authenticates with needs D1 edit permission, not just
Workers deployment. Without it the run fails at the migration step, before
either upload.

## Abuse controls

Application-level throttles live in `src/core/codes.rs`: per-email 60 s
resend cooldown, 10-minute code TTL, five verification attempts per code.
Everything IP-shaped is enforced at the Cloudflare edge, not in code:

- **Rate rule** (zone `tonk.xyz`, and the staging host): one rate-limiting
  rule per environment covering the unauthenticated and code-authenticated
  bootstrap paths. Each rule's expression should match `/codes`,
  `/accounts/preflight`, and `/links`:

  ```
  (http.host eq "accounts.tonk.xyz" and http.request.method eq "POST" and
   http.request.uri.path in {"/codes" "/accounts/preflight" "/links"})
  ```

  Deployed threshold: 3 requests per 10 seconds per IP, action Block.
  Verify each environment's existing rule covers all three paths in the
  expression.
  `/links/resolve|complete|consume` need no rule: they demand the 256-bit
  bearer secret and cheap lookups fail closed.
- **Turnstile**: deliberately not deployed. Revisit only if the rate rule
  proves insufficient in practice.

### Deploy verification

Migrations must be applied to both environments (wrangler reads
`wrangler.account.toml`):

```sh
wrangler d1 migrations list tonk-accounts --remote -c wrangler.account.toml
wrangler d1 migrations list tonk-accounts-staging --remote -c wrangler.account.toml --env staging
```

Preview is not on this list: its migrations are applied by the deploy workflow
on every pull request, so its schema is whatever the branch under review says it
is, and drift there is the expected state rather than a fault.

Both must show `0001_init.sql`, `0002_link_requests.sql`,
`0003_device_delegation_path.sql`, `0004_account_repository_descriptor.sql`,
`0005_normalize_devices.sql`, and `0006_device_attachment_lifecycle.sql` as applied;
apply any pending ones with the matching `d1 migrations apply` command.
Migration 0003 deliberately leaves the
new column null for legacy devices because a delegation CID cannot reconstruct
its signed path bytes. Those rows remain visible and may self-revoke, but a
different device cannot revoke them through the account UI until they relink.
Confirm the rate rule exists in the Cloudflare dashboard (Security →
WAF → Rate limiting rules) and that its path list includes `/links`.

### Non-destructive staging smoke

Treat staging as shared infrastructure. Before changing it, establish who owns
the run and record the exact account, access, and UI commits being deployed,
the applied D1 migrations, and the bound R2 buckets. A healthy `/health`
response alone is not deployment provenance.

Deploy serially: account worker first, access worker second, UI/service worker
last. Use fresh browser storage and disposable account emails. Exercise
root creation, restore, cross-device and self-revocation, open and targeted
invites, guest promotion, and typed manual-sync failure. Confirm immutable R2
revocation enforcement while deliberately leaving the D1 projection stale.

Never reset or replace `tonk-spaces-staging` for this smoke. Only test-created
account rows and revocation objects may be removed, and only by the operator
who owns the run when schema compatibility requires it. If ownership, deployed
commit provenance, or the live schema is ambiguous, stop: results from a
mixed-version environment are not E2E evidence.
