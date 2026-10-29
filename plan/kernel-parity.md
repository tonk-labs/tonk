# The kernel replaces the workers

Tonk runs the person's profile as a persona and each space as a kernel
(`plan/space-kernel.md`, `plan/kernel-capabilities.md`). The kernel path can
replace the browser workers when everything they do works on it, through the
`sapling:kernel` world, and the browser-only code in `tonk-worker` is gone.

## The bar

The real-browser suite (`test:e2e`, `rust/tonk-ui/src/account_flow.rs`,
`service_worker_upgrade.rs`, `custody_relay.rs`) is what the old system does.
It runs against this branch's build: the profile boots the persona, a space its
kernel. A flow is done when its tests pass there.

To run it locally, without a nix build of the app:

- build the app with `trunk build .index.e2e.html --release true --html-output
  index.html --dist <dist>` (the index with `connection-invites`, as the nix
  build makes it), then `scripts/stamp-service-worker.sh <dist>`;
- point `TONK_UI_TEST_ARTIFACT` at `<dist>`, and `TONK_UI_TEST_SERVER` at the
  script of `.#tonk-ui-test-server` (`nix eval --raw
  .#tonk-ui-test-server.drvAttrs.text`), which then builds nothing;
- `CHROMEDRIVER` must match the installed Chrome.

## Where the kernel build stops

The world is cooperating-system#2's (`session` is a persona's or a space's,
with a `representative`; a persona's key is lent for one call of the
lifecycle). In the kernel build:

| Area | Now | How |
|---|---|---|
| A space's delegation | works | `representative.delegate`, answered by the persona's `guest.delegate` through the hosts |
| A site's pages | works | the kernel answers every page load: with the content its routes keep at the address, an asset opened as a page, or its page, which renders the site's view with the renderer the host serves under `/guest/`; and with a page saying what it answered otherwise. The host keeps a page of its own only for a kernel that cannot answer, and the deployment's `space.html` and `profile.html` only to start a site's worker |
| The profile reaching a space: names, copies | works | `space_reach` sends its request with `application.send`; the persona's host passes it over the port the profile's page opens; the answer comes back with `channel.receive` |
| A space's seed and copy | works | the space's kernel asks for the seed it was made from on its first boot, and the persona's host hands along the source's snapshot for a copy; requests wait until it is created |
| Public reads (seeds, the deployment's configuration) | works | `{ ask, read }` on the channel, answered by the host |
| The access service (lookups, signed invocations, ticket claims) | works for the persona | `{ ask, fetch }` on the channel, delivered by the persona's host; a space's host delivers nothing |
| Sign up, sign in | works | `provisional-persona.merge` and the lifecycle's `merge` (see below) |
| Approve a device or a connection, promote a member, delete the account | works | the command asks `permanent-persona.authorize` with the custody intent the old worker asked a page for as the context; the host asks the page for the passkey, opens the account and lends it for the lifecycle's `authorize` (see below) |
| Add a passkey | absent | needs `persona-authority.add-recovery` in the host; no test covers it |
| Sync with a remote | works | Dialog's UCAN site hands each operation to a relay (dialog `feat/ucan-relay`), which opens the subject's replica through `network.open` and `peer.open`; the host signs with the key the chain names and runs the UCAN site against the service. The kernel syncs after each write |
| A space's content, asked of the profile | works | the profile's host passes requests under a space's branches to its kernel, streaming answers that come in pieces |
| An account's spaces left local, once it is active | works | the account's convergence runs on the kernel; the persona tells the host what a space's kernel takes up changed, `{ tell: "changed" }`, and the space's kernel asks for its delegation again |
| A space asking the persona (invites, authority, revocation) | works | `{ ask, profile }` on the channel, up the space's port, answered on the persona's `/_kernel/space/{space}/…` |
| Telling a page to navigate | works | `{ tell: "navigate", client, href }` on the channel, the client named by the host in `x-tonk-client-id` |
| A second account on a device | works | a branch of the profile per account, as before; the kernel asks its host to act for the account the branch is signed in to, or for its provisional persona, `{ ask, standing }` |
| Publishing a space | works | the persona mints the publication and has the service keep its ticket; the space's kernel records it (`RecordPublication`), as it records an invite |
| A space and a profile on the kernel they pin | works | `kernel_pins.rs`: a deployment that ships another kernel leaves each space, and each account's spaces, on the kernel they were made with |

## What the suite says

The whole suite on the kernel build (2026-10-29): 90 tests, 87 pass. The
old worker's upgrade tests are gone (`service_worker_upgrade.rs`, and
`profile_library_repairs_claims_from_an_old_account_writer`); `kernel_pins.rs`
checks in their place that a space, and each account's spaces, stay on the
kernel they were made with when the deployment ships another.

Of the three that failed:

| Test | Why |
|---|---|
| `it_lands_on_the_account_when_signing_back_in_through_another_deployment` | passes on a rerun: the sign-out button was not yet interactable |
| `it_removes_the_test_workspace_after_orderly_shutdown` | passes without `TONK_E2E_KEEP_WORKSPACE`, which keeps the workspace on purpose |
| `it_reaps_browsers_that_outlive_their_driver` | the harness's own `pkill` does not reach its stand-in here; nothing it touches changed |

## Sign-up on the world

What the old worker did with a passkey (`router/custody.rs`), split between
the host, which alone holds a passkey's PRF and the account's key, and the
kernel, which keeps the records:

1. The panel's `CreateAccount` or `LogIn` runs in the persona, which calls
   `persona.merge(email)` on its provisional persona instead of asking the
   page for a ceremony. It keeps the rest (the display name, the device's
   name) until the merge comes back.
2. The host asks the profile's pages for the ceremony with the message the
   old worker sent (`WebAuthnRequest`, intent `CreateAccount` or `Login`,
   by whether the address is registered), and takes the custody hand-off the
   page posts (`{ type: "custody", key, kek, request }`).
3. The host does the custody work with `tonk-identity`: for a new account,
   it makes the account's secret, writes its custody record and enrolls the
   account at the access service; for a sign-in, it reads and opens the
   record. It drops the PRF.
4. The host calls the lifecycle's `merge(provisional, permanent)`. With the
   lent persona-authorities, the kernel has the account delegate to the
   representative (the link), reseals the spaces' keys from the provisional
   persona to the permanent one, makes the permanent one the profile's
   upstream, and names the account, then returns, and the account's key is
   gone.

Settled:

- `merge(id: string)` stays: the kernel passes the email, and the host looks
  it up to know whether this is a sign-up or a sign-in, as it does today.
  The passkey's name and label are the host's to choose.
- The host writes the custody record and enrolls the account at the access
  service, as the one that holds the passkey. The lifecycle's `merge` lends
  both persona-authorities, so the kernel reseals the spaces' keys to the
  permanent persona and makes it the profile's upstream.

Done as above, with these differences:

- The kernel asks its host for what the ceremony made of the account inside
  `merge` (`{ ask, ceremony }`, handing over the link it made) and to enroll
  one it created (`{ ask, enroll }`). A refusal or failure reaches the panel
  as `{ tell: "refused" }` / `{ tell: "failed" }`.
- `access.subject` is optional (WIT change): the account's link to a device
  is of everything it has.
- In `merge`, each space the provisional persona holds is backed up to the
  permanent persona too, delegates to the account, and is settled as the
  old worker's rotation settled it.

Approving with the account's key, done the same way:

- A command that needs the account's key asks `permanent-persona.authorize`
  with the custody intent (`CustodyIntent`, as the old worker sent the page)
  as the context. The host posts that intent to the page as the ceremony's
  `WebAuthnRequest`, opens the account with what the passkey derived, refuses
  one that is not the account the kernel acts for, and lends it for the
  lifecycle's `authorize`. A refusal reaches the kernel as
  `{ tell: "unauthorized" }`.
- `access` gains `meta`, `not-before` and `expiration` (WIT change): a
  device's grant carries where the account syncs in its meta, and a purge is
  delegated to the session for five minutes and invoked by it.
- A page's own ask for a delegation (the bar promoting a member) runs its
  ceremony first: the host keeps the hand-off, tells the kernel
  `{ tell: "delegate" }`, lends the account for the authorization the kernel
  then asks for, and answers the page with the chain the kernel tells it,
  `{ tell: "delegated" }`.

## Decisions

1. **Pages.** No notices so far: the hub opens a space from the creation's
   receipt, and the e2e helper does the same. The kernel answers a site's
   page loads, its 404s included; the host serves only the renderer's
   bundles. Later, the renderer (`tonk-display`) becomes a library the host
   installs into a space, the profile included.
2. **The access service and other workers.** Through the channel
   (`application.send`, `channel.receive`): the persona's host delivers the
   invocations the kernel signs, and reads what anyone may read; it passes a
   request for a space's kernel over the port the page brokers. Not yet
   signed invocations between kernels: a space's host answers what comes down
   its port as a page's request.
3. **The remote.** `network.open(peer, routes)` and `peer.open(subject,
   chain)`, a replica that reads and writes as a layer does.
4. **Approving with the account's key.** `permanent-persona.authorize` and
   the lifecycle's `authorize`, with `access` carrying `meta`, `not-before`
   and `expiration`, as a UCAN does.
5. **A second account.** A branch of the profile per account, as Tonk has
   it; the host acts for whichever the kernel says it is on.
6. **Upgrades.** The old worker's generation tests are gone; what replaces
   them checks that sites stay on the kernel they pin, across spaces and
   accounts (`kernel_pins.rs`).

## Order

1. Sign-up, sign-in and the account flows, on `merge` and `authorize`. Done.
2. The remote: sync, sharing, joining, and a second device. Done.
3. Kernels sites pin, in place of upgrades. Done.
4. Delete the browser-only paths of `tonk-worker`, and the old workers.
