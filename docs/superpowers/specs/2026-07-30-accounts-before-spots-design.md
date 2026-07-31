# An account before a spot

## Problem

A first-time visitor who clicks "new spot" is asked to create a passkey — and
nothing else. `CreateSpaceHandler` (`rust/tonk-worker/src/router/repository.rs`)
reads `identity::local_root`, gets `RootRequired`, and posts an
`identity-required` message; `identity_gate.rs` turns that into a modal headed
"Create your local identity" with two buttons, "Create a new passkey" and "Use
an existing passkey".

That passkey is anonymous. It has no email, no recovery, no service behind it,
and the credential lands in the user's passkey manager under an opaque handle.
The user has done the thing that everywhere else on the web means "I have an
account here", and they do not have an account. They find that out later, when
something offers to attach one.

Two costs follow.

The user-facing one is that the spots created against that root are strictly
local. The remote attach in `CreateSpaceHandler` is best-effort and its failure
is a `log!`; account backup (`account_backup::back_up_claim`) no-ops without a
provider. So the spots exist, appear to work, and are one cleared browser
profile away from being gone — with no signal that this was ever in question.

The structural one is that "has durable authority" and "has an account" are two
different predicates in a system that only ever wants one. Every durable
operation checks the first. Sharing, backup, cross-device recovery and
revocation all need the second. The gap between them is where a local replica
silently degrades.

## Decision

Durable authority requires an account. A passkey is created as part of creating
an account, never on its own.

Concretely:

- Creating a spot and joining an invite durably require an attached account.
- The passkey ceremony that used to stand alone is deleted. `<tonk-account>`
  already runs it as part of account creation
  (`rust/tonk-ui/src/account.rs`, `#account-create-submit`), labelled with the
  verified email, so the credential arrives named.
- Visiting an open invite requires nothing. A guest reads and writes and syncs
  with no root and no account, exactly as today.
- Sharing stays gated on durable membership, which now implies an account. That
  is what keeps the delegation chain rooted in something revocable.

### What is not gated

The hub renders for everyone. An account-less guest who has been invited to
three spots still sees those three spots at `/`, and can open and edit them.
Gating the route rather than the action would take that away, and a guest's
replicas are exactly the local state this change exists to protect.

## The gate

`identity-required` becomes `account-required`, and the modal is replaced by a
navigation to the account page.

1. The worker's precondition changes from `identity::local_root` to
   `account::provider(...).is_some()` — the stored `AccountProviderRecord`,
   resolved against the local root. `AccountStatus::Registered` is the bar;
   `Unhydrated` and `Unconfigured` accounts pass. Hydration is a
   synchronization state, and blocking spot creation on it would invent a new
   way to be stuck.
2. On refusal the worker posts `account-required` with the same intent payload
   it posts today.
3. The top document stores the intent in `sessionStorage` and navigates — in
   the same tab — to `/account?next=<current path+query+fragment>`.
4. `<tonk-account>` finishes creating or logging in, then consumes the stored
   intent exactly once and replays it — the same `replay()` that runs today.
   Each arm navigates on success, so the user lands in the spot that was
   created or joined rather than back where they were refused.
5. With nothing parked, a completed ceremony returns to `next` instead.

Steps 4 and 5 belong to the ceremony, not to loading the page. A visit that
merely *finds* an account replays a parked intent and stops there: `next` also
rides the FAB's account link, and honouring it on load would bounce someone who
opened their account settings from a spot straight back out of the page they
asked for.

`next` is validated as a host-relative path (leading `/`, not `//`) before it is
used, so the parameter cannot become an open redirect. It also re-points the
page's "Back" links, which otherwise return to `/` — the one place a gated user
was not.

The stored intent can carry an authority-bearing invite URL. `sessionStorage` is
per-origin and per-tab and dies with the tab; the same URL is already in the
address bar during a join and already stored by the worker as the guest record,
so this adds no exposure class. `PendingIntent`'s `Debug` already redacts it.

### Same tab, and back again

The FAB's account link (`markup.rs`,
`<a class="fab__account-link" href="/account">`) opens in a new tab, and so does
every other link in the hub. The cause is in `guest_host.rs`: a click is
classified as in-app by comparing the resolved URL against the guest's
*synthetic space origin*, and `space_origin_for` only answers for a `did:key`.
The profile guest — the hub, and the space chrome that mounts the FAB — is not
one, so it has no synthetic origin, every link classifies as external, and the
host relays it to `open_external`, which opens our own origin with
`window.open(_, "_blank")`.

`space_origin.rs` already says what should happen — the profile's links "are
genuinely top-level and want the real origin" — so the classifier falls back to
the real origin (`context.origin`) when there is no synthetic one. Off-origin
links still leave through the host and its confirmation dialog.

On top of that the FAB stamps `next` on its account link, so the trip returns to
the spot it started from.

## Membership goes stale

The FAB fetches `/api/repository/{repo}/membership` once, in
`attach_membership`, and stamps `data-share-unavailable` from the answer. Every
later promotion leaves that attribute exactly as it was:

- the join button's own success path sets `hidden` on itself and never clears
  the share mark;
- `promote_to_member` in `share.rs` is fire-and-forget, and its comment defers
  to "the membership check on the next render", which nothing triggers.

So a guest who promotes gets a greyed share button until a reload. `apply_membership`
becomes the single call both success paths make, and the bar re-checks when the
guest window regains visibility — which also covers a promotion that happened
in another tab.

## The command line

The same rule, in the place the CLI already checks. `require_root`
(`rust/tonk-cli/src/site.rs`) becomes an account requirement: `bootstrap_repository`
and the invite-claim path in `invite.rs` demand a stored `AccountProviderRecord`
and point at `tonk account link`. `TONK_UNSAFE_ALLOW_DEVICE_ROOT` keeps its
meaning for isolated fixtures — it already mints a software root, and now also
skips the account check.

`tonk identity link` is retired along with the `/identity/link` route and
`show_cli_link`, and with them `--root`, `--link-url` and `--no-open`, whose
only producer was that route. It exists to provision an anonymous root over a
browser handoff, which is the thing this change removes. `tonk account link`
does the same handoff with an account behind it. `tonk identity` keeps what it
was actually for: reporting this device's DID and its root, and `--reset`.

## Two things this uncovered

Neither is part of the decision above; both had to be fixed to land it.

**Test profiles shared one root.** `test_state` derived every profile's root
from one hardcoded seed, and the account repository's routing key *is* the
root's — so every test profile shared one account repository, whose storage is
not scoped by profile the way a space's is. Two tests linking descriptors that
name different remotes fought over the same mount, and the loser read the
winner's remote and refused it as a conflict. Invisible until the ordering
shifted; attaching an account to the default fixture shifted it. The seed is
now derived from the profile name.

**The local account service could not finish a sign-up.** `tonk-account-local`
captures verification codes in memory and printed nothing, so the code the page
asks for existed only inside that process. It now drains them to stdout, which
is what makes a browser sign-up against a local service possible at all.

## Migration

None. Pre-account local roots are treated as disposable: there is no upgrade
prompt and no compatibility path. A device holding one is `Unregistered`, so it
hits the account gate the next time it creates or joins, and creating the
account adopts the existing root rather than replacing it (`persist` in
`account.rs` already skips `save_root` when one is `Ready`).

## Testing

- Worker: the create and durable-join preconditions refuse without a provider
  record and pass with one; guest visit is unaffected.
- Gate: `next` validation rejects absolute and protocol-relative values; the
  stored intent is consumed once.
- FAB: `apply_membership` clears the share mark on promotion; the account link
  carries a return path.
- CLI: bootstrap and invite-claim refuse without an account and name
  `tonk account link`.
- Browser: first-run create routes to sign-up and replays; an open invite admits
  an account-less guest who can edit; a guest's share leads to sign-up and then
  works without a reload.
