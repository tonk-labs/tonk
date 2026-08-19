-- Ingest: one row per invocation, with the invocation bytes inline.
-- Bulky, write-heavy, disposable once charged and archived; split from
-- the control database so it has its own 10 GB ceiling and its schema
-- churn stays clear of billing state. See plan/Access metering.md.

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
  chain    TEXT    NOT NULL,      -- id of the flattened proof set
  body     BLOB    NOT NULL       -- invocation bytes, proofs by reference
);

-- The transitive proof set, flattened at write time so evidence
-- retrieval is two queries rather than a recursive walk. Written once
-- per unique operator session, referenced by every invocation in it.
CREATE TABLE chain (
  chain TEXT NOT NULL,
  proof TEXT NOT NULL,
  PRIMARY KEY (chain, proof)
);

CREATE TABLE block (
  cid  TEXT PRIMARY KEY,
  body BLOB NOT NULL
);
