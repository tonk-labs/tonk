# Account activation gate

Status: draft for implementation
Scope: `tonk-access-service` presign path, the browser custody ceremony, and the worker's pending-publish outbox
Builds on: #726 (`feat/account-envelope`), implements the enforcement half of `plan/Access metering.md` §3.1–3.2

## 1. What is missing

`plan/Access metering.md` already specifies the registration lifecycle, and #726 implements all of it: `/customer/enroll` writes a `Registered` customer, the service emails a self-signed `/customer/activate` invocation, and executing it flips the customer to `Active`. The status is recorded, surfaced on the dashboard, and polled.

Nothing reads it on the hot path. `provisioning::screen` exists in `rust/tonk-access-service/src/provisioning.rs` and is never called: the presign path runs `screen_credentials` and `screen_consumer_state` and stops there. Every space, and every account, is served identically whether or not its customer ever confirmed an email address. Any self-minted keypair stores bytes under its own namespace, unbilled and unattributable.

This plan wires the gate in and enforces it unconditionally.

## 2. The rule

A subject is servable only when an `Active` customer pays for it:

- the subject is itself a customer with `status = Active`, or
- the subject is a consumer whose `provider` names a customer with `status = Active`.

`Registered` (enrolled, email not yet confirmed) and `Suspended` both deny. An unknown subject, and a consumer with no provider, deny. A store failure is the service's own unavailability — 503, not a denial billed to the customer.

That is exactly what `screen` already implements. This plan does not change the predicate; it calls it.

### Adding also requires activation

Registration commands are intercepted in `serve` before `presign` runs (`handlers/ucan.rs`), so `/customer/enroll`, `/customer/activate`, `/provider/add`, and the deletion family never reach the presign gate. For enroll and activate that is load-bearing: activation itself must work while the customer is `Registered`, or nothing could ever activate.

`/provider/add` is different, and this plan **reverses `plan/Access metering.md` §3.3**. That section allowed adding under a `Registered` customer on the grounds that servability is derived state and such a consumer derives to denied anyway. The reversal makes the rule uniform instead: an unactivated customer gets nothing, neither service nor provisioning. `add` checks the status itself and refuses anything but `Active`.

The cost §3.3 predicted is real — a client that creates spaces before the email is confirmed now holds deferred provisioning work — and section 5 is where that queue is specified. What it buys is one rule with one enforcement point per act, rather than an accepted write whose effect is silently withheld.

## 3. Enforcement

`provisioning::screen` is called from both presign paths, after cryptographic authorization succeeds and alongside the existing screens:

- the worker: `presign` in `rust/tonk-access-service/src/handlers/ucan.rs`
- the native twin: `handle_request` in `rust/tonk-access-service/src/helpers/server.rs`

Always enforced. No configuration var, no ramp: a flag defaulted off is a gate that is not there, and one defaulted on is a switch for turning billing enforcement off in production. The refusal is an `AuthorizeError::PolicyViolation` carrying the cause, which answers 403 and meters as an attributable denial through the path that already exists.

Both paths must gate. The native server backs the integration tests and `tonk dev`, so a gate wired only into the worker would be untested and would behave differently in development than in production.

## 4. The ceremony ordering problem

With the gate on, browser account creation breaks. Today (`rust/tonk-identity/src/ceremony.rs`, `create_custody_account`):

```
1. generate the account secret, create the custody passkey, seal the envelope
2. publish the custody cell        <- /memory/publish on the CUSTODY DID, via /ucan/
3. sign the account-creation request
4. /customer/enroll                <- the account DID becomes a Registered customer
5. /provider/add kind=custody      <- the custody DID becomes a consumer
```

Step 2 is a data-plane write screened by the gate, and at that moment its subject is neither a customer nor a consumer: both rows are created in steps 4–5. So it is denied, and `create_custody_account` treats a refused publish as fatal — deliberately, because an account whose sealed secret was never published can only be unlocked by the page that is still holding it.

Regular spaces do not have this problem. `create_repository` invokes `/provider/add` before any remote is attached, and the first block write happens later, only once sync is enabled. Provisioning already precedes the data plane there.

## 5. The pending-work queue

Nothing an unactivated customer asks for can succeed: not `/provider/add`, not the custody publish, not provisioning a space created in the meantime. Rather than fail each of those at its call site, the client records them as pending work and replays them when activation lands.

The queue is a list of entries at a credential site in the profile repository, the way the account provider record and the customer record already live (`state.profile.credential().site(...)`). It survives reload and browser restart, which page memory does not — the activation click routinely lands in a different tab, and may land days later.

Two entry kinds:

- **provision** — a consumer DID, its consent chain, and its kind (`space` or `custody`); replayed as `/provider/add`
- **publish** — the custody DID and the sealed envelope bytes; replayed as the `/memory/publish` that fills the custody cell

Ordering matters within the queue: a publish for a custody DID must not be replayed before that DID's provision entry, or the presign gate denies it for having no provider. Entries replay in the order they were recorded, which gives that for free.

Account creation becomes:

```
1. generate the account secret, create the custody passkey, seal the envelope
2. sign the account-creation request
3. /customer/enroll                     -> Registered
4. queue provision(custody, kind=custody)
5. queue publish(custody cell, sealed)
6. drain                                -> both refused while Registered; entries stand
   ...
7. user clicks the activation link      -> Active
8. drain                                -> provision, then publish; entries cleared
```

A space created before activation appends a `provision` entry the same way and is simply not hosted until the drain succeeds; it works locally throughout.

The envelope is sealed under the passkey's PRF-derived KEK before it is ever handed out, so what waits at rest is ciphertext, not key material. That is the same envelope that would otherwise sit in the custody cell, which is itself remote storage the service can read; parking it locally is strictly less exposure, not more.

### Drain points

The queue is drained whenever the customer's status is read or changes:

- immediately after an entry is appended, so an already-active customer never waits
- on the customer status probe the dashboard polls, which is what notices activation
- after `/customer/activate` completes in the same browser

No timer and no background loop: the status probe already runs on the account panel, and it is the only thing that learns about activation. A drain stops at the first entry that fails and leaves it, and everything after it, queued — this is what keeps provision-before-publish honest without the drain having to know why an entry failed.

An entry whose failure means the work is already done is cleared rather than retried: `ConsumerProvided` for a provision (some other device got there first), and a custody cell that already holds an envelope for this credential, which is how `enroll_custody` already reconciles a re-enrolled credential.

### Failure surface

Until the publish lands, the account cannot be unlocked from a *different* browser — `assert_unlock` resolves the cell, and there is nothing there. The creating browser is unaffected: it holds the queue. This is a real narrowing of the window in which an account is portable, and it is why the drain is driven from the status probe rather than left for the next ceremony.

## 6. Saying why

A refusal that only says "forbidden" is unactionable: the user's account is fine, they simply have an unopened email. Both refusal paths name that cause.

The presign gate answers `AuthorizeError::PolicyViolation` whose predicate distinguishes the reason — `"awaits email activation"` for a `Registered` provider, as against `"is suspended"` or `"is not provisioned"`. That string is already what `provisioning::denial` builds; it reaches the client because the refusal body is the serde-tagged error itself, on both the worker and the native server.

`/provider/add` gets a first-class `RegistrationError::CustomerInactive`, alongside the existing `CustomerActive` and `CustomerSuspended`, carrying the same meaning in the registration vocabulary: enrolled, awaiting email confirmation. The worker maps it to a queued entry rather than a surfaced failure, and the dashboard's activation notice — which already reads "Sync activation pending: open the link we emailed to …" — is what the user sees.

### Getting another email

The way out of a stuck `Registered` is always available: `/customer/enroll` is idempotent while `Registered` — the rows stand and the activation email is resent (§3.1 names this the recovery for an expired link, and the link carries its own `exp`, so an expired one fails without any stored state). Enroll is a registration command, so it never touches the presign gate and cannot be locked out by it. The dashboard's activation notice gets a resend control wired to the `enroll_customer` call it already makes, so a user whose link expired or never arrived is one click from another. Nothing else in this plan may refuse while a customer is `Registered` without leaving that path open.

## 7. Tests

- `provisioning.rs` unit tests already cover the predicate. They stay.
- Service: a presign against an unprovisioned subject, against a consumer of a `Registered` customer, and against a consumer of an `Active` customer — the first two 403, the third serves. Driven through the native server so the HTTP path is covered, not just `screen`.
- Service: activation flips a previously denied consumer to servable without any further provisioning call, which is the §3.2 rewrite guarantee.
- Worker: a recorded pending publish is retried and cleared once the customer activates.
- Browser: `it_signs_up_through_the_account_panels` already creates an account and follows the activation link. It gains an assertion that the custody cell is absent while `Registered` and present after activation.

## 7. Consequences for existing deployments

Every space already served by a deployment that has no `consumer` row, or whose provider never activated, stops being served on deploy. That is the intended effect and the reason the metering plan calls provisioning "state, not a credential" — but it is not a silent change, and staging should be watched through a full sign-up before production follows.
