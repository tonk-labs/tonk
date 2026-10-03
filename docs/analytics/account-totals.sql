-- Read-only snapshot of currently retained hosted accounts, not historical
-- signup events or weekly usage. Run against the intended CONTROL database.
-- No account identifiers, email addresses, or other row data leave D1.
SELECT
    COUNT(*) AS current_accounts,
    COUNT(CASE WHEN status = 'Registered' THEN 1 END) AS awaiting_activation,
    COUNT(CASE WHEN status = 'Active' THEN 1 END) AS active_status_accounts,
    COUNT(CASE WHEN status = 'Suspended' THEN 1 END) AS suspended_accounts,
    COUNT(CASE WHEN verified_at > 0 THEN 1 END) AS ever_verified_current_accounts
FROM customer;
