-- Control state for customer registration: who is billable, which spaces
-- they provide, and the plans that price service. The full metering and
-- billing schema (sponsorships, usage, ledger) arrives with the increments
-- that read it; see plan/Access metering.md.

-- Plan rows are immutable: a repricing inserts a new row rather than
-- mutating one, so any row referencing a plan fully determines how it was
-- priced. Rates are phase-0 placeholders until calibration fixes them.
CREATE TABLE plan (
  id              TEXT PRIMARY KEY,
  name            TEXT NOT NULL,
  credit_limit    INTEGER NOT NULL,
  term            INTEGER,            -- days a customer may sit on it, null is open-ended
  may_sponsor     INTEGER NOT NULL DEFAULT 0,
  read_rate       INTEGER NOT NULL,   -- credits per operation
  write_rate      INTEGER NOT NULL,
  write_byte_rate INTEGER NOT NULL,
  storage_rate    INTEGER NOT NULL,   -- credits per GB per cycle
  compute_rate    INTEGER NOT NULL,
  stripe_price    TEXT                -- null on an unpaid plan
);

CREATE TABLE customer (
  did               TEXT PRIMARY KEY, -- also the DID of its account consumer
  email             TEXT NOT NULL,
  verified          INTEGER NOT NULL DEFAULT 0, -- activation timestamp, 0 while Registered
  terms_version     TEXT,             -- accepted at activation
  terms_accepted_at INTEGER,
  status            TEXT NOT NULL,    -- Registered | Active | Suspended
  plan              TEXT NOT NULL REFERENCES plan(id),
  credit_limit      INTEGER,          -- override, null means use plan
  cycle_anchor      INTEGER NOT NULL, -- subscription time, periods derive from it
  limit_code        TEXT,             -- null when under limit
  limit_resets      INTEGER,          -- null with code set: cleared by event
  stripe_customer   TEXT,             -- null until payment is set up
  access            BLOB NOT NULL     -- deposited delegation to the service over the account space
);

CREATE TABLE consumer (
  did             TEXT PRIMARY KEY,
  provider        TEXT REFERENCES customer(did), -- null means not servable
  registered      INTEGER NOT NULL,
  archived_at     INTEGER,
  suspend_code    TEXT,
  suspend_message TEXT,
  suspend_until   INTEGER,            -- null with code set: indefinite
  size            INTEGER NOT NULL DEFAULT 0,    -- last measurement
  measured_at     INTEGER NOT NULL DEFAULT 0
);

INSERT INTO plan (id, name, credit_limit, term, may_sponsor, read_rate,
                  write_rate, write_byte_rate, storage_rate, compute_rate,
                  stripe_price)
VALUES
  ('trial@2026-08', 'Trial', 1000000000, 90,   0, 1, 5, 0, 100, 0, NULL),
  ('free@2026-08',  'Free',   100000000, NULL, 0, 1, 5, 0, 100, 0, NULL);
