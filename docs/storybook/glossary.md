# Glossary

These terms describe what a user can observe or what must remain distinct to
explain an observable result.

## Identity and account terms

**Profile**
: One local identity context and its local stores. A browser can keep several
  profiles and switch between them. A CLI installation ordinarily uses one
  current profile. Switching or rotating a profile changes the device identity
  available to account and space operations.

**Device**
: One profile DID authorized to act. “Device” is a user-facing label for an
  authorization endpoint; it may be a browser profile or a CLI profile, not
  necessarily a whole physical machine.

**Device DID**
: The DID of the current profile. It is the audience of account-to-device
  authority and is not the account root DID.

**Root**
: A passkey-controlled local identity that can exist without an account
  provider. A root is durable local authority, not proof that provider services
  or sync are available.

**Root DID**
: The DID controlled by the account passkey. Once an account is attached, this
  is the account's durable authority identity.

**Account**
: The root-authorized identity plus its account repository and provider
  relationship. In casual UI text “account” may also include customer billing
  or activation state; the storybook names that state separately.

**Account repository**
: The synchronized repository of account facts such as devices, names,
  passkeys, spaces, and retained delegations. It can be unconfigured,
  unhydrated, or ready independently of the local provider attachment.

**Provider**
: The account or access-service origin attached to a profile and used for
  hosted account operations. A recorded provider can exist while it is offline.

**Attachment**
: One provider-issued generation connecting an authorized device to an
  account. Logging out ends the local active attachment but is not the same as
  revoking the device's authority.

**Customer**
: The account's access-service enrollment. Its state is absent, waiting for
  email confirmation (`Registered`), active, suspended, or unreachable. This
  state controls service availability; it does not erase local identity.

**Activation**
: Presenting the signed link from an email to move a customer from waiting to
  active. The activation page needs the link, not the account passkey.

**Passkey**
: A WebAuthn credential used to control the root and authorize account
  ceremonies. A user can reject, cancel, lose, or use a different passkey.

**Custody**
: Sealed account key material made usable through a passkey assertion. Custody
  publication may be queued until customer activation makes the service
  available.

**Hydration**
: Fetching and mounting the account repository so its facts can be used
  locally. A device may be correctly attached but still unhydrated.

**Login / link**
: Authorizing the current profile as a device and attaching it to an account.
  Browser text says “log in”; CLI implementation and older prose may say
  “link.” Neither action means the customer is active.

**Logout**
: Removing the active provider attachment from the current profile while
  preserving its local identity, root, repositories, and spaces. Logout is not
  device revocation or account deletion.

**Revocation**
: An authority change that prevents a device or invite from acting. It must
  take effect on later remote access even if the revoked actor retains local
  data.

## Space and collaboration terms

**Space**
: A named local registration for one repository and its site data.

**Local-only space**
: A space with no account ownership or upstream service. It remains readable
  and writable offline and can later be linked to the active account.

**Owned space**
: A space listed under and controlled by an account. Ownership does not imply
  that every device has pulled a local replica.

**Joined space**
: A space another authority shared with this profile. It remains outside the
  current account's deletion scope unless its owner deletes or revokes it.

**Hosted space**
: A space whose repository is served by a provider. Hosting can be queued or
  unavailable while customer activation is pending or suspended.

**Space binding**
: A directory-to-space selection pointer. Removing a binding does not remove
  registration or data.

**Upstream**
: The remote branch tracked by a local branch. A branch may have no upstream or
  be synced, ahead, behind, diverged, unreachable, or unauthorized.

**Remote**
: A named service endpoint registered for a space. Registering a remote and
  choosing an upstream are distinct changes.

**Invite**
: A URL carrying authority to claim access to a space. It may be audience-open
  or restricted to one recipient root and may carry remote configuration.

**Claim**
: Redeeming an invite into the current profile. A profile can claim before it
  has an account and later retain that authority into an account repository.

**Delete**
: Permanently remove a named scope. Local space removal, hosted-space deletion,
  and whole-account deletion have different boundaries and confirmations.

**Unbind**
: Remove only a directory's space selection. No space facts or site data are
  deleted.

## Test and evidence terms

**Journey**
: One user goal from a named starting state to an observable and durable ending
  state, including failure and recovery paths.

**Hot path**
: A common or high-impact journey whose failure blocks normal use or risks
  authority, identity, or data.

**Source evidence**
: A behavior inferred from current code or an existing test body. It has not
  necessarily been executed in the current audit.

**Executed evidence**
: A fresh test or hand-verification result against the pinned commit after the
  latest change.

**Covered**
: The journey's normal, rejected, interrupted, recovery, and durable invariant
  cases are all proven at appropriate layers.

Source audit refreshed on the current product branch. Historical tests and
compatibility fixtures retain retired machine vocabulary; every public surface
uses “space.”
