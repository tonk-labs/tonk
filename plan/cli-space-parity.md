# One account, and one way a space joins it

**Goal:** Give the native CLI one space model with two rules the user has to
remember: tonk is signed into one account at a time, and a local space can be
linked into that account — after which it stays there and reaches other people
through an invite.

**Approach:** Build on `staging` after merged PR #726 (`53821ebe3`), using its
account-as-profile-main upstream, signed account directory, membership roles,
and `/provider/add` protocol. One installation store (`spots.json`), one Dialog
identity, one account slot. Ownership is a property of each space — the account
root that owns it — not of a profile the user selects, so replicas outlive the
account switches that made them unreachable.

**Constraints:**
- One account at a time. `tonk account link` (alias `login`) refuses while
  another account is signed in; `tonk account logout` clears the slot.
- Spaces are never deleted, hidden, or re-tagged by an account switch. A space
  belonging to another account stays registered and listed, and becomes usable
  again when that account signs back in.
- A command that would open a space belonging to another account is refused
  during resolution, before any site is opened, with copy that names the owner
  and points at an invite.
- Signed out, every replica on the device stays readable and writable. Only
  account services stop. Signing out is not disowning.
- `tonk space new` creates local-only when signed out, and account-owned when
  signed in. Linking an account never scans, provisions, or enrolls the spaces
  already on the device.
- The only ownership transition is local → the signed-in account, one space at
  a time. An account-owned space cannot be linked to another account.
- Linking is non-destructive: the space keeps its name, site, subject, and
  directory binding. Every step is idempotent or guarded by the state the last
  attempt left, and the registry is tagged last, so an interrupted link leaves
  a working local space and a retry converges. No rollback, no journal, and no
  `/provider/remove`.
- A space with an upstream that is not the account's own content service, a
  durable member other than this device, or recorded invitations is not
  eligible. Fail before provider or registry mutation.
- Sharing never changes ownership: `tonk invite` / `tonk join` add members.
- Local removal, account ownership, provider hosting, membership, authority,
  and remote bytes stay separate. `tonk space rm` removes a local replica and
  nothing else.
- Canonical vocabulary `space`, `--space`, `TONK_SPACE`, `account spaces`, with
  visible `spot`, `--spot`, `TONK_SPOT`, `account spots` compatibility aliases.
- Out of scope: browser profile UX, account-to-account ownership transfer,
  delegation-chain rebasing, rotating existing bearer links, provider billing
  transfer, and multiple simultaneous accounts on one installation.

## Product contract

```text
$ tonk space list
NAME      SUBJECT         ACCOUNT      ROLE     ACCESS
scratch   did:key:...     -            local    yes
garden    did:key:...     did:key:aa   owner    yes
roadmap   did:key:...     did:key:bb   owner    no

spaces marked no belong to another account; sign back into it, or ask its
owner for an invite
```

Reaching one of those is refused where the space is selected, not where it
fails:

```text
$ tonk assert task --title "…"
error: this account doesn't have access to 'roadmap'
'roadmap' belongs to another account (did:key:bb); ask its owner for an invite
and claim it with `tonk join <URL>`
```

Linking is one word and no target, because there is only one account:

```text
$ tonk space link garden
linked	garden	did:key:...
account: did:key:aa
site: /…/spots/garden
```

An account-owned space explains itself rather than moving:

```text
$ tonk space link garden
error: "garden" already belongs to an account, so it stays there.

Once a space is synced with an account, it stays owned by that account.
This keeps existing shares working.

Share it instead:
  tonk invite

owner account: did:key:aa
```

## Rejected alternatives

- Labeled profiles with `account add`/`account use`. Multiple simultaneous
  accounts multiplied the state (identity, session, registry, replica storage)
  for a case the product does not have, and made every command answer "which
  profile?" before it could answer anything else.
- Deleting or hiding a previous account's replicas on switch. Data the user
  can still read is not the same as data they can sync, and losing the former
  to explain the latter is a bad trade.
- Blocking every command while signed out. It breaks offline editing of spaces
  this device demonstrably holds authority over.
- Account-to-account ownership transfer. Revoking the old shared authority
  prefix can invalidate downstream users and existing invite chains.
- Silently reinterpreting a link as a share; that hides the owner/member
  distinction.

## Durable data model

`spots.json` stays the one public registry. Two additions, both optional and
omitted when empty, so an older `tonk` or a third-party writer round-trips a
local-only registry byte-for-byte:

```rust
pub struct SpotEntry {
    pub site: PathBuf,
    /// Root DID of the account this space belongs to; absent = local-only.
    pub account: Option<String>,
}

pub struct AccountRecord {
    pub root: String,
    pub ceremony_origin: Option<String>,
    pub access_remote: Option<String>,
    pub revocation_relay: Option<String>,
}

pub struct Registry {
    pub spots: BTreeMap<String, SpotEntry>,
    pub bindings: BTreeMap<PathBuf, String>,
    /// The account signed in here, if any.
    pub account: Option<AccountRecord>,
}
```

Access is a comparison, evaluated in `SpotStore::resolve`:

| space `account` | registry `account` | outcome |
| --- | --- | --- |
| none | any | allowed (local-only) |
| some | none | allowed (signed out, offline-first) |
| some | equal | allowed |
| some | different | `SpotError::ForeignAccount` |

Role is never stored. It is read from the repository's own signed
`Membership`/`MemberRole` facts: founder → `owner`, member → `member`, an
untagged space → `local`, and an account-tagged space whose membership cannot
be read → `unknown`, listed with a diagnostic rather than hidden.

## File map

- `plan/cli-space-parity.md`: this contract and its verification state.
- `rust/tonk-cli/src/spot.rs`: registry, account fields, and the access rule.
- `rust/tonk-cli/src/inventory.rs`: the local listing and role classification.
- `rust/tonk-cli/src/space_link.rs`: local → account linking and its preflight.
- `rust/tonk-cli/src/account.rs`: link/logout/status against the one store,
  plus the sign-in phase read-only commands use.
- `rust/tonk-cli/src/account_spots.rs`: signed-directory list, pull (tagging
  what it registers), and the push-required directory record linking uses.
- `rust/tonk-cli/src/site.rs`: founder membership and the adopt-vs-load
  account-root prefix boundary.
- `rust/tonk-cli/src/bin/tonk.rs`: `account link|logout|status|spaces|…`,
  `space new|list|link|rm|unbind`, and the copy above.
- `rust/tonk-cli/tests/space_access.rs`: the access rule and its copy.
- `rust/tonk-cli/tests/space_inventory.rs`: rows, roles, access, diagnostics.
- `rust/tonk-cli/tests/space_link.rs`: live link, retry, refusals, switch.
- `rust/tonk-cli/tests/cli_spot.rs`: CLI-level refusals, listing, and parsing.

## Upgrading an installation linked before this change

A `spots.json` written by PR #726 has no `account` fields at all, so an
already-linked device reads as signed out: every space keeps working offline,
`tonk space list` shows them as `local`, and sync keeps using the credentials
the device already holds. `tonk account link` is the one-command fix — the
sign-in refusal keys off the registry record, so it runs, and it writes the
account root and the content endpoints that `tonk space new` and `tonk space
link` need. Deriving those endpoints from the stored descriptor instead was
rejected: the revocation relay is not recoverable from it, and guessing it
would produce spaces hosted with an invitation channel nobody can revoke.

## Verification checkpoint — 2026-08-20

Delivered on `feat/cli-space-parity-build` over `origin/staging`:

- One installation store and one Dialog identity; the labeled-profile roster,
  its `profiles.json`, and the cross-profile share path are gone.
- `tonk account link`/`login` records the one account; `logout` clears it and
  leaves every replica readable and writable.
- `tonk space link <space>` performs the whole local → account transition
  (authority, `/provider/add`, remote, founder membership, push, retained
  delegation, pushed directory record, registry tag) with no destructive step.
- `tonk space list` shows account, role, and access, and says what to do about
  a row it cannot reach.
- Resolution refuses another account's space with the invite copy, in one
  place, so every space-opening command inherits it.

Verified:

- `cargo test -p tonk-cli --features integration-tests` — all suites pass,
  including live account/access-service coverage of linking, relinking, the
  account switch, and every refusal. One run of the whole suite saw a single
  failure in `tests/site.rs` that did not reproduce in four later runs of that
  suite alone or of the whole suite; treat it as load flakiness under full
  parallelism until it recurs with a name attached.
- `cargo clippy --workspace --all-targets --all-features` — clean.
- `cargo fmt --all --check` — clean.

Not run here: the WASM suite (`test:web:debug`), unchanged by this work — no
shared schema or provider behavior was modified.
