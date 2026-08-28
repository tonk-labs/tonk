# Account and hosted-space deletion design

**Status:** implemented in `feat/delete-account`; pending review and deployment

## Goal

Let a person permanently delete a Tonk account from `tonk-ui` or
`tonk-cli`. Deletion removes every space that account created from Tonk's
hosted services, leaves spaces created by other accounts intact, removes the
account's service-side personal data, and frees its email for a future account.
Tonk does not claim to erase replicas held by other devices or providers.

## Authority

Deleting a hosted space is authorized by an explicit UCAN capability minted by
the space at creation time:

- subject: the space DID;
- ability: exactly `/space/delete`;
- issuer: the space DID;
- audience: the creator's account root DID.

The access service accepts a deletion only when a fresh account-root-signed
invocation presents that exact grant. It rejects broad grants, indirect member
chains, device-signed invocations, onward delegation, and a target other than
the invocation subject.

This exact-shape rule is necessary because existing operational ownership and
invite delegations use an empty command prefix, which otherwise covers every
future command. `consumer.provider` remains a discovery, billing, and
consistency index; it is not deletion authority.

At provisioning, the access service records the CID of the deletion grant on
the consumer. A submitted grant must both verify cryptographically and match
the registered CID.

## Existing spaces

An account may upgrade a pre-feature space when it presents the original,
direct `space -> account-root` ownership delegation. The access service checks
that direct proof and binds the space to a deletion grant for that owner. An
invite-derived or otherwise indirect member chain cannot upgrade a space.

The upgrade is idempotent. A space whose direct ownership proof is unavailable
is reported as not deletable; there is no administrative authorization
fallback.

## Hosted-space deletion

Deletion is a service lifecycle operation distinct from today's local
`tonk spot rm` and Hub removal.

For one space, the access service:

1. verifies the exact deletion capability and fresh root invocation;
2. atomically marks the consumer `deleting` before removing content, so stale
   replicas cannot repopulate it;
3. denies all subsequent storage authorization for the deleted consumer;
4. deletes every R2 object under the `{space DID}/` prefix, including branch
   heads, archive blocks, and blobs;
5. clears the mutable provider association while retaining the immutable owner
   inventory and non-personal denial marker needed to prevent resurrection; and
6. returns an idempotent deletion receipt.

The initiating client then removes its local replica. Other devices' local
replicas are outside Tonk service deletion and may remain, but cannot upload to
the deleted Tonk consumer.

Short invite links are content-addressed rather than space-indexed today. They
are not included in the space prefix purge and remain only until their existing
bounded expiry. Product copy therefore promises deletion of hosted space
content, not every derived redirect object.

## Account deletion

Account deletion is an ordered, retryable client orchestration rather than a
cross-service transaction:

1. Tonk obtains a device-authorized deletion inventory from the access service
   and account backups from the account service. Each
   candidate must still present its registered deletion capability before the
   access service purges it. Joined spaces have no such capability and are not
   candidates for hosted-content deletion.
2. One passkey ceremony creates root-signed invocations for the exact reviewed
   space set, access-customer finalization, and account-service deletion.
3. Tonk submits each space invocation. The access service refuses customer
   finalization while any owned hosted space is not deleted.
4. The access service purges the account repository consumer and removes the
   customer row and its email-bearing control
   data. Its deleted-consumer denial markers no longer retain the provider DID.
5. The account service deletes R2 chain backups and spot-head indexes, then
   atomically deletes dependent link requests and devices,
   matching email codes, and finally the account row. Removing the account row
   frees the normalized email and root DID for a new account.
6. The initiating client clears its local account attachment and reports an
   exact receipt. Local space data is removed only on that initiating device.

If a service step fails, repeating the reviewed operation resumes from the
service state: deleted spaces are idempotent, an already-removed access
customer is accepted, and account-service object deletion can be retried. The
account service does not currently persist a global `deleting` state, so other
account mutations are not blocked during a partial cross-service attempt. The
product flow orders account-row/email removal last, after access-service
cleanup; the account-service endpoint itself does not independently verify an
access-service receipt.

## Product safeguards

The UI and CLI show the same consequences before starting:

- every Tonk-hosted space created by the account becomes unavailable to all
  members;
- spaces created by other accounts remain hosted;
- Tonk cannot erase replicas held on other devices or independent providers;
- account deletion cannot be undone; and
- the email may be used to create a new, empty account after completion.

The UI requires typing the normalized account email, checking a consequences
acknowledgement, accepting a final native confirmation, and completing a
passkey ceremony. `tonk account delete` only opens that browser review; it has
no direct or non-interactive deletion flag. `tonk account spots delete
<SUBJECT>` opens the same safeguards scoped to one owned space and leaves the
account and all other spaces intact.

## Failure and compatibility boundaries

- New account-backed space creation mints and retains the deletion grant.
  Backup and registration failures are surfaced by the existing creation or
  reconciliation path; later reconciliation retries registration.
- Legacy account/spot backup JSON remains readable; new deletion metadata is
  optional and versioned.
- Deleted consumers are denied before R2 cleanup begins, making retries safe.
- A newly created account may reuse a deleted email. It creates a new passkey
  by default and therefore a new root DID; if a caller deliberately reuses an
  old root, old deleted-space denial markers still cannot grant content back.
- Revocation records required to reject old credentials are security records,
  not user content, and may be retained without email or account association.
