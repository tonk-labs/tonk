# Signing in is the host's

The person types their address, names a new account and runs the passkey on
the host's page: the app's top page, which boots the kernels and gives them
the world they expect. The persona kernel only asks for a sign-in
(`provisional-persona.merge()`) and records what it made of the account
(`lifecycle.merge`). This reverses 750752889, which moved the account panel
into the profile's views.

## WIT

`provisional-persona.merge: func()` takes no address. The embedder asks the
person who they are, then signs them in, or up when they have no account,
and answers with `lifecycle.merge`, or not at all when the person declines.
The embedder may also merge on its own, when the person signs in from the
host without the kernel asking (a space shown at its DID that needs the
account).

## The host

- `kernel_boot.js` (profile SW): `askedToMerge()` posts `{ type: "sign-in" }`
  to the page. A custody hand-off whose `request` is `create-account` or
  `login` is a sign-in, whoever started it: `signing` comes from the intent
  (kind, email, name, device, remote). Refusals and failures still reach
  the kernel as `refused` / `failed`, which put its sign-in away.
- Embedder (`tonk-kernel-host`): `Asked` carries the account's `name`;
  `ceremony()` answers `email` and `name` too, so the kernel no longer keeps
  a pending address.
- Page (`tonk-ui/src/sign_in.rs`): the panel, `#tonk-register`, same
  markup and button words as the profile's views had, in the light DOM of
  the top page:
  1. address: email (autocomplete `username webauthn`), continue
  2. lookup at `{app}/customer/{domain}/{local}/did.json`
     (404: free; else an account has it)
  3. free: display name, "create a passkey" runs `createAccount` in the click
  4. taken: "log in with passkey" runs the login in the click
  5. hand-off answered: ok closes (an account awaiting its emailed link is
     the profile's email notice); `awaiting-activation` says to open the
     link; other errors say why with "try again"
  6. "sign in through another Tonk" goes to `/settings/link?via=<origin>`,
     where the profile's settings pane carries on as before
  cancel posts `ceremony-refused` (name `AbortError`) so the kernel's
  sign-in is put away.

## The kernel

- `account/sign-in` (`tonk:sign-in`) replaces open-registration,
  start-registration, check-email, create, log-in, open-sign-in-via and
  dismiss-registration. Its provider records `tonk:sign-in` (overlay,
  `state:sign-in`, `asked`) and calls `merge()`.
- `merged`, `refused`, `failed` retract the `tonk:sign-in` overlay.
- Deleted: the six `tonk:registration/*` concepts and views, `email-status`
  if nothing else reads it, `registration.rs` stages, the registration
  seats in the hub and over a space.
- `account-task` (profile.yaml): open dispatches the sign-in; done when
  `tonk:sign-in` goes away, `completed` if an account is linked.

## Direct mode

A space shown at its DID asking for a delegation with no account: the page
asks the profile (connector `persona` ask) before any passkey; with none it
opens the panel, and after the merge it runs the delegation.

## Tests

e2e helpers drive `#tonk-register` in the top page instead of the profile
frame. Full e2e must pass.
