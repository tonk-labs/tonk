# Email-First Sign In

Status: draft for implementation
Scope: the account panel's entry screen, the access service's refusal shape, and the sync path that reads it

## 1. Problem

The account panel forks before it knows anything. `#account-choice` offers "Create account" and "Log in", and the user picks. Choosing wrong is expensive in one direction: "Create account" with an address that already has one runs the full WebAuthn creation ceremony (`ceremony.rs:253`), completes it, and only then hits `EMAIL_TAKEN` from `POST /accounts`. The user is left with an orphan passkey in their authenticator and no account.

Nothing checks the address first. `preflight_account` was written for exactly this (`core/accounts.rs:18-24`: *"reject a known uniqueness conflict before the browser creates or evaluates a passkey"*), is routed, handled, and tested, and has zero callers in `tonk-ui`. Its companion `request_account_code` has exactly one occurrence in the tree: its own definition.

## 2. The address decides the branch

The entry screen collects the address first and routes on what it finds, so no passkey is created for an address that already has one.

`GET /customer/{domain}/{local}/did.json` answers this without a code round trip, which is why it is the check rather than `preflight_account`. It also distinguishes a state preflight cannot see: enrolled but unconfirmed.

| Lookup | Button | Ceremony |
| --- | --- | --- |
| 404 | Create account | create, as today |
| 200 | Sign in | `unlock_with_passkey`, as today |
| 202 | Sign in | `unlock_with_passkey`, then the awaiting-activation screen |

The 202 case is the one the two-branch sketch missed. A passkey exists, so creating another is wrong; but the account is not servable, so signing in and saying nothing is also wrong. Signing in and landing on "waiting for confirmation" is what the user needs, and it is where a user who lost the email gets a resend.

The lookup fires 600ms after typing stops, once the input parses as an address. The button label follows it, so the user knows which ceremony is coming before the browser prompt appears.

A "Sign in with passkey" button sits beside the field for the returning user on their own device. The assertion already uses an empty `allowCredentials` (`passkey.rs:248-274`) with `residentKey: "required"`, so the browser's own picker resolves the identity and no address is needed.

## 3. Activation is a refusal that stops

The flow ends on "waiting for email confirmation", and that screen has to notice when confirmation happens, including when the link is opened somewhere else entirely (a phone mail client, another browser).

**No poll.** The account space is already in the sync loop, and while the customer is unactivated every one of its syncs is refused by the access service with a reason. `provisioning.rs:70-76` produces three outcomes from one status:

```rust
fn servable(status: CustomerStatus, who: &str) -> Result<(), AuthorizeError> {
    match status {
        CustomerStatus::Active => Ok(()),
        CustomerStatus::Registered => Err(denial(&format!("{who} awaits email activation"))),
        CustomerStatus::Suspended => Err(denial(&format!("{who} is suspended"))),
    }
}
```

So the state is already on the wire, on every sync attempt, for free. Activation is the refusal turning into a success. Reading that is strictly better than adding a probe: no extra request, no timer, no cadence to tune, and it works wherever the link was opened because it asks the service that actually knows.

This is also what `c939714f` argued for when it deleted the status probe: *"A suspension reaches the client the way it should: the access service denies service and says why in the refusal."*

### 3.1 The refusal needs a typed reason

Today the three cases differ only inside a formatted string. Matching `"awaits email activation"` as a substring would make a wording edit in `provisioning.rs` silently break the client, with no compile error and no failing test.

The denial grows a machine-readable code beside the human `predicate`, so the client matches a value. The predicate stays exactly as it is: it is what a person reads in a log or an error surface.

### 3.2 Both directions

The sync path records the customer state whenever the refusal outcome *changes*, not only when it clears:

- refused, awaiting activation → status `Registered`
- refused, suspended → status `Suspended`
- previously refused, now allowed → status `Active`

Watching both directions means a suspension after activation is noticed too, which a clear-only trigger would miss. The write goes through `record_customer_status`, the single existing writer, so the outcome lands as `AccountCustomer` on profile main.

### 3.3 The UI subscribes

The waiting screen reads `registration()` (`customer.rs:540`) and subscribes to the fact rather than re-fetching. `AwaitingActivation { email }` renders "check your email at {email}" with the resend button; the transition to `Served` re-renders it as done.

This is the point of putting the outcome in a fact: every tab showing the state updates, with no polling on the page side. The panel today reads `crate::api::customer_state()` into untyped JSON once per dashboard load, which is why an activation elsewhere never reaches an open tab.

## 4. Deleting the code ceremony

`preflight_account` and `request_account_code` are superseded by the address lookup: both need an emailed code, and the lookup needs nothing. `CreateAccount.code` is already `Option`, with the comment at `core/accounts.rs:45-47` saying new clients omit it because control of the address is proven by customer activation instead.

Removed: both client wrappers, `POST /codes` and `POST /accounts/preflight` with their handlers and CORS routes, `core/codes.rs`, `CreateAccount.code` and its check, the `email_codes` table with its store methods, and their tests.

This changes the `POST /accounts` wire contract: it no longer accepts `code`. Nothing deployed sends one, since no client ever called the code endpoints.

## 5. Commands, not routes

The sign-up path uses `POST /api/customer/enroll`, one of four `/api/customer/*` routes the route-table skill names as commands wearing HTTP. Enrollment is user intent, its outcome is a fact, and it is triggered from a page: it is a command. It migrates here, since this change touches it.

The other three (`/api/customer`, `/api/customer/pending`, `/api/customer/pending/custody`) are untouched by this work and stay as they are.

## 6. Open

**`data-mode="blocked"` renders nothing.** `account.rs:991` sets it when the account service is unreachable, but no `#account-blocked` panel exists and no `[data-mode="blocked"]` rule is in the CSS, so `set_mode` hides every panel and the user sees a bare error string. Same state machine as this work; worth fixing while the entry screen is being rebuilt.

**The e2e harness pins the current shape.** `account_flow.rs` asserts on `#account-choose-create`, `#account-choose-link`, and the `data-mode` values throughout. Replacing the fork breaks those assertions, which is correct: they are the executable spec of the flow being replaced, and they get updated in the same change.
