# Slide — sync plan

Status: Phase 1 shipped (push/pull primitives + in-process tests).
Phase 2 is invite/join + remote management.
Scope: bring slide to feature-parity with carry's sync surface so
the agent-driven CLI and the human-driven tonk-ui browser can
read and write the same data through a shared remote.

## Goal

Slide is the headless interface; tonk-ui is the human one. Both
target the same dialog repository — slide writes locally to
`.tonk/`, tonk-ui (via tonk-worker) writes to the browser's
IndexedDB. Two pieces bridge them:

1. **A shared remote** (an access-service endpoint serving
   UCAN-authorized S3) hosts the wire-format artifacts.
2. **A UCAN delegation chain** (an *invite*) lets each surface
   prove it has authority on that remote, even though they hold
   different keypairs in different physical stores.

## Identity model

Slide and tonk-ui hold **different DIDs**, not the same one.
Their profile keypairs live in separate physical stores: slide
writes to `~/Library/Application Support/dialog/tonk/` on the
filesystem; tonk-worker (in the browser) writes to an IndexedDB
database named `tonk.profile`. The two backends don't see each
other, so each surface generates its own keypair on first run.

The architectural answer to "how does the agent's data become
the human's data" is therefore **UCAN delegation**, not shared
storage. This is the same pattern carry already uses:

1. The minter (slide *or* tonk-ui) creates a repository and
   stamps a UCAN delegation chain that grants access on that
   repo's subject DID.
2. The chain is encoded into an invite URL — base58 of the
   delegation bytes plus an optional ephemeral seed for
   audience-open invites.
3. The claimer (the other surface) parses the URL, redelegates
   from the chain's tail to its own DID, and persists the
   resulting chain. From that point on, the claimer's pushes
   and pulls against the shared remote authenticate as the
   claimer's own DID, with the access service walking the
   delegation chain back to the original minter to verify
   authority.

Slide therefore needs `tonk-invite` (mint + claim) plus the
`dialog-remote-ucan-s3` stack (so the operator's authority can
be presented to the access service). The original RFC's "tiny
CLI" framing softens here, but the dep set still excludes
`tonk-access-service` (we never need to *be* an access service,
only talk to one).

## Surface

```
slide invite [--remote <name>]                # mint an invite URL for the local repo
slide join <invite-url>                       # claim an invite into the local site

slide remote add <name> <url> [--subject <did>]
slide remote list
slide remote set-upstream <remote>            # links main → <remote>/main

slide push                                    # pushes main to its upstream
slide pull                                    # pulls main from its upstream
slide sync                                    # pull then push (the agent loop)
```

Reasoning for each:

- **`invite`** mints an audience-open UCAN chain over the local
  repo's subject DID and prints a paste-able URL. With
  `--remote <name>` the URL embeds the remote's access-service
  endpoint as the `remote=` query parameter, so the claimer
  picks up the same upstream. Default is audience-open
  (anyone-can-claim) because it matches the agent → human
  workflow; `--audience <did>` later for the scoped form.

- **`join`** parses an invite URL, claims it (redelegating to
  the local profile's DID), persists the chain via the operator,
  and — when the URL carries `remote=` — registers that remote
  on the local side and sets the main branch's upstream to it.
  After `slide join`, `slide pull` works without further
  configuration.

- **`remote add`** takes an access-service endpoint URL (e.g.
  `https://tonk-access-service.tonk.workers.dev/ucan/`).
  Internally builds a `dialog_remote_ucan_s3::UcanAddress` and
  calls `repository.remote(name).create(address)`. `--subject`
  overrides the remote's subject DID; default is the local
  repo's DID (matches `tonk-worker`'s `create_repository`
  convention).

- **`remote list`** reads the meta branch's `Remote` concept and
  prints the table. Same data the worker already exposes via
  `GET /api/repository/{name}` — interoperability is a property
  of writing the same meta facts, not a separate read path.

- **`remote set-upstream`** sets `main`'s upstream to
  `<remote>/main` (slide is single-branch) and writes a
  `TrackingBranch` meta record. Two writes (dialog + meta) match
  what the worker does in `record_repository_meta`.

- **`push` / `pull`** call dialog's `Branch::push()` /
  `Branch::pull()` directly. Slide has no subscriptions, so
  there's no reactor wrapping to add — it's the dialog primitive
  unmodified. (Phase 1 — already shipped.)

- **`sync`** is `pull` then `push`. Convenient for the
  end-of-task agent loop ("apply my changes, rebase, push"). On
  failure, prints whichever leg failed and exits accordingly.

Exit codes follow the existing convention: 0 success, 4 I/O /
network / not-found / parse-of-invite-URL, 3 dialog-side commit
errors. Sync routes don't have parse / analyze classes for
documents, so codes 1 and 2 don't apply here — invite-URL parse
errors are I/O-class.

## Dependencies to add

```
tonk-invite             # mint + claim, shared with carry / tonk-ui
dialog-remote-ucan-s3
dialog-remote-s3        # address builder is here
dialog-ucan
dialog-ucan-core
dialog-varsig
url
```

The original RFC dropped these. Adding them back is the cost of
the agent ↔ browser handoff being load-bearing. Slide's binary
size grows, but the dep graph still excludes
`tonk-access-service` and the broader sync-service plumbing
(slide is purely a *client* of access services, never a host).

## Meta-branch interop

When slide creates a remote, sets an upstream, or claims an
invite, it writes the same `tonk_schema::Remote` and
`tonk_schema::TrackingBranch` concepts the worker writes. Three
reasons:

1. The browser-side worker's `GET /api/repository/{name}` reads
   these concepts to populate the UI. Without slide's writes,
   tonk-ui wouldn't know the remote exists even after a pull.
2. Future slide invocations (or another tool) discover existing
   remotes by querying the meta branch — no separate registry.
3. After `slide join`, the meta branch records what the claim
   produced (which remote, which upstream branch), so a
   re-running agent can locate it without a separate config
   file.

The exact write sequence mirrors `tonk-worker::router::repository::
record_repository_meta`'s remote/upstream branches, and
`tonk-worker::router::join`'s post-claim wiring. Slide
extracts the small subset it needs into `slide::remote` and
`slide::invite` rather than depending on the worker.

## Test approach

Three layers, all native:

1. **In-process push/pull** — Phase 1 already covers this
   (`tests/site.rs::when_syncing_with_an_upstream`). Sibling
   branch as upstream, no S3.

2. **Mint-and-claim round trip** — slide site A mints an invite,
   slide site B parses it and claims. Verifies the chain
   serializes/deserializes correctly and the claimer's DID
   ends up with delegated authority on A's subject. No remote
   needed.

3. **Remote-as-meta** — `slide remote add` writes a `Remote`
   concept, `slide remote list` reads it back. These don't
   require a working remote; they verify the meta layer.

Real S3 / access-service smoke tests are deliberately out of
scope. We'll add them once there's an integration-test access
service we can stand up locally (or once tonk-ui's
`web-integration-tests` harness can host one).

## Phasing

Each phase ships independently.

**Phase 1 — push/pull primitives.** ✅ shipped. New module
`slide::sync` exposing `push` and `pull` over a `SlideSite`. CLI
subcommands. Tests use the in-process upstream pattern. No new
crate dependencies.

**Phase 2a — invite mint + claim.** Add `tonk-invite` plus its
transitive UCAN deps. New module `slide::invite` with `mint` and
`claim` functions over a `SlideSite`. CLI subcommands `slide
invite` and `slide join`. Tests cover the round-trip plus
malformed URLs. The remote argument on `mint` and the embedded
`remote=` parameter on `claim` integrate with Phase 2b.

**Phase 2b — remote management.** Add `dialog-remote-ucan-s3`
and the rest of the dep block. New module `slide::remote`
exposing `add`, `list`, `set_upstream`. CLI subcommands wire to
the new module. Meta-branch writes match the worker's. Tests
cover the meta-layer round trip.

`push` and `pull` resolve their target by reading the local
branch's upstream pointer. When exactly one remote exists and
the upstream is unset, slide picks it implicitly; with two or
more, it errors out asking for an explicit `set-upstream`. The
agent's normal flow — one remote, set once at site creation
or by `slide join` — never hits the explicit path.

**Phase 3 — agent loop polish.** `slide sync` (pull then
push). Catch authorization failures from the access service
and emit a friendlier message ("remote rejected the operator's
authority — re-check the endpoint or delegation") rather than
the raw dialog error. A `slide doctor`-style check that
confirms the remote handshake without writing.

## Deferred

These stay open but aren't in the v1 plan. Each is flagged so a
future pass picks them up without re-deriving the context.

- **Concurrent-write conflict UX.** Dialog's `pull` resolves
  via tree-merge; schema-layer conflicts (two writers asserting
  different values for the same cardinality-one field) aren't
  surfaced. Revisit once a real workflow hits it.
- **Audience-scoped invites.** The mint path stays
  audience-open (`#seed` fragment present) for v1. Adding a
  `--audience <did>` flag is mechanical once the audience-open
  flow is solid.
- **`slide remote remove`.** Worker doesn't expose a remove
  route either; if a remote really needs to go, dropping the
  meta-branch claim plus the dialog handle is the path. Add
  when a real user needs it.
- **Profile portability across machines.** The current model
  assumes one profile per device, with cross-device sharing via
  invites. If a user wants the *same* DID on two machines, the
  fix is profile keypair export/import — the same problem
  carry already needs to solve. Out of scope here.
