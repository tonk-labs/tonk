# Account

Accounts are simply credentials who's athority spans across set of device profiles. They are used to accrete delegations that provide access to different spaces.

In order to maintain access across all linked device profiles those delegations need to by replicated across them. We do this via dialog, specifically each device profile when linked with an account gains

1. An upstream pointed at account repository
2. Account to device profile delegation granting it access

When profile syncs it gains access to everything that account has being delegated as long as it is covered by account -> profile delegation. When device profile is unlinked delegation from account is revoked.

## Onboarding

When system first loads it performs an onboarding ceremony consisting of follwing steps

1. Generate **interim** account secret.
1. Generate pre-passkey recovery credentials.
1. Generates device profile credentials.
1. Conceals account secret for recovery.
1. Assert concealed secret in profile db.
1. Assert delegeation from account to profile.

At the end of this ceremony we have non-extractable recovery credentials, non-extractable profile credentials and authorization for profile to act on behalf of an account.


### Space creation while onboarding

New spaces can be created during onboarding which consists of following ceremony:

1. Generate space secret.
2. Conceal space secret for recovery with account.
3. Assert concealed secret into profile db.
4. Assert delegation from space to account.

At the end of ceremony we have space -> account -> profile authorization chain enabling profile to operate a space on accountns behalf. We `customer/enroll` also have space secre that can be recovered by account when needed.

## Account Registration

Tonk service will only replicate spaces for accounts registered with a service. To do so account invokes self issued `/customer/enroll` capability invocation providing contact email and account recovery setup invocation.

When registration is initiated system will ask user for their email address and create a passkey credentials and perform following steps similar to onboarding ceremony

1. Generate account secret.
1. Conceals account secret for recovery (with passkey credentials).
1. Assert concealed secret in profile db.
1. Assert delegeation from account to profile.

It also issues `/use/put/memory/cell` invocation from passkey credentials to store concealed account secret so it can be recovered from any device with the same passkey. This invocation is bundled with `/customer/enroll` invocation send to did:web:network.tonk service endpoint.

On succeful response from the service `account/registered` concept is asserted into profile space capturing email and `provider-address`.


We also assert `recovery/passkey` concept capturing details of passkey credentials.
