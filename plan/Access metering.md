# Access Service: Metering, Rate Limiting, and Billing

Status: draft for implementation
Scope: changes to the existing Cloudflare Worker access service, plus new supporting components

## 1. System today

A stateless Worker receives an invocation naming a consumer space, verifies a UCAN delegation chain, and returns a presigned R2 URL for a single GET or PUT against `/space/<did:key>/<block>`.

Authorization is per block: one permit, one R2 operation. Presigning stays. Proxying block traffic through the Worker is out of scope.

## 2. Model

**Customer** is a billable party. It has an email address, a plan, and Stripe billing once payment is set up. It is identified by a DID derived from a passkey via the WebAuthn PRF extension.

**Consumer** is a space that this service replicates. A customer's own account space is a consumer like any other, identified by the same DID as the customer.

**Provider** is the customer responsible for a consumer. Exactly one, required: a consumer without a provider is not servable. The provider draws on their own remaining credit limit and pays whatever the pledge pool does not cover.

**Sponsor** is a customer who pledges a fixed number of credits per funding cycle to a consumer they do not provide. Zero or more per consumer. At the opening of each funding cycle every pledge is withheld from its sponsor's limit and pooled for the consumer; the consumer draws on the pool before it touches the provider. A sponsor is billed for what the pool actually used, in proportion to pledge, and whatever remains unused at the close of the cycle is released back the same way.

The **funding cycle** of a consumer is its provider's billing period. Sponsors keep their own billing periods for their own invoices; only the pool's lifetime is anchored to the provider. A provider change closes the funding cycle and opens a new one, re-drawing every pledge.

Only paid plans may sponsor. A free plan may provide consumers and draws them all from one limit, so additional free accounts and additional free spaces add no capacity.

**Plan** carries the rates and the credit limit. Plan rows are immutable: a repricing creates a new row rather than mutating one, so a ledger entry naming a plan fully determines how it was computed.

Provisioning is state, not a credential. A delegation chain roots at the consumer DID, which is self-certifying, so it proves present authority over the space but nothing about whether the service agreed to serve it. The `consumer` row is that agreement, held as a looked-up row rather than a token because it must be revocable.

Billing authority and access authority are separate. Who pays for a consumer is the provider and sponsor relationship; who may read, write, or delete it comes from the delegation chain.

## 3. Registration

Three invocations. All are verified by the access service; none bypass the chain. Each roots at the invoking customer's DID, which is self-certifying, so the standard chain check proves the issuer holds the customer's authority and no delegation from the service is involved. The one service-rooted invocation is activation, which the service issues to itself (section 3.2).

### 3.1 Enroll

The client holds an account keypair and its account space. It issues a delegation to `did:web:tonk.network` granting `/archive` and `/memory` on `cell: /branch/account` of the account DID, then invokes:

```
{ cmd: "/customer/enroll",
  sub: "did:key:zAlice",
  args: { email: "alice@example.com",
          access: { "/": "bai..." } } }
```

The chain roots at the customer DID: signed by the account key itself, or by a device key carrying a delegation from it. `access` names the deposited delegation by CID; its bytes travel in the same container. It is an argument, not a proof: it does not extend the invocation's chain, and the service verifies it separately as its own chain, issued under the customer's authority with the service as audience.

The service verifies both and writes, in one batch:

- `customer` row, `status = Registered`, `verified = 0`
- `consumer` row for the same DID, `registered = now`
- `consumer.provider` set to that same DID, so the customer provides its own account space

KV gets the derived state, which denies service at this point. The service then emails a link carrying a single-use token.

Writing both rows together matters. Two steps would leave a window in which a consumer exists with no provider, which is not servable.

### 3.2 Activate

At enroll the service signs an activation invocation and emails a link carrying it, base64url encoded in a query parameter:

```
{ cmd: "/customer/activate",
  sub: "did:web:tonk.network",
  args: { customer: "did:key:zAlice" },
  exp: <enroll + EMAIL_TOKEN_TTL> }
```

The invocation is self-signed by the service, so it carries no proof chain and stays small enough for a URL. `exp` travels inside it, so an expired link fails verification without a storage lookup.

Replay is harmless: activating an already-active customer is a no-op.

Clicking presents the invocation. The service verifies it exactly as it verifies any other, then executes: `customer.verified` is set, `status` becomes `Active`, and KV is rewritten for every consumer this customer funds. Replication on the account space is live.

The link is a bearer credential in a URL, so it reaches browser history, referrer headers, and intermediaries. Same exposure as any magic link, which argues for a short `EMAIL_TOKEN_TTL`. The prize is weak: the invocation names one customer and confers verification of an email the service already chose, so an interceptor gains an activated account whose keys they do not hold.

Mail-client prefetching can fire the link without a human click. That makes the confirmation meaningless rather than dangerous. To prevent it, render a confirm button on GET and execute on POST.

**Waiting client.** The click often lands on a different device than the one waiting, so the activation is authoritative in D1 and the enrolling device learns of it separately. A held HTTP request is ruled out by Cloudflare's 100 to 120 second proxy read timeout on response headers. Open decision 2.

### 3.3 Add a consumer

Enrolling a further space needs consent from both sides. The consumer delegates `/provider/add` to the customer; the audience is what names the provider it accepts:

```
{ cmd: "/provider/add",
  sub: "did:key:zPhotos",
  aud: "did:key:zAlice" }
```

The customer then invokes, carrying that delegation as consent:

```
{ cmd: "/consumer/add",
  sub: "did:key:zAlice",
  args: { consumer: "did:key:zPhotos",
          consent: { "/": "bai..." } } }
```

The provider being added is the invocation's subject; it needs no argument of its own. The invocation chain is the customer's consent. The enclosed delegation is the consumer's. Neither party is enrolled unilaterally. In practice the client already holds a powerline delegation from the space to the account, which satisfies `/provider/add` as-is.

The service validates the consent as a chain of its own: it must root at the consumer being added, its audience must be the invoking customer, and it must grant `/provider/add` or broader. Audience is what binds it, so a consent given to one customer cannot be used to enrol a different one.

The service checks the customer is `Registered` or `Active`, writes a `consumer` row with `provider` set to that customer, and writes KV. A consumer has exactly one provider, so this fails if one is already set.

Activation is not required to add, only to serve. Servability is derived state, and a consumer whose provider is `Registered` derives to denied, exactly as the customer's own account consumer does between enroll and the email click. When the customer activates, the rewrite pass in section 3.2 already covers every consumer they fund, so consumers added before activation go live in the same stroke. Requiring `Active` here would buy nothing the derivation does not already enforce, and would push a queue of deferred invocations into the client for spaces created before the email arrives.

### 3.4 Sponsor a consumer

A customer on a plan with `may_sponsor` pledges a fixed number of credits per cycle to a consumer they do not provide:

```
{ cmd: "/consumer/pledge",
  sub: "did:key:zBob",
  args: { consumer: "did:key:zPhotos",
          pledge: 3000 } }
```

The sponsor is the invocation's subject, same shape as `/consumer/add`. The service checks the sponsor is `Active` and their plan permits sponsoring, that the pledge plus the undrawn remainder of their existing pledges plus their current period usage stays within their limit, and that the consumer has a provider. The undrawn remainder rather than the full pledge: settled shares already sit in the sponsor's usage, and counting the whole pledge would count them twice. It writes a `sponsorship` row with `effective` set to the consumer's next funding cycle.

Withdrawal sets `ends` to the current funding cycle. Both take effect at the next boundary.

## 4. What is measurable

Properties of the current system. Inputs to the design, not decisions taken here.

| Property | Consequence |
|---|---|
| Presigned GETs bypass the Worker and R2 publishes no read access log | Reads can be counted at authorization and nowhere else |
| Tree references carry no block sizes | Read metering is by operation count. R2 charges by count too, so this tracks cost |
| Declared size is bound into the URL as a signed `Content-Length` | Write bytes are exact and enforced by R2 |
| Blocks are namespaced per consumer, duplicated across consumers | No shared-block attribution problem |
| Archiving a consumer deletes its data | Storage is not monotonic, so it must accrue per run rather than be measured once at period close |
| Block reads are content addressed and client cached | Permits stay short lived; replay is bounded and low value |
| The branch revision pointer is mutable and polled | Its permit must be long lived, so one authorization covers unbounded reads. Open decision 5 |
| Invocations are signed by the client and content addressed | The bill can be evidenced by artifacts the service could not have forged |

## 5. Billing units

| Unit | Source | Accuracy |
|---|---|---|
| Read operations | Read permits issued, per consumer | Exact modulo replay and unused permits |
| Write operations | Write permits issued, per consumer | Same |
| Write bytes | Signed `Content-Length`, enforced by R2 | Exact |
| Storage | GB-hours accrued per run from R2 prefix measurements | Bounded by sampling cadence |
| Compute | Open decision 4 | Fitted approximation if billed at all |

Read bytes are excluded: sizes are unknown at read time, R2 charges by operation count, and egress is free.

Metering is authorization-based, not delivery-based. An issued permit is billed whether or not the client uses it. This is by design and must be stated in customer-facing pricing.

Denied invocations are recorded with `outcome = 'denied'`, since a client retrying against a blocked consumer still costs invocations. Whether denials are billed is open decision 12; the data supports either.

## 6. Architecture

Two D1 databases, deliberately separate.

**Ingest** holds one row per invocation, with the invocation bytes inline. Bulky, write-heavy, disposable once charged and archived. Splitting it out gives it its own 10 GB ceiling and isolates schema churn from billing state.

**Control** holds customers, consumers, sponsorships, plans, runs, and the ledger. Small, transactional, permanent.

KV holds one derived state value per consumer, read on the hot path. R2 holds the archived evidence. Analytics Engine carries a parallel per-invocation stream for dashboards and calibration, not for billing.

## 7. Hot path

Per invocation:

1. Read consumer state from the isolate cache. On miss, read KV. On KV miss, read control D1, derive, write back. On KV error, default to serving and alert.
2. Verify the delegation chain.
3. If state permits, issue the presigned URL. Otherwise return the appropriate status.
4. Inside `ctx.waitUntil`, insert one `invocation` row into ingest, plus `chain` and `block` rows if this chain has not been seen.

The insert is durable when it returns. D1 bills per row regardless of grouping, so batching saves nothing that a durable buffer does not cost back.

A KV miss is not an answer. KV is eventually consistent, so a freshly enrolled consumer would be rejected until propagation if a miss were treated as absence. Negative results are cached under `NEGATIVE_CACHE_TTL` so traffic against nonexistent consumers cannot become unbounded D1 reads.

## 8. Ingest schema

```sql
-- No secondary indexes: each one adds a written row per insert.
CREATE TABLE invocation (
  id       INTEGER PRIMARY KEY,   -- cursor within this database
  ts       INTEGER NOT NULL,
  cid      TEXT    NOT NULL,      -- evidence key
  consumer TEXT    NOT NULL,
  issuer   TEXT    NOT NULL,
  cmd      TEXT    NOT NULL,
  outcome  TEXT    NOT NULL,      -- ok | denied
  reason   TEXT,
  bytes    INTEGER NOT NULL DEFAULT 0,
  compute  INTEGER NOT NULL DEFAULT 0,
  chain    TEXT    NOT NULL,      -- CID of the proof set
  body     BLOB    NOT NULL       -- invocation bytes, proofs by reference
);

CREATE TABLE chain (
  chain TEXT NOT NULL,
  proof TEXT NOT NULL,
  PRIMARY KEY (chain, proof)
);

CREATE TABLE block (
  cid  TEXT PRIMARY KEY,
  body BLOB NOT NULL
);
```

`chain` is the transitive proof set, flattened at write time so evidence retrieval is two queries rather than a recursive walk. It is written once per unique operator session and referenced by every invocation in it. Use `INSERT OR IGNORE` on `block` and `chain`, and verify against `meta.rows_written` whether an ignored conflict is billed.

Delegations already name their own proofs internally, so the flattened set caches something derivable. It exists because writes happen every request and retrieval happens on dispute.

## 9. Control schema

```sql
CREATE TABLE plan (
  id              TEXT PRIMARY KEY,   -- 'pro@2026-08', immutable
  name            TEXT NOT NULL,
  credit_limit    INTEGER NOT NULL,
  may_sponsor     INTEGER NOT NULL DEFAULT 0,
  read_rate       INTEGER NOT NULL,   -- credits per operation
  write_rate      INTEGER NOT NULL,
  write_byte_rate INTEGER NOT NULL,
  storage_rate    INTEGER NOT NULL,   -- credits per GB per cycle
  compute_rate    INTEGER NOT NULL,
  stripe_price    TEXT                -- null on a free plan
);

CREATE TABLE customer (
  did             TEXT PRIMARY KEY,   -- also the DID of its account consumer
  email           TEXT NOT NULL,
  verified        INTEGER NOT NULL DEFAULT 0,
  status          TEXT NOT NULL,      -- Registered | Active | Suspended
  plan            TEXT NOT NULL REFERENCES plan(id),
  credit_limit    INTEGER,            -- override, null means use plan
  cycle_anchor    INTEGER NOT NULL,   -- subscription day, periods derive from it
  limit_code      TEXT,               -- null when under limit
  limit_resets    INTEGER,            -- null with code set: cleared by event
  stripe_customer TEXT                -- null until payment is set up
);

CREATE TABLE consumer (
  did             TEXT PRIMARY KEY,
  provider        TEXT REFERENCES customer(did),   -- null means not servable
  registered      INTEGER NOT NULL,
  archived_at     INTEGER,
  suspend_code    TEXT,
  suspend_message TEXT,
  suspend_until   INTEGER,            -- null with code set: indefinite
  size            INTEGER NOT NULL DEFAULT 0,   -- last measurement
  measured_at     INTEGER NOT NULL DEFAULT 0
);

-- Fixed for the funding cycle. Adding or removing a sponsorship takes effect
-- at the next funding-cycle boundary, so the pool does not move mid-cycle.
CREATE TABLE sponsorship (
  consumer  TEXT    NOT NULL REFERENCES consumer(did),
  customer  TEXT    NOT NULL REFERENCES customer(did),
  pledge    INTEGER NOT NULL,   -- credits per funding cycle
  effective TEXT    NOT NULL,   -- first funding cycle this applies to
  ends      TEXT,               -- last funding cycle, null while open
  PRIMARY KEY (consumer, customer)
);

-- Storage accrues per run and converts at period close.
CREATE TABLE accrual (
  period   TEXT    NOT NULL,
  consumer TEXT    NOT NULL REFERENCES consumer(did),
  gb_hours INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (period, consumer)
);

-- Running totals per customer per period. Enforcement reads this rather than
-- scanning ledger rows, which grow through the period.
CREATE TABLE usage (
  period   TEXT    NOT NULL,
  customer TEXT    NOT NULL REFERENCES customer(did),
  credits  INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (period, customer)
);

-- Each sponsor's settled share of the pool this funding cycle. The pool's
-- remainder is Σ pledge − Σ drawn, so exhaustion is one per-consumer fact.
-- Keyed by funding cycle, not the payer's period: those are different clocks.
CREATE TABLE drawn (
  cycle    TEXT    NOT NULL,
  consumer TEXT    NOT NULL,
  customer TEXT    NOT NULL,
  credits  INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (cycle, consumer, customer)
);

CREATE TABLE run (
  id            TEXT PRIMARY KEY,   -- Stripe idempotency key
  started       INTEGER NOT NULL,
  ingest        TEXT    NOT NULL,
  charged_upto  INTEGER NOT NULL,   -- max invocation.id consumed
  archived_upto INTEGER NOT NULL DEFAULT 0,
  pushed        INTEGER
);

CREATE TABLE ledger (
  id       INTEGER PRIMARY KEY,
  run      TEXT    NOT NULL REFERENCES run(id),
  ts       INTEGER NOT NULL,
  period   TEXT    NOT NULL,       -- payer's billing period at charge time
  consumer TEXT    NOT NULL,
  customer TEXT    NOT NULL,       -- payer
  role     TEXT    NOT NULL,       -- provider | sponsor
  kind     TEXT    NOT NULL,       -- usage | storage
  reads    INTEGER NOT NULL DEFAULT 0,
  writes   INTEGER NOT NULL DEFAULT 0,
  bytes    INTEGER NOT NULL DEFAULT 0,
  compute  INTEGER NOT NULL DEFAULT 0,
  storage  INTEGER NOT NULL DEFAULT 0,
  credits  INTEGER NOT NULL,
  plan     TEXT    NOT NULL REFERENCES plan(id)
);
```

Notes on shape.

`ledger` carries operation counts, not only credits, so a payer can be shown exactly which share they carried and the shares sum to the consumer's total for that run.

`ledger.plan` is the plan that priced the row: the provider's plan at charge time, since the consumer's usage converts at one set of rates (section 10.1) and sponsor shares transfer those credits. Plans imply rates, so a historical row must not be reinterpreted after an upgrade or a repricing. Recording it per row also means an upgrade needs no backfill.

`ledger.period` is the payer's billing period, computed at charge time from their `cycle_anchor`. Runs land on a cron cadence; periods are per customer and anchored to their subscription date. They do not align, so the period travels on the row. An invoice is `SUM(credits) WHERE customer = ? AND period = ?`, and a cycle rolls by the period key advancing, so history survives.

`usage` is the same sum, materialised, and must reconcile against the ledger. A cache that can drift silently from an auditable source is worse than no cache.

A customer's available credit is `credit_limit` minus `usage.credits` for the current period minus the undrawn remainder of every pledge they sponsor: `Σ max(pledge − drawn, 0)` over their open sponsorships. The remainder rather than the full pledge, because settled shares are already in `usage.credits` and subtracting the whole pledge would count them twice. The full pledge is thus withheld the moment a funding cycle opens and returns as it settles, with whatever never settles releasing at the close.

Credits are integers. Choose the denomination so the cheapest billable operation is a comfortable whole number, and record what a credit is worth somewhere durable.

## 10. Charging

One cron. Each execution is a run, named before any work starts, so every derived row is answerable to whether it already happened.

### 10.1 Usage

Read aggregates from ingest above the cursor. This is a read, so it is idempotent and repeatable:

```sql
SELECT consumer, cmd, outcome,
       COUNT(*) AS ops, SUM(bytes) AS bytes, SUM(compute) AS compute
  FROM invocation
 WHERE id > :charged_upto
 GROUP BY consumer, cmd, outcome;
```

Convert each consumer's aggregate to credits at the provider's plan rates, then allocate. One conversion per consumer: the pool is a single number in credits, so per-payer rates would make its arithmetic incoherent. The provider's plan is the consumer's service terms; sponsor shares are transfers of those credits, not repricings.

**Pool first.** Load the consumer's sponsorships effective for its current funding cycle. The pool is `Σ pledge`; what remains is `Σ pledge − Σ drawn`. Draw this run's credits from the remaining pool and split the draw across sponsors in exact proportion to pledge, largest remainder, so the shares sum exactly to the draw:

```
share_i = draw × pledge_i / Σ pledge    (largest remainder)
```

Proportions are fixed by the pledges, so every sponsor's `drawn` reaches its `pledge` at the same moment the pool empties. There is no per-sponsor exhaustion order and no cap to water-fill against.

**Provider last.** Whatever exceeds the remaining pool goes to the provider, drawn from their remaining limit.

Write one `ledger` row per payer with `role` and `kind = 'usage'`, increment `drawn` for each sponsor, increment `usage.credits` for each payer, and write `run.charged_upto`, all in the same `db.batch()`. D1 offers no interactive transactions, so the batch is the atomicity. A failed batch leaves the cursor unmoved and the rerun recomputes from the same read.

The cross-database boundary sits on the read side deliberately: aggregates come from ingest, every write lands in control, so the atomicity that matters is available.

Each sponsor's ledger row lands in that sponsor's own current period, computed from their `cycle_anchor` at charge time as always. The funding cycle governs the pool; the payer's period governs their invoice. The two clocks never need to agree.

Unused pool at the close of a funding cycle is simply never settled: each sponsor's reservation ends with the cycle, and the undrawn remainder, proportional to pledge by construction, returns to their available credit. Release is the absence of a charge, so it writes no ledger row.

### 10.2 Storage

Storage is a rent, accrued every run and converted at period close.

Each run measures the consumer prefix from R2 bucket metrics, adds `size × hours since measured_at` to `accrual.gb_hours`, and updates `consumer.size` and `measured_at`. Sampling cadence sets the resolution: at hourly runs the error is bounded by an hour of growth.

Accruing per run is what makes archival work. Archiving a consumer deletes its data, so the next measurement reads zero. The hours before it are already accrued, so archival needs no special handling. A single end-of-period reading would bill nothing for a consumer archived on day 20 of 30, which is both wrong and an incentive to archive just before the boundary.

At period close, convert `accrual.gb_hours` to credits and allocate by the same pool-then-provider rule as usage, writing `kind = 'storage'` rows.

Order within a cycle-end run: charge usage, charge storage, then push to Stripe. Pushing first omits the storage line.

### 10.3 Exhaustion

When the pool is exhausted, the provider carries the whole cost. When the provider is also at their limit, the consumer's derived state becomes limited and subsequent invocations are denied and recorded with `outcome = 'denied'` until the next funding cycle opens, resetting `drawn` and withholding every pledge afresh. Attribution continues: charges still land on the provider, so overage is visible as `usage.credits` exceeding `credit_limit` rather than accumulating as an unattributed balance.

Sponsorships are fixed for the funding cycle. Adding or removing one takes effect at the next boundary, so the pool and its proportions are stable within a cycle and no usage has to be split into before-and-after segments. A sponsor who withdraws mid-cycle remains a payer for that cycle: their pledge stays in the pool and settles in proportion as usual.

### 10.4 Archive

Write `invocation`, `chain`, and `block` rows to R2 under `{consumer}/{period}/{cid}`. Invocations and proofs share one keyspace there, so a CID resolves to bytes uniformly. Content addressing makes the write idempotent, so a partial run resumes rather than restarts. Advance `run.archived_upto`.

The key omits the customer deliberately. Attribution is not known at write time, since an invocation may be charged to a sponsor or to the provider depending on what remains at charge time. A customer component would also duplicate objects per payer or leak the sponsor set through the key.

Evidence is written by the cron rather than by the request, so it is amortised and the hot path does not depend on R2 write availability.

The point of keeping it: the invocation is signed by the customer, so a bill can be evidenced by artifacts the service could not have forged. It proves nothing was invented; it does not prove nothing was omitted, which errs in the customer's favour.

Retrieval is a prefix list plus a fetch per object. Listing pages at 1,000 keys and each page is a Class A operation, so `ledger` remains the source for rendering a bill and R2 is for the evidence behind a specific line.

### 10.5 Prune

Delete charged and archived rows from ingest by id range. `DELETE` counts as a write in D1, so pruning costs the same per row as inserting, doubling the effective per-invocation D1 cost. At 50 million included writes a month, the effective ceiling is 25 million invocations before this costs anything.

The alternative is rotating ingest databases and dropping rather than deleting, which is free if `DROP TABLE` is free. Cloudflare declines to say: the pricing page states only that DDL may contribute to a mix of read and write rows. Measure before building it.

Keep the option cheap by treating the cursor as `(database, id)` from the start and never referencing row ids across periods. Then switching later is a change to the cron, not a migration.

### 10.6 Stripe

Push `usage.credits` for the closed period, one call per customer. Skip customers with no `stripe_customer`, which is the normal state before payment is set up. Identifier `{customer}:{period}`. Pushing the counter rather than re-summing the ledger keeps one definition of an invoice. The dedup window is rolling 24 hours only, so retries or backfill older than a day double-bill.

Do not send consumer as a dimension; dimensions cap near 100 unique combinations per customer per meter. Per-consumer detail stays in `ledger`.

The API path is sufficient at any plausible volume: one call per customer per run against a 1,000 per second limit. The S3 connector exists for high-throughput ingestion, requires an AWS account ID and an IAM role for Stripe to assume, and therefore cannot be pointed at R2.

Failures arrive as webhook events rather than as failed calls, so a handler is needed regardless of transport: `meter_event_customer_not_found`, `timestamp_too_far_in_past`, `timestamp_in_future`, `meter_event_invalid_value`.

Webhooks update `customer.limit_code` and `status`, then write affected consumer states to KV, idempotent on the Stripe event ID.

## 11. Enforcement

Four axes are stored, and the value KV serves is derived from them.

```
provider:   did?                           on consumer, null means unserved
archived:   timestamp?                     on consumer
suspension: { code, message?, until? }?    on consumer
funding:    { code, resets? }?             on the provider's customer row
```

Precedence: unprovided, archived, suspended, limited, ok. Unregistered is absence of the consumer row.

Funding is limited when the provider is at their limit and the consumer's pool for the current funding cycle is exhausted. The pool's position comes from `sponsorship` and `drawn`; the provider's from `usage.credits` against their `credit_limit`.

Storing them separately is what makes unblocking free: clear the suspension, re-derive, and it lands on limited or ok according to funding, which was never touched.

`until` and `resets` are absolute epoch timestamps, and null means indefinite rather than a sentinel far-future number. Null forces the branch to be written; a large number silently succeeds every comparison, and zero is worse still since it is a plausible uninitialised value that would read as permanent.

Null `resets` is the out-of-credit case, cleared by the Stripe webhook rather than by a clock. If that webhook is missed nothing else lifts it, so the cron needs a reconciliation pass rather than trusting the webhook alone.

Codes are stable identifiers the client switches on. Messages are optional prose. A code alone cannot be specific; a message alone cannot be localised or keyed off.

### 11.1 The KV value

The value is a versioned struct, not a bare string, since a stale isolate can hold a shape written by a previous deploy.

Every value carries `not_after`, an absolute timestamp after which the reader must revalidate. Absolute rather than relative, because the isolate copy ages after it is read.

Staleness is asymmetric: a stale permit costs a handful of operations, a stale denial costs a paying customer service and generates a support ticket. So `not_after` is set per variant, generous on ok and short on anything denying service, with jitter so expiry does not cluster.

Where a limit genuinely resets on a clock, the value also carries `resets`, which becomes `Retry-After` on the response. That is client-facing information, not a cache control input.

### 11.2 Writers

Three writers, plus deletion.

The cron writes on recompute. The hot path writes back on a miss. The Stripe webhook writes on state change. Deprovisioning deletes the key rather than writing a negative value, forcing the next request through authoritative D1.

KV has no compare-and-swap, so writes are last-write-wins. A miss-path backfill that loses its scheduling slice can overwrite a fresher value written by the cron, producing a stale value with fresh-looking validity, which is worse than a stale value that admits it. So backfill writes get a deliberately short `not_after`, on the order of the cron interval. A losing race then self-corrects on the next run.

Rule: only the cron may write long validity.

The cron writes changed consumers, not all of them. `not_after` must therefore be set well beyond the cron interval, so stable consumers do not expire together and produce a synchronised D1 read storm on the miss path.

### 11.3 Resolution order

1. Isolate cache.
2. KV.
3. On KV miss, read `consumer`, its sponsorships and their `drawn` totals, and the provider's `customer`, `plan`, and `usage` rows from control D1, derive, write back with short validity.
4. On KV error, default to serving and alert.

Miss versus error is `null` versus thrown.

### 11.4 Rate limiting

Workers `ratelimit` binding, free and per-colo, on `issuer` and `consumer` namespaces, plus `RATELIMIT_REGISTER` on `/customer/enroll`, `/consumer/add`, and `/consumer/pledge`. The consumer namespace also bounds nonexistent-consumer traffic ahead of the state lookup.

Counts are unweighted, which is valid only while authorization stays per block.

## 12. Credit conversion

Do not fix the rates before data exists. Anchor on marginal cost: a read permit is one Worker request plus one R2 Class B operation; a write permit is one Worker request plus one Class A, materially more expensive; storage is per GB-month.

Set price per credit as the actual monthly Cloudflare bill divided by total credits charged, refit monthly. This recovers cost without maintaining a per-instruction schedule and stays correct when the traffic mix shifts, which matters because a change in polling behaviour would otherwise invalidate a fitted formula.

Publish the ratios, not the cost basis.

## 13. Configuration

| Key | Governs |
|---|---|
| `CHARGE_CRON` | Charge latency and Stripe cadence |
| `STATE_CACHE_TTL_MS` | Isolate cache lifetime |
| `NOT_AFTER_OK` / `NOT_AFTER_DENY` / `NOT_AFTER_BACKFILL` | Per-variant KV validity |
| `NEGATIVE_CACHE_TTL` | How long an unregistered result is cached |
| `INGEST_RETENTION` | How long charged rows stay in ingest before pruning |
| `RATELIMIT_ISSUER` / `RATELIMIT_CONSUMER` / `RATELIMIT_REGISTER` | Limit and period per namespace |
| `PERMIT_TTL_READ` / `_WRITE` / `_REVISION` | Replay window |
| `CHAIN_DEPTH_MAX` | CPU exposure per request |
| `EMAIL_TOKEN_TTL` | `exp` on the activation invocation, so link lifetime |

## 14. Rollout

| Phase | Contents |
|---|---|
| 0. Meter | Full pipeline, no enforcement, no Stripe. Generous limits. Goal is distributions |
| 1. Calibrate | Fix rates from phase 0. Build the usage display |
| 2. Warn | Enable limit warnings and Stripe reporting. Pipeline errors surface as wrong numbers, not outages |
| 3. Enforce | Enable denial. Tighten limits to observed plus headroom |

Do not compress 0 and 1. The rates are not derivable a priori, and guessing produces a repricing after launch.

## 15. Acceptance criteria

- A permit request on a warm isolate performs no blocking IO beyond chain verification.
- An `invocation` row is durable when the insert returns.
- A charge run failing partway leaves `run.charged_upto` unmoved, and rerunning produces identical ledger rows.
- Ledger rows for one consumer and one run sum, per unit, to that consumer's totals for the run.
- Sponsors pledging 2000, 3000, and 3000 against 5000 credits of usage are billed 1250, 1875, and 1875: proportional to pledge and summing exactly to the usage.
- A run drawing past the end of the pool charges the sponsors exactly the pool's remainder and the provider the excess.
- With the pool exhausted, the provider carries the remainder and ledger rows still cover every operation.
- A pool unused at funding-cycle close leaves each sponsor billed only their settled share, and the undrawn remainder returns to their available credit.
- A new funding cycle opens with the full pool available and every pledge withheld again.
- A consumer with no provider is denied regardless of sponsorships.
- A free plan cannot create a sponsorship.
- A customer cannot pledge more than their limit across all sponsorships.
- Storage shares sum exactly to the consumer's charged storage, with no rounding surplus.
- A consumer archived mid-period is charged for the hours before archival and none after.
- `usage.credits` reconciles against the ledger sum for the period.
- A run crossing a cycle boundary resets the counters rather than accumulating across periods.
- Archived R2 objects reconcile exactly against the ledger rows derived from them.
- A Stripe push retried within 24 hours does not double-bill.
- Enrollment, activation, and consumer addition each produce a verifiable invocation whose chain roots at the invoking customer's DID.
- A consumer added while its provider is `Registered` is denied, and the provider's activation makes it served with no further invocation.
- Clicking an activation link twice leaves the customer active and writes no duplicate state.
- An activation link presented after `EMAIL_TOKEN_TTL` fails chain verification, with no storage lookup involved.
- Enrollment writes the customer and its self-provided consumer atomically, with no intermediate state where one exists without the other.
- A consumer with no `consumer` row is denied, and a valid delegation chain does not change that.
- A consumer enrolled moments earlier is served on first request, before KV has propagated.
- A KV read error is served; a KV miss is not, and falls through to D1.
- Repeated requests against nonexistent consumers do not produce a D1 read per request.
- Clearing a suspension returns the consumer to whatever its funding state implies.
- A customer changing plan mid-period leaves prior ledger rows naming the old plan.

## 16. Open decisions

1. **Free allowance at signup.** Is a newly activated customer with no Stripe setup servable? If yes, `limit_code` starts null and something must set it when the allowance runs out. If no, signup is broken until payment. On the critical path, not a corner case.
2. **Activation notification transport.** The activation invocation itself is resolved (section 3.2), but the enrolling device still needs to learn that the click happened, possibly on another device. A held HTTP request is ruled out by the proxy read timeout. The remaining options are a streamed response, which is server-sent events and bills Durable Object duration for the wait; a hibernating WebSocket, which has no timeout and no duration billing but is the most machinery; or polling, which at two-second intervals over five minutes is about 150 requests and costs effectively nothing. For a once-per-signup event the cost difference rounds to zero, so polling is the smaller thing unless a branch-subscription WebSocket is built for other reasons, in which case activation should reuse that connect-delivers-current-state machinery.
3. **Ingest retention.** `INGEST_RETENTION` is what keeps ingest under the 10 GB cap, which at roughly 400 bytes a row is about 25 million invocations. Too low and disputes have raw detail only in R2. This number is still owed.
4. **Compute metering.** Whether to bill compute at all, and if so how. Gas instrumentation via a `wasm-instrument`-style pass gives a deterministic counter readable without leaving the request, but sees only code inside the module, so confirm Ed25519 verification is in-module rather than WebCrypto, that the tool parses the built artifact, and that the counter survives `wasm-opt` and `wasm-bindgen`. Out-of-band `CPUTimeMs` from Logpush needs no code change but is lossy and late, so it suits calibration rather than billing. Decision 6 may moot both. Wall-clock timing is unavailable: Spectre mitigation makes `performance.now()` advance only after IO, and it works under `wrangler dev`, so the failure is silent.
5. **Branch revision poll metering.** The pointer permit is long lived, so one authorization covers unbounded reads. Options: extrapolate from TTL and an assumed poll interval, unvalidatable; client self-report at renewal, which needs signing or accepted understatement; or serve the pointer from the Worker, exact by construction, with a one to two second cache collapsing N clients to roughly one R2 read. Measure the poll-to-permit ratio in phase 0.
6. **Verification memoization.** A delegation chain is immutable and content addressed, so its verification result is a pure function of the chain CID. Caching by CID for the delegation's remaining lifetime would make repeat polls a lookup. If the hit rate is high, verification stops dominating CPU and decision 4 resolves to not billing compute.
7. **Sponsor visibility.** Can a sponsor see usage detail, or the sponsor set, for a consumer they do not provide? A privacy question when sponsors are different organisations, and it sharpens when evidence is handed over on dispute.
8. **Consent lifetime.** `/provider/add` is audience-bound so it cannot be replayed by a third party, but it survives removal, so a former provider can re-add themselves once the consumer has none. Harmless if the relationship only confers payment. Not harmless if it also confers visibility, which is decision 7 arriving through a side door.
9. **Fail open or closed.** Section 11 serves on KV error and denies on a D1 miss. The asymmetry means a total D1 outage denies everything, since the miss path cannot reach the source of truth.
10. **Unregistered response.** Whether an unregistered consumer returns a distinct status from a limited one, or both collapse, to avoid disclosing which consumers exist.
11. **Storage measurement source.** R2 bucket metrics per prefix is the reliable path but may not exist at prefix granularity. Accumulating write bytes overstates, because content addressing means a duplicate block adds no storage, and it cannot see archival deletions.
12. **Whether to charge for denied invocations.** The data supports either; the pricing page has to say which.
13. **Terms acceptance boundary.** Acceptance should gate something, and which thing changes the flow. Gating registration means enroll writes nothing and the signed invocation carries the pending signup, so an abandoned signup leaves no trace and the link is the only copy. Gating activation means enroll writes rows and acceptance promotes them, which is what section 3 currently describes. Either way the accepted terms version and timestamp must be recorded, and neither is stored today.
14. **Period boundary within a run.** A run crossing a customer's cycle boundary holds invocations belonging to two periods. Splitting the batch by timestamp against each `cycle_anchor` is exact; letting them all land in one period is bounded by the cron interval and self-corrects across periods. Cheap either way, but it should be chosen.

## Appendix A: Rationale

**Why D1 for the write path.** R2 has no append, and one object per invocation costs $4.50 per million against D1's $1.00 per million rows, with the first 50 million included. R2 only wins when many events share an object, which needs a durable buffer, which costs about what the write it defers costs. Queues are worse still at roughly $1.20 per million messages plus consumer invocation, and a queue in front of a volatile buffer does not close the durability hole because the ack happens before the flush.

**Why the invocation body goes in D1 rather than R2.** It rides along in a row already being written, so it is free. Writing evidence to R2 on the hot path would cost $4.50 per million and commit to that before volume is known. Content-addressed objects cannot be batched without losing point lookup, so per-object cost does not amortise.

**Why two databases.** The 10 GB cap is per database. Isolating bulky invocation rows keeps billing state clear of it, separates schema churn, and preserves the option to rotate and drop rather than prune.

**Why aggregation runs in SQL.** Rows read cost about $0.001 per million against $1.00 per million written, a thousand to one, so `GROUP BY` is near free. Pulling raw rows into a Worker incurs the identical read charge and adds serialization, CPU, and a 128 MB ceiling. Only the grouped result crosses the database boundary.

**Why no secondary index on `invocation`.** An index adds a written row per insert touching the indexed column, doubling the expensive dimension to save reads priced a thousand times lower.

**Why the flattened chain table.** Storing proof-to-invocation edges would write one row per proof per invocation, so a five-link chain shared across ten thousand invocations writes fifty thousand rows. The flattened set writes once per session and adds one column per invocation. Storing parent edges instead would recover the delegation graph but require a recursive walk to reconstruct a chain.

**Why provisioning is state rather than a credential.** Provisioning is revocable, so no credential issued at provisioning time can express current standing. Short TTLs with refresh is a lookup on a slower clock; a revocation list is a lookup with extra steps.

**Why Logpush is not a billing transport.** It cannot backfill, logs generated while a job fails are permanently lost, and a 2024 incident lost 55% of logs over 3.5 hours. Correlated unrecoverable loss discovered at invoice time is a different class of failure from small uncorrelated loss.

**Why not Cloudflare Pipelines yet.** Ingress is free and Parquet sinks are $0.06/GB, two orders of magnitude below D1 writes, but D1 includes 50 million writes a month, so the crossover is above the included tier. Pipelines also cannot hold transactional state, so it would displace only the ingest table and leave two data stacks. Iceberg gives time predicates rather than a monotonic cursor, which is fine for dashboards and not fine for anything feeding charging. It is the right answer for the archive and for replacing Analytics Engine once volume justifies it, and billing for it is not yet enabled.

**Why a pooled draw rather than per-sponsor caps.** An earlier draft allocated each run's credits across sponsors with per-pledge caps, which meant water-filling: a sponsor could exhaust before the others, the proportions shifted as caps were hit, and rounding up over-collected. Pooling fixes the proportions for the whole cycle, so shares settle exactly by largest remainder, every pledge empties at the same moment, exhaustion becomes one per-consumer fact the hot path can cache, and release is just the undrawn remainder. It also forces the conversion question into the open: a pool is one number, so usage prices at the provider's plan and sponsor shares transfer credits rather than reprice them.

**Why allocation is resolved at charge time.** A funding delegation rooted at the payer, presented alongside the access chain, would let each permit name its own payer and need no allocation policy. It costs propagation, since a new sponsor means redistributing delegations to every actor, and revocation, and it lets the actor rather than the consumer choose who pays.

**Why deletion rather than revocation of superseded delegations.** Invocation chains travel to the service, but the service verifies and discards rather than storing, and operators are local and networkless. So the only durable copies are on the device. Making revocation meaningful would require the service to maintain and check a revocation list, which is a larger protocol change than issuing revocation records.

## Appendix B: Cost reference

Approximate 2026 rates, for sizing rather than quoting.

| Service | Rate |
|---|---|
| D1 rows written | $1.00/M, first 50M/month included. `INSERT`, `UPDATE`, and `DELETE` all count |
| D1 rows read | ~$0.001/M, first 25B/month included |
| D1 storage | $0.75/GB-month after 5 GB. 10 GB per-database cap |
| D1 DDL | Unspecified: "may contribute to a mix of read rows and write rows" |
| D1 throughput | Roughly 500 to 2,000 writes/s, single writer. No interactive transactions |
| D1 bindings | ~5,000 databases per Worker script; six simultaneous connections per invocation |
| R2 Class A (writes, lists) | $4.50/M, 1M free |
| R2 Class B (reads) | $0.36/M, 10M free |
| R2 storage | $0.015/GB-month, 10 GB free. Objects and bytes per bucket unlimited |
| R2 keys | 1,024 bytes max; one write per second to the same key |
| Worker | $0.30/M requests + $0.02/M CPU-ms |
| Queues | $0.40/M operations, roughly 3 per message |
| Durable Objects | $0.15/M requests; duration $12.50/M GB-s at the full 128 MB, shared across concurrent requests |
| KV | $5.00/M writes, $0.50/M reads, 1M and 10M included |
| Pipelines | Ingress free, transforms $0.04/GB, Parquet sink $0.06/GB. Billing not yet enabled |
| R2 SQL | $2.50/TB scanned. Billing not yet enabled |
| Edge proxy read timeout | 100 to 120s on response headers, not adjustable below Enterprise |

## Appendix C: Glossary

| Term | What it is |
|---|---|
| **Customer** | A billable party, identified by a passkey-derived DID, holding a plan and an email |
| **Consumer** | A space this service replicates. A customer's account space is one, sharing its DID |
| **Provider** | The single customer responsible for a consumer. Required for service |
| **Sponsor** | A customer pledging fixed credits per cycle to a consumer they do not provide |
| **Run** | One execution of the charge cron, and the Stripe idempotency key |
| **Period** | A customer's billing cycle, derived from `cycle_anchor` |
| **Funding cycle** | The lifetime of a consumer's pledge pool, anchored to its provider's billing period |
| **Pool** | The sum of pledges withheld for a consumer at the opening of its funding cycle |
| **Powerline** | A delegation asserting equivalence of authority between two keys |
| **D1** | Serverless SQLite. Bills per row read and written, not per query. 10 GB per database |
| **R2** | S3-compatible object storage. Objects immutable, no append. Bills per operation |
| **KV** | Eventually consistent key-value store, read optimised, globally replicated |
| **Analytics Engine** | High-cardinality sampled time series. Dashboards and calibration, not durable enough to bill from |
| **`ratelimit` binding** | Per-colo, eventually consistent limiter. No IO, no extra cost |
