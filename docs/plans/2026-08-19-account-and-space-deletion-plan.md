# Account and hosted-space deletion implementation record

**Status:** implementation and local verification complete; review pending

## Delivered behavior

- New account-backed spaces receive an exact direct
  `space -> account-root /space/delete` grant while the space signer is
  available. The grant is retained with account backup metadata.
- Existing spaces are upgraded only from their original single-proof direct
  broad `space -> account-root` delegation. Indirect member chains are refused.
- The access service registers the proof CID and mode, changes a consumer to
  `deleting` before object removal, denies subsequent storage access, purges
  its `{space DID}/` object prefix, and finalizes an idempotent `deleted`
  marker.
- Access-service customer inventory uses immutable owner metadata only to find
  candidates. Each space still requires its registered cryptographic deletion
  proof. Customer finalization is root-signed and is refused until every owned
  hosted space is deleted.
- The account service accepts only a fresh root-signed `/account/delete`
  invocation whose confirmed email matches. It deletes the account backup
  namespace, then removes link requests, devices, matching email codes, and the
  account row in dependent-first order. Removing the row frees the normalized
  email for a new account.
- The browser account dashboard presents an exact service-backed review,
  separates owned and joined spaces, blocks if authority is unavailable,
  requires typed email, a consequences checkbox, final confirmation, and one
  passkey ceremony.
- `tonk account delete` opens the guarded browser account-deletion review.
  `tonk account spots delete <SUBJECT>` opens the same flow for one owned
  hosted space and preserves the account and every other space. Neither CLI
  command deletes directly or offers an automation bypass.

## Retry and lifecycle boundaries

- Hosted-space deletion is denial-first and idempotent. A failed object purge
  leaves the consumer denied in `deleting`; submitting the same root-authorized
  operation retries it.
- The browser composes space deletion, access-customer cleanup, account-service
  cleanup, and local unlink in that order. A retry tolerates spaces already
  deleted and an access customer already removed.
- There is no account-service-wide persisted `deleting` state in this change.
  The browser order keeps email/account-row deletion last, but the
  account-service endpoint does not independently attest access-service
  completion.
- Local cleanup affects only the initiating device. Tonk makes no claim to
  erase replicas already held on other devices or independent providers.
- Existing content-addressed short invite redirects are not indexed by space;
  they retain their existing bounded expiry rather than being included in the
  hosted space prefix purge.

## Verification checklist

- [x] `cargo fmt --all -- --check`
- [x] `cargo test -p tonk-account`
- [x] `cargo test -p tonk-account-service --features helpers`
- [x] `cargo test -p tonk-access-service --features helpers`
- [x] `cargo test -p tonk-cli`
- [x] `cargo test -p tonk-worker-api`
- [x] native checks for changed crates
- [x] wasm checks for `tonk-access-service`, `tonk-account-service`,
      `tonk-worker`, `tonk-ui`, and `tonk-identity`
- [x] `NEXTEST_TEST_THREADS=4 nix develop . -c test:web:debug`
- [x] `cargo clippy` for the changed native crates
- [x] `git diff --check`
