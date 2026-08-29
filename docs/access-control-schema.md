# Access service control schema

The D1 database (`CONTROL` binding) behind `rust/tonk-access-service`.
It answers one question on the hot path — may this subject be served —
and holds the billing state that decides it.

Migrations live in `rust/tonk-access-service/migrations/`. The tables
here are the state after all of them; the SQL in each file is the
history, this is the shape.

## Tables

```mermaid
erDiagram
    plan ||--o{ customer : prices
    account ||--|| customer : subscribes
    customer ||--o{ consumer : provides
    account ||--o{ consumer : owns

    plan {
        TEXT id PK "not a DID: an opaque plan id like trial@2026-08"
        TEXT name
        INTEGER credit_limit
        INTEGER term "days on this plan, null is open-ended"
        INTEGER may_sponsor
        INTEGER read_rate "credits per operation"
        INTEGER write_rate
        INTEGER write_byte_rate
        INTEGER storage_rate "credits per GB per cycle"
        INTEGER compute_rate
        TEXT stripe_price "null on an unpaid plan"
    }

    customer {
        TEXT did PK "DID (did:key) of the ACCOUNT: identity and subscription are fused"
        TEXT email UK
        TEXT ledger "DID (did:key), nullable: the space this service replicates metering into"
        TEXT status "enum: Registered | Active | Suspended"
        TEXT plan FK "plan.id, not a DID"
        INTEGER verified "activation time, 0 while Registered"
        TEXT terms_version
        INTEGER terms_accepted_at
        INTEGER credit_limit "override, null uses the plan"
        INTEGER cycle_anchor "periods derive from it"
        TEXT limit_code "null when under limit"
        INTEGER limit_resets
        TEXT stripe_customer "Stripe id, not a DID"
    }

    consumer {
        TEXT did PK "DID (did:key) of the space or custody principal"
        TEXT provider FK "DID (did:key): the customer who pays; null is not servable"
        TEXT owner "DID (did:key): the account whose data this is"
        TEXT kind "enum: space | customer | custody"
        INTEGER registered
        INTEGER expires "reservation lapse; null never lapses"
        INTEGER archived_at
        TEXT suspend_code
        TEXT suspend_message
        INTEGER suspend_until "null with a code set: indefinite"
        INTEGER size "last measurement"
        INTEGER measured_at
        TEXT deletion_state "enum: active | deleting | deleted"
        INTEGER deleted_at
    }
```

`account` is drawn above but is **not a table**. `customer.did` is the
account DID, so identity and subscription share a primary key. That
fusion is deliberate for now and has a consequence worth knowing: an
account has exactly one subscription, forever, with no history of a
lapsed one. Splitting them is the change to make when an account needs
a second subscription or a cancelled one has to be kept.

## What decides service

`provisioning::screen` runs before every presign and asks only about the
**subject**, never the command.

Every subject is a consumer, and every consumer is served on the
strength of its provider's status:

```mermaid
flowchart TD
    A["subject"] --> D{"a consumer?"}
    D -->|no| E["denied"]
    D -->|yes| F{"reservation still held?"}
    F -->|yes| G["denied, retryable: reserved, not provisioned"]
    F -->|no| H{"has a provider?"}
    H -->|no| I["denied"]
    H -->|yes| Z{"the provider's status"}
    Z -->|Active| K["served"]
    Z -->|Registered| L["denied, retryable: awaits email activation"]
    Z -->|Suspended| M["denied"]
```

An account is not a special case here: enrollment writes it a
self-provided consumer row (`did = provider = owner`), so "the
provider's status" is its own. `screen` does look the subject up as a
customer first, but only to save a hop — the consumer path reaches the
same verdict one join later.

This is **one query** (`SELECT_SERVABILITY`): a `LEFT JOIN` from the
asked-for DID to `customer`, to `consumer`, and on to the provider's
`customer`. Read in three separate steps it could see a customer that
activates between the first and the last, and it would cost three round
trips on a path that runs before every presign.

It still reaches D1 directly, every time. `plan/Access metering.md` §11
specifies a read through an isolate cache, then KV, and D1 only on a
miss — but the consumer-state KV namespace does not exist yet, and
`REVOCATIONS_KV` is the only one bound. The value KV is meant to serve
is derived from sponsorships, usage, and plan rates (§11.1), so that
tier belongs with the increment that brings those tables. The join does
not have to wait for it.

Because the gate is subject-level, it cannot serve a read while refusing
a write. Anything needing that distinction has to come from the
delegation chain, not from a status column.

## Registration lifecycle

```mermaid
stateDiagram-v2
    [*] --> Registered: /customer/enroll
    Registered --> Registered: resend the activation link
    Registered --> Active: /customer/activate
    Active --> Suspended: service withdrawn
    Suspended --> Active: restored
```

`Registered` means enrolled with the activation link unopened. Nothing
is served in that state — not the customer's own account space, not any
consumer it provides. Consumers may be *added* while `Registered`
(`plan/Access metering.md` §3.3); only serving waits.

## Roles

Three relationships that a single word would blur:

| Role | Column | Meaning | Cardinality |
|---|---|---|---|
| Account | `consumer.owner` | whose data this is | one per consumer |
| Provider | `consumer.provider` | who pays for it | exactly one, required to serve |
| Sponsor | *(not yet built)* | pledges credits to a consumer it does not provide | zero or more |

`owner` and `provider` are seeded identical and diverge only when a
provider changes.

## Not yet built

`plan/Access metering.md` specifies `sponsorship`, `usage`, `ledger`,
and `run`. They arrive with the increments that read them.

Note that `ledger` names two different things: the planned D1 table
(authoritative accounting) and `customer.ledger` (the space this service
replicates that accounting into, which the account may read but not
write). D1 stays authoritative.

`customer.ledger` is nullable, and the receipt's field is
`Option<Ledger>` to match. A deployment that replicates nothing has no
ledger space to name, and a receipt synthesized locally — the worker's
answer for an already-active customer — names neither a provider nor a
ledger, because it learned about neither. Absent means "not stated
here", never "none exists".

## Kinds of consumer

- `space` — an ordinary tonk space. The default.
- `customer` — the account's own space, written at enrollment.
- `custody` — a passkey's custody principal. One per passkey, so an
  account has as many as it has authenticators. Reservations here
  **must** expire: the DID is PRF-derived and therefore stable, so a
  lapsed reservation is re-derived by the same passkey on a new device,
  which is the recovery case. Only the passkey holder can produce that
  DID, so nothing can squat it.
