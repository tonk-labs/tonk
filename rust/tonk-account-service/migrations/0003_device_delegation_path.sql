-- Existing deployments applied 0001 before device delegation bytes were
-- retained. Those bytes cannot be reconstructed from the CID, so legacy rows
-- remain NULL while every new registration supplies the exact public path.
ALTER TABLE devices ADD COLUMN delegation_hex TEXT;
