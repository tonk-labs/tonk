-- Lookup by email address: `did:web:{host}:customer:{domain}:{local}`
-- resolves an address to its customer's `did:key`. The address is the
-- lookup key, so its stored spelling has to be the one a caller can
-- reconstruct: normalize existing rows to the `trim().to_lowercase()`
-- form the account service already writes, then make the address unique.
--
-- One address holds one customer. Registration has always worked that
-- way in practice; the constraint is what makes the lookup able to
-- answer with a single DID rather than a set.
UPDATE customer SET email = lower(trim(email));

-- Normalization can merge two spellings of one address into a duplicate,
-- and nothing before now forbade one. This service has no production
-- deployment, so duplicates are dropped rather than reconciled: the
-- highest rowid, the most recently inserted, is kept.
--
-- The self-provided consumer rows of dropped customers go with them.
-- They are deleted first, while the customers they name still exist.
DELETE FROM consumer WHERE provider IN (
  SELECT did FROM customer
   WHERE rowid NOT IN (SELECT max(rowid) FROM customer GROUP BY email)
);

DELETE FROM customer
 WHERE rowid NOT IN (SELECT max(rowid) FROM customer GROUP BY email);

CREATE UNIQUE INDEX customer_email ON customer(email);
