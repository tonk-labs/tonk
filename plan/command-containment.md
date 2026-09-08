# Command containment: which space may invoke what

Status: the origin rule below is ENFORCED (see `CommandEnv::may_target_space`
in `rust/tonk-worker/src/router/command.rs`), and the VOCABULARY SPLIT is
landed (`CommandProviders` in the same file): dispatch selects the registry
by origin, so a profile-only command asserted on a space branch matches
nothing at all. The capability-scoped operator under "Where this should go"
is a proposal awaiting a decision.

## The vocabulary split (enforced)

Two registries, selected per dispatch by `CommandOrigin`:

- **Profile branch** (empty origin repo): the full vocabulary — every
  command the worker supports. All UI dispatch is routeless (the FAB and
  Hub commit on the profile branch, commands NAME their target space as a
  field), so nothing the user drives changes behaviour.
- **Space (content) branch**: only what a space may run on itself —
  - `Load`: its target IS the origin branch (route stamping).
  - `InviteRequest`: a space may request an invite FOR ITSELF — the
    space view's blank-canvas share (and the seeded `tonk:invite`
    descriptor) dispatches on the space branch, and the refusal flow
    ("sharing unavailable") publishes to that branch's overlay.
    Self-scoped by `may_target_space`; profile-side remains the
    destination once that surface moves.
  - `RenameRepository`: names its space, refused cross-space by
    `may_target_space`; kept space-side so a space can rename itself.
    The provider updates BOTH records regardless of dispatch origin:
    the space's own `RepositoryName` on its content branch (the
    editable source of truth) and the profile branch's `SpaceName`
    directory mirror (what labels the space on a device that never
    replicated it). The `Replica` row carries no name by design.
    Pinned by `it_updates_both_records_when_a_space_renames_itself`
    (`router/repository.rs`), which dispatches from a space origin.
  - `ExpelMember`: target is the origin space (the command carries only
    the member DID). Interim — "a space could REQUEST to expel, but it's
    really a profile's job"; moving it profile-side needs the command to
    grow a space field, since a profile-branch dispatch has no origin
    space. No shipped UI dispatches it today (the roster only offers
    "make admin"), so the placement is free to change.

Everything else — space lifecycle (create/remove/enable-sync), `Join`,
`PromoteMember`, `ProfileRename`, `PauseSync`, and every
account/passkey/device ceremony — exists only in the profile vocabulary.
A same-shaped transient on a space branch is logged as unmatched
("no handler in the space '…' vocabulary") instead of relying on each
provider's origin check. The origin checks stay as defense in depth for
the commands both vocabularies carry.

Consequence, accepted: the legacy topbar "Enable sync" descriptor
seeded on old space branches now matches nothing when asserted there;
the FAB's profile-dispatched `EnableSync` is the supported path. (The
space-side `tonk:invite` dispatch is deliberately NOT contained — the
space view's share/refusal surface lives on the space branch today.)

The split is pinned by
`a_space_origin_selects_a_vocabulary_without_profile_only_commands`
(`router/command.rs`), which proves each probe shape decodes on the
profile before asserting its absence on the space side.

## The problem

A command is matched by SHAPE: the set of attribute names a transient carries
is its whole identity. Nothing about matching says who asserted it or where.
Several commands name their target space as a field (a DID), so before the
origin rule landed, a same-shaped fact committed on any content branch — a
joined space's own notation, or a same-origin POST to that space's
`/transact` — could act on a *different* space: rename it, pause its sync,
attach a remote to it, or mint an invite for it. Only `RemoveSpace` refused
foreign origins.

The operator cannot catch this. `TonkState` holds ONE operator whose session
grant is `profile.access().claim(Subject::any())` (`session.rs`) — every
capability the profile holds, over every space, bounded only in time. Any code
path holding `&tonk.operator` can act on every space the profile has a chain
for, so "which command may this evaluation surface run" has to be decided
above the operator, at dispatch, where `CommandOrigin { repo, branch, client }`
records where the triggering commit landed.

## The rule (enforced)

> A command may act on the origin space itself, or on a space named by DID
> only when it fired from the profile branch. Space-lifecycle authority lives
> on the profile space; a content space's evaluation surface reaches only
> itself.

`CommandEnv::from_profile()` — the profile branch is the origin whose `repo`
is empty (`transact_profile` never names one).
`CommandEnv::may_target_space(key)` — profile origin, or origin == target.

Per command:

| Command | Target | Constraint |
|---|---|---|
| `RemoveSpace` | DID field | profile-only (pre-existing, kept — destructive) |
| `CreateSpace` | fresh identity | profile-only: a space cannot mint spaces |
| `RenameRepository` | DID field | `may_target_space` + `require_real_space` |
| `PauseSync` | DID field | `may_target_space` (+ `require_real_space` in `run_pause_sync`) |
| `EnableSync` | raw-fact DID | `may_target_space` + `require_real_space` |
| `Invite` | fact DID, else origin | `may_target_space` (self-invite from a space's own branch stays legal — frozen legacy descriptors dispatch it) |
| `ExpelMember` | the origin | safe by construction; revocation chain is the real gate |
| `Load` | the origin | safe by construction (`CreateNotebook` is gone — notebook creation is pure rules + the page-built notation) |
| `PromoteMember` | DID field | unconstrained by origin: the hop must be SIGNED by this profile's account authority over exactly that space, which no foreign branch can forge |
| `ProfileRename`, `Join`, account/ceremony commands | the profile/account | no space target; guarded by email match / passkey ceremony / signed chains |

`require_real_space` additionally keeps user-space controls off system
replicas (the profile's own hidden replica, the account repo).

Known behaviour change: a space seeded before `EnableSync` became its own
command carries a legacy topbar "Enable sync" form asserting the
`CreateSpace` shape on its own content branch — which used to mint a
fresh space as a side effect. That path is now refused (logged); the FAB's
profile-dispatched `EnableSync` is the supported way to attach a remote.

## Where this should go: capability-scoped invocation

The origin rule is dispatch-level containment, like Level-0 path routing:
code, not capability. The principled end state the machinery already
supports is scoping what the OPERATOR may do, so that "who can invoke which
command" is decided by delegation, not by a match on origin strings:

- `dialog-capability` already carries `Subject`, `Ability`, `Attenuate`,
  `Policy`, and `Authorization`; invites already mint `/use`-attenuated
  chains; the access service already refuses a revocation not minted under
  a `/` chain. Command dispatch uses none of it today — the compile-time
  `CommandEnv: Provider<C>` bound is the only gate, and `command.rs`
  explicitly defers the "runtime UCAN-style gate" to later.
- The session grant is the place to start: replace the blanket
  `Subject::any()` claim with per-space claims proven at dispatch time, so
  an evaluation surface's env carries an operator attenuated to its origin
  space (plus, for the profile surface, the space-lifecycle ability). The
  CLI's `AccountBoundOperator` (`tonk-cli/src/account_authority.rs`)
  already demonstrates the wrapper pattern: it exclusively owns
  `Authorize<Ucan>` and every remote fork, and refuses without an active
  account session.
- Open questions before building it: whether a per-space attenuated
  operator can share the storage pool and reactor cache without a
  per-space session-rotation cost; what ability names commands map to
  (`/space/create`, `/space/remove`, …— kebab-cased attenuation segments
  already exist in `dialog-capability`'s ability paths); and whether the
  profile branch doubling as the operator's UCAN `ACCESS_BRANCH` (they are
  the same branch head — see `dialog-operator`'s `operator/access.rs`)
  should survive that change, since today anything that can write profile
  facts shares a head with the delegation store.
