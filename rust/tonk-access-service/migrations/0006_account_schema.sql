-- The account-db restructure, as one transformation of the deployed
-- schema. The columns rename to say what they hold (`verified` →
-- `verified_at`), the deposited `access` delegation goes away with the
-- deposit design, the ledger space and activation bookkeeping arrive,
-- and `consumer` becomes `subscription` — a row is the customer paying
-- for a consumer, not the consumer itself.
--
-- Both tables change constraints (the primary key renames, `provider`
-- becomes NOT NULL), which SQLite cannot ALTER in place, so each is
-- rebuilt and the rows carried over. Foreign keys are deferred to the
-- commit: `consumer` still references the old `customer` while the
-- tables swap, and by commit both sides of every reference are the new
-- tables.
PRAGMA defer_foreign_keys = true;

CREATE TABLE customer_next (
  account           TEXT PRIMARY KEY, -- the account DID this subscribes
  email             TEXT NOT NULL,
  verified_at          INTEGER NOT NULL DEFAULT 0, -- activation timestamp, 0 while Registered
  terms_version     TEXT,             -- accepted at activation
  terms_accepted_at INTEGER,
  status            TEXT NOT NULL,    -- Registered | Active | Suspended
  plan              TEXT NOT NULL REFERENCES plan(id),
  credit_limit      INTEGER,          -- override, null means use plan
  cycle_anchor_at      INTEGER NOT NULL, -- subscription time, periods derive from it
  limit_code        TEXT,             -- null when under limit
  limit_resets_at      INTEGER,          -- null with code set: cleared by event
  stripe_customer   TEXT,             -- null until payment is set up
  ledger            TEXT,             -- DID of the space this service replicates its accounting into; null until one exists
  activation_sent_at INTEGER          -- when the activation link was last emailed; null reads as never
);

INSERT INTO customer_next (account, email, verified_at, terms_version,
                           terms_accepted_at, status, plan, credit_limit,
                           cycle_anchor_at, limit_code, limit_resets_at,
                           stripe_customer)
SELECT did, email, verified, terms_version, terms_accepted_at, status,
       plan, credit_limit, cycle_anchor, limit_code, limit_resets,
       stripe_customer
  FROM customer;

DROP TABLE customer;
ALTER TABLE customer_next RENAME TO customer;
CREATE UNIQUE INDEX customer_email ON customer(email);

CREATE TABLE subscription (
  consumer        TEXT PRIMARY KEY, -- the DID this subscription is for
  provider        TEXT NOT NULL REFERENCES customer(account), -- the customer who pays
  registered_at      INTEGER NOT NULL,
  archived_at     INTEGER,
  suspend_code    TEXT,
  suspend_message TEXT,
  suspend_until_at   INTEGER,          -- null with code set: indefinite
  size            INTEGER NOT NULL DEFAULT 0,    -- last measurement
  measured_at     INTEGER NOT NULL DEFAULT 0,
  deleted_at      INTEGER,           -- when deletion began; the row disappears when it finishes
  kind            TEXT NOT NULL DEFAULT 'space', -- 'space' | 'custody'
  expires_at      INTEGER            -- when this subscription lapses; null never expires
);

-- `provider` was nullable and meant "not servable"; the new shape has
-- no unservable subscription, so those rows do not carry over. Neither
-- do rows whose deletion already finished — the new model records a
-- finished deletion by the row's absence.
INSERT INTO subscription (consumer, provider, registered_at, archived_at,
                          suspend_code, suspend_message, suspend_until_at,
                          size, measured_at, deleted_at, kind)
SELECT did, provider, registered, archived_at, suspend_code,
       suspend_message, suspend_until, size, measured_at, deleted_at, kind
  FROM consumer
 WHERE provider IS NOT NULL
   AND deletion_state != 'deleted';

DROP TABLE consumer;
CREATE INDEX subscription_provider ON subscription(provider);
