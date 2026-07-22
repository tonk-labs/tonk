# CLI invite links follow the remote's origin

## Problem

`tonk invite` mints a link against a hardcoded base URL, independent of
the remote it embeds. The two diverge:

```
$ tonk invite
warning: could not shorten the invite URL: shortcut PUT returned HTTP 404 Not Found: Not Found
https://hub.tonk.xyz/join?access=RzwAxwTx3pBx...   (~2.8 KB)

$ tonk remote list
origin  https://staging.tonk.xyz/ucan/  did:key:z6Mkjd4VRSvRsZ4sYbKAzvZuZxEeNbLyaZvYguk9gU3id8s2
```

The link points at the production deployment while the repo lives on
staging. Shortening PUTs to `https://hub.tonk.xyz/@`, which does not run
the shortcut service, so it 404s and degrades to the long URL.

Measured, 2026-07-22:

| origin | `PUT /@` |
| --- | --- |
| `tonk.spot` (= `hub.tonk.xyz`, prod) | 404 Not Found |
| `staging.tonk.spot` (= `staging.tonk.xyz`) | 200, returns a base58 hash |

`wrangler.toml` gates the routes on `run_worker_first = ["/@", "/@/*",
...]` under `[assets]`; the production deploy predates that, so the asset
handler swallows `/@`. Staging is current.

The web UI never hits this. `repository.rs`'s `invite_url` reads the
service worker's own scope, so the link, the embedded remote, and the
shortcut service are the same origin by construction. The CLI has no
equivalent anchor and picks one by fiat.

Two consequences, one root cause:

1. Shortening fails whenever the remote is not on the hardcoded origin.
2. A `tonk invite` with no `--remote` embeds no `remote=` at all, so the
   joiner lands with no upstream. `tonk share` auto-resolves a single
   remote; `tonk invite` does not.

## Design

### Anchor the invite to the remote

When `--base-url` is absent, derive the base from the resolved remote's
endpoint origin: `https://staging.tonk.xyz/ucan/` becomes
`https://staging.tonk.xyz/join`. This restores the invariant the UI gets
for free — link, remote, and shortcut service are one deployment.

A pure helper in `rust/tonk-cli/src/invite.rs`, unit-testable offline:

```rust
/// Derive the invite base from a remote's endpoint. The invite must
/// live on the remote's own origin: that is the deployment serving the
/// repo, and the only one whose shortcut service can answer.
pub fn base_url_for_remote(endpoint: &str) -> Result<String, InviteError>
```

`invite::mint` keeps its current signature (explicit base, explicit
remote). Resolution happens in `mint_invite` in `bin/tonk.rs`, so the
existing offline tests that pass a literal base stay untouched.

The tradeoff: this assumes the access service and the UI share an origin,
which is how tonk deploys today (one worker serves assets, `/ucan/`, and
`/@`). A split deployment keeps `--base-url` as the escape hatch.

### Resolve the remote by default

`--base-url` becomes `Option<String>`. Clap's `default_value_t` currently
makes "the user passed it" indistinguishable from the default, which is
what blocks the derivation.

Remote resolution moves out of the removed `share.rs` into
`rust/tonk-cli/src/remote.rs`:

```rust
pub async fn resolve(site: &TonkSite, explicit: Option<&str>)
    -> Result<Option<RemoteRecord>, RemoteError>
```

- explicit name: that remote, `UnknownRemote` if it is not registered
- zero remotes: `None` — a local-only invite against the fallback base
- exactly one: that one, embedded as `remote=`
- several: error naming them, directing to `--remote <name>`

`--no-remote` forces a local-only invite when several are registered.

This mirrors `tonk push`'s implicit-when-unambiguous heuristic, and is
the behaviour `tonk share` already had.

### Retarget the fallback base

`DEFAULT_BASE_URL` moves from `https://hub.tonk.xyz/join` to
`https://tonk.spot/join`. It is reached only when a spot has no remote.
Its doc comment calls a change breaking; it is not, because the base is
not a lookup key — invites already minted against `hub.tonk.xyz` keep
redeeming for as long as that host stays up.

Prod will not shorten until `tonk.spot` is redeployed with the current
worker. That is a deploy, not a code change, and is out of scope here.

### Remove `tonk share`

Unused. It is also the other caller of the resolution logic and the only
producer of the launcher URL shape, so removing it first keeps the invite
change small.

Deleted:

- `rust/tonk-cli/src/share.rs` and `pub mod share` in `lib.rs`
- `Command::Share`, `ShareCommand`, `share_op`, and the three
  `print_share_*` functions in `bin/tonk.rs`
- the `share` test modules in `rust/tonk-cli/tests/site.rs`
- `rust/tonk-cli/SHARE.md`
- mentions in `rust/tonk-cli/README.md`, `guide-index.md`,
  `guide-views.md`, `guide-events.md`, `guide-workspace.md`,
  `.claude/commands/tonk.md`, `bench/README.md`

`compose_launcher_url` and the `?name=<space>&then=<path>` parameters go
with it. Tonk-ui's consumer side is untouched: invites still land on
`/join`, and nothing in the CLI emits `name=`/`then=` any more.

## Shape of the change

Two PRs, both onto `staging`.

**PR 1 — remove `tonk share`.** Pure deletion, roughly a thousand lines
across source, tests, and docs. Kept separate so it does not bury the
behaviour change.

**PR 2 — invite follows the remote.** `base_url_for_remote`,
`remote::resolve`, the clap changes, `DEFAULT_BASE_URL`. Around a hundred
lines.

## Testing

Offline unit tests in `invite.rs` for `base_url_for_remote`: a `/ucan/`
endpoint yields `{origin}/join`; a bare origin does too; a non-URL errors.

Offline tests in `remote.rs` for `resolve` across the four cases above,
using the existing site test harness.

An end-to-end shorten-and-claim path already exists at
`tests/site.rs::it_shortens_and_claims_a_minted_invite`, driving a local
access service via `AccessServiceAddress`. Extend it to mint without an
explicit `--base-url` and assert the link's origin matches the harness
remote's, which is the regression this whole change is about.

Manual check against staging: register a `staging.tonk.spot` remote,
run `tonk invite`, confirm a short link with no warning.
