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
| A space's pages | works | the host answers page loads with the site's shell, as the profile's does; the kernel answers the content its routes keep, `/api/health`, `PUT /` and `/asset:{hash}` |
| The profile reaching a space: names, copies | works | `space_reach` sends its request with `application.send`; the persona's host passes it over the port the profile's page opens; the answer comes back with `channel.receive` |
| A space's seed and copy | works | the space's kernel asks for the seed it was made from on its first boot, and the persona's host hands along the source's snapshot for a copy; requests wait until it is created |
| Public reads (seeds, the deployment's configuration) | works | `{ ask, read }` on the channel, answered by the host |
| The access service (lookups, signed invocations) | works for the persona | `{ ask, fetch }` on the channel, delivered by the persona's host; a space's host delivers nothing |
| Passkey ceremonies (sign up, sign in, add a passkey, approve a device, delete) | absent | `provisional-persona.merge`, `permanent-persona.authorize` and the lifecycle's `merge` / `authorize` (see below) |
| Sync with a remote | absent | `network.open(peer, routes)` and `peer.open(subject, chain)` (host: not yet) |
| Telling a page to navigate | not needed so far | the hub navigates from the creation's receipt |
| Upgrades | absent | the bootloader's, not the old worker's generations |

## What the suite says

The whole suite on the kernel build (2026-10-28): 99 tests, 30 pass, 69
fail. Every space flow that needs no account passes: the hub, creating a
space from the collection card, the wizard, a seed link and a remote
template, duplicating (its menu click is flaky), renaming, removing, the
blank canvas, and storing an asset and reading it in parts.

The failures:

| Count | Why |
|---|---|
| 55 | sign-up and sign-in: no passkey ceremony runs, so account creation fails (see below) |
| 11 | the old worker's upgrade generations (`service_worker_upgrade.rs`, and `profile_library_repairs_claims_from_an_old_account_writer`) |
| 1 | `it_returns_bare_join_visits_home`: a bare `/join` is not sent home |
| 1 | `it_switches_between_a_space_and_the_hub_without_an_absence_flash`: a display renders against a blank context while switching |
| 1 | `it_removes_the_test_workspace_after_orderly_shutdown`: the harness's own test |

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
   it makes the account's secret and writes its custody record at the access
   service; for a sign-in, it reads and opens it. It drops the PRF.
4. The host calls the lifecycle's `merge(provisional, permanent)`. The
   kernel has the account delegate to the representative (the link), records
   the root, names the account, and enrolls it, all with the lent
   `persona-authority`, then returns, and the account's key is gone.

Open:

- `merge(id: string)` is too thin for the panel: making the passkey needs
  the name and the label it shows (the email and the display name), which
  is the `recovery` record `add-recovery` takes. Proposed:
  `merge: func(recovery: recovery)`. The host knows the rest: the device's
  name, the deployment's service, and, by looking the address up, whether
  this is a sign-up or a sign-in.
- The lifecycle's `merge` carries nothing the host signed with the passkey
  (the custody consent, the cell's write). So either the host enrolls the
  account at the access service itself, as the one that holds the passkey,
  or the lifecycle's `merge` takes that material too. Proposed: the host
  enrolls, and the kernel learns the customer from the access service.
- The old worker switched to a profile of the account's own
  (`profiles::for_account`, then a reload). On the world the provisional
  persona merges into the permanent one instead: the profile goes on.
- `persona-authority` has no `sign`. Enrolling signs an invocation as the
  account today; it would be signed by the session, under the account's
  delegation, or `sign` comes back for the lent authority.
- Which of add-a-passkey, approve-a-device and delete-the-account become
  `authorize` with a `context` the kernel reads back.

## Decisions

1. **Pages.** No notices so far: the hub opens a space from the creation's
   receipt, and the e2e helper does the same. The host serves each site's
   shell, so a space renders as before.
2. **The access service and other workers.** Through the channel
   (`application.send`, `channel.receive`): the persona's host delivers the
   invocations the kernel signs, and reads what anyone may read; it passes a
   request for a space's kernel over the port the page brokers. Not yet
   signed invocations between kernels: a space's host answers what comes down
   its port as a page's request.
3. **The remote.** `network.open(peer, routes)` and `peer.open(subject,
   chain)`, a replica that reads and writes as a layer does.

## Order

1. Sign-up, sign-in and the account flows, on `merge` and `authorize`.
2. The remote: sync, sharing, joining, and a second device.
3. Upgrades (`service_worker_upgrade.rs`) on the bootloader.
4. Delete the browser-only paths of `tonk-worker`, and the old workers.
