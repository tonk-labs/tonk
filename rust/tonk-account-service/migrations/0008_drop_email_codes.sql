-- The emailed verification code is gone. It proved control of an address
-- before an account could be created, which the activation link the
-- access service sends now does instead: an account exists first and its
-- registration state says whether the address was confirmed.
--
-- Nothing read this table after `POST /codes` and `POST /accounts/preflight`
-- were retired, and no client ever called either.
DROP TABLE IF EXISTS email_codes;
