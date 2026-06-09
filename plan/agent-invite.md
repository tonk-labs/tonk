# Agent invite — artifact-scoped invites

Status: implemented. Describes how the empty-artifact canvas's copy
button produces an invite link that drops a CLI agent directly on a
specific artifact, and why the design deliberately stores nothing.

## What an agent invite is

A freshly created sheet is an *empty artifact*: a tab with a name but no
content yet. The empty-artifact canvas (see
[tonk-viewer.md](./tonk-viewer.md)) invites the user to hand the artifact
to an agent. The agent link is the handoff:

```
http://<host>/space/<name>/<artifact-entity>@artifact?access=<chain>#<seed>
```

- `<artifact-entity>` is the sheet's `this` (the entity the sheet
  displays — its view/model/entity pair).
- `@artifact` names the concept, so the link routes through the display
  route's `{entity}@{model}` grammar (see
  [tonk-display.md](./tonk-display.md), `parse_subject`) and lands on
  that artifact's view.
- `?access=` + `#fragment` are an ordinary tonk invite (see the invite
  format below): the `access` query carries the base58 delegation chain,
  the fragment carries the ephemeral seed for an audience-open invite.

Handing this one link to a CLI agent both **grants it access** (it can
claim the invite and sync) and **points it at the artifact** to edit.

## Two halves: targeting vs. scope

Separate what the link *targets* from what it *grants*:

- **Targeting** is the path (`/space/<name>/<entity>@artifact`). Fully
  under our control — the display route already parses it.
- **Scope** is the UCAN delegation in `?access=`. In dialog the
  **repository (database) is the unit of sharing**: a delegation grants
  access to a whole repo DID, by design. So the link points at one
  artifact but the access it carries is the whole repo's.

This is intentional, not a limitation to be lifted — there is no
per-artifact capability scope. The link's artifact targeting is a
*navigation* affordance (drop the agent on the right view), layered over
a repo-wide grant.

## The secret problem, and why we store nothing

For an **audience-open** invite the URL fragment is a 32-byte Ed25519
**private-key seed** — anyone who has the fragment can claim. That makes
the full URL a credential.

This rules out the obvious "store the generated link so the canvas can
show it again" approach: the synced prolly-tree DB replicates to every
peer, so a seed written there is leaked to everyone, forever.

We considered and rejected several storage shapes:

- **Command + Provider asserting the URL as a fact.** The command
  subsystem (see [commands.md](./commands.md)) communicates results
  through DB facts — which is exactly what we must not do with the seed.
- **In-memory secret map + opaque id fact.** Keeps the seed out of the
  DB (RAM only) but needs an id channel, a fetch route, and dangling-id
  handling across service-worker restarts.
- **Cache Storage keyed by the entity.** Avoids the id channel but the
  Cache is on-disk; acceptable (it's local, not synced) yet still more
  machinery than needed.

The realization that collapsed all of it: **we don't need to store
anything.** A link is cheap to mint and meant to be used once. So:

> The endpoint mints an invite and returns it in the HTTP response. The
> web component shows it. Nothing is persisted anywhere — the seed exists
> only transiently in the response and in the element's DOM for the
> current session.

Re-opening the canvas (or reload) simply mints a fresh link. Stable links
across sessions would require *deriving* the seed deterministically from
the artifact (a future optimization), not storing it.

## How it's built

This reuses the existing repo-invite endpoint unchanged — an artifact
link is that endpoint called with an artifact-targeted base URL.

### The endpoint (existing, reused)

`POST /api/repository/{repo}/invite` (`tonk-worker`,
`router/create_invite.rs`) already:

1. takes a caller-supplied `base_url` in the request body,
2. mints a repo delegation
   (`profile.access().claim(&repository).delegate(audience)`) with a
   fresh ephemeral seed,
3. serializes the invite onto `base_url` via `Invite::to_url` (appends
   `?access=…[&remote=…]#…`),
4. returns the URL in the response — **no storage**.

The repo-level share button drives this the same way: the Leptos client
(`api::create_invite`) supplies `base_url = {window.origin}/join`. The
worker is origin-agnostic; the client owns the origin.

### The element (new)

`<tonk-invite>` (`tonk-workspace`, sibling of `<tonk-share>`) is the
artifact-scoped caller, presented as a copy button modeled on
`<wa-copy-button>`: a copy icon plus a `label`, the icon swapping to a
check on success. On click it:

1. resolves the repo name from the nearest `<tonk-repository>` ancestor
   (`repo_from_ancestor`, the same source `<tonk-share>` uses),
2. builds `base_url = {window.location.origin}/space/{repo}/{entity}@{concept}`
   from its `artifact` / `concept` attributes,
3. hands the clipboard a `ClipboardItem` backed by the pending mint
   promise, synchronously inside the click so the copy runs under the
   click's user activation (`clipboard.writeText` after the `await`
   would lose it),
4. flashes the check icon on success; on failure (no promise-valued
   `ClipboardItem`, lost activation) reveals the URL + a copy control as
   a fallback.

The mint itself POSTs `{ base_url }` to the existing invite endpoint and
reads the `url` from the response.

The empty-artifact view (`core.yaml`) mounts it as
`<tonk-invite artifact={this} concept="tonk:artifact" button-class=…
label="copy">`; `{this}` is the sheet's artifact entity. It carries the
same `button-class` / `label` reskin attributes `<tonk-share>` grew.

## URL-encoding note

`Invite::to_url` parses `base_url` with the `url` crate before appending
the query/fragment, and the display route splits the path on a literal
`@`. The `@` and `:` in `/space/{repo}/{did:key:…}@artifact` must survive
that round-trip un-encoded. If the `url` crate percent-encodes them, the
final URL is assembled by appending `?access=…#…` to the raw base string
rather than round-tripping the path through `Url`. (Verified during
implementation.)

## Future work

- **Deterministic seed derivation** so the link is stable per artifact
  without storing the secret (re-mint yields the same URL).
