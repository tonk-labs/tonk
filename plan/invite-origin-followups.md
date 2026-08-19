# Follow-ups from the invite-origin work

Everything here surfaced while fixing `tonk invite` to build its link
on the resolved remote's origin (#631, #632) and was deliberately left
out of scope. Most of it has since been fixed; what remains is at the
bottom, with what fixing it would take.

## Still open

### `tonk invite` pushes to the upstream, not the remote it embeds

`invite::mint` (`rust/tonk-cli/src/invite.rs`) pushes via `sync::push`,
which is upstream-bound. With several remotes registered, `tonk invite
--remote backup` builds the link on backup's origin, embeds backup's
endpoint, and ships the repo state to origin — the recipient joins a
deployment that never received the data.

#632 warns rather than re-routing. Building a push-to-named-remote path
is the real fix, and it needs a decision first: dialog's `Push`
(`dialog-repository/src/repository/branch/push.rs`) borrows `&Branch`
and reads `branch.upstream()` — there is no remote parameter anywhere
in the type. Either

1. land the parameter upstream in dialog-db and bump the pinned tag, or
2. do it CLI-side by borrowing the upstream (`set_upstream` → `push` →
   restore), which needs no dep bump but is not crash-safe.

In the single-remote case the two coincide, so this only bites
multi-remote setups.

### `wiki-conversion` needs a `display:wiki` baseline

Its bang-form checkpoint was deleted along with the "Model not found"
baseline it had promoted. `display:wiki` has never had one. Promote it
from the next run that renders correctly:

```bash
bench baseline wiki-conversion bench/runs/<run-dir>
```

Note that `baseline.sh` copies without deleting, so a stale baseline
whose checkpoint has gone away lingers silently — it just never matches
a shot again.

### Verify the bench pipeline end to end

The spot migration below was verified per-command against a real
binary (spot registration, DID parse, cwd-independent `remote add` /
`set-upstream` / `invite`, and the registry isolation `cold-onboard`
depends on) but no full `bench run` has been executed since. The
browser half — stack, bridge, shots — is unexercised.

## Fixed

**Process-global env in tests.** Eight tests across `update.rs`,
`update/receipt.rs`, `update/state.rs` and `telemetry.rs` called
`std::env::set_var` in a multi-threaded test binary, each carrying a
`SAFETY` comment claiming "tests in this mod run on one thread per
process invocation" — which is false. This was not a flake: `cargo test
-p tonk-cli --lib` failed outright, with
`it_loads_none_when_the_receipt_is_corrupt` reading the receipt
`it_round_trips_through_the_state_file` had just written. Each module
now has a pure `load_from` / `store_at` core the tests drive directly,
and `resolve_channel`'s precedence split into `channel_from`. No
`set_var` remains in `rust/tonk-cli/src/`.

**`tonk invite --no-shorten` / `TONK_NO_SHORTEN`.** Shortening is a
live `PUT` to the link's own origin. With no remote resolved that
origin is production, so the `(None, None)` base-selection arm could
not be tested without writing to the real shortcut store. It now has
CLI coverage in both flag and environment form. Follows the existing
`--no-sync` / `TONK_NO_SYNC` shape (shared `env_value_opts_out`) rather
than the endpoint override first sketched here — an endpoint override
would break the same-origin invariant the shortcut design rests on.

**Bench is spot-based.** `site.sh` calls `tonk spot new <spot> --site
"$RUN_DIR/site"` (which prints `DID: …` in the same shape `tonk init`
did, so the existing parse survives); `run.sh` exports
`TONK_SPOTS_STATE` and `TONK_SPOT`; `shots.sh` lost its stale
`$site/.tonk` guard and its inert `cd`s, as did
`targeted-edit/prepare.sh`; `episode.sh` passes both variables into the
agent's environment, overridable as `EPISODE_SPOT` /
`EPISODE_SPOTS_STATE`. `cold-onboard` sets both — an empty spot *and* a
separate empty registry, because an empty `TONK_SPOT` alone still falls
through to the registry's `current`, which would have been the origin
site the agent is supposed to be joining.

**Bang-form checkpoints.** Removed from all four scenarios. They were
not merely broken — in `smoke` and `from-scratch` the bang baseline was
byte-identical to its `display:` sibling, so they were duplicates of a
checkpoint already covered. Baselines were renumbered to match, since
`shots.sh` numbers by position and `visual-diff.sh` matches by exact
filename. `bench/README.md`'s route section, which recommended the bang
form and described the pre-#547 Leptos routes, now describes the seeded
`route!` table.

**`base_url_for_remote` strips userinfo.** `https://u:p@h/ucan/` no
longer carries credentials into a link printed to stdout. The
no-trailing-slash shape is pinned too — `https://host/ucan` and
`https://host/ucan/` behave identically because an absolute-path join
discards the base path (RFC 3986 §5.3), which is the shape most readers
would guess wrong about.

**`remote::list`'s doc comment.** It claimed dropped rows are "logged
via the error path"; nothing logs. It also missed a second silent drop
(an unparseable subject DID). Both documented, along with the
consequence `resolve`'s doc left implicit: a repo with two registered
remotes where one fails to decode resolves the other implicitly.

**Analytics classified CLI invitees as `dev`.**
`rust/tonk-analytics/src/web.rs` knew only the old production host and
`staging.tonk.xyz`. It now maps the production origin `tonk.network` too.

**Stale command names.** `tonk concepts` → `tonk concept ls` in
`.claude/skills/tonk-bug/SKILL.md` (agent-facing, so a live footgun)
and `README.md`; `tonk views` → `tonk view ls` and `tonk init` → `tonk
spot new` in `README.md`, whose CLI blurb also still described a
`.tonk/` site in the current directory. `bench/README.md`'s
harness description updated. Its dated measurement sections remain because
they record what past runs actually did.

## Resolved without action

**Production serves the shortcut service.** The checked-in Worker configuration
routes `PUT /@` through the Worker via
`run_worker_first = ["/@", "/@/*", ...]` in `wrangler.toml`. After moving the
production route, verify the new origin directly:

```bash
curl -s -X PUT https://tonk.network/@ --data-binary "/join?access=probe" -w " [%{http_code}]\n"
```

## Test-harness traps worth knowing

**A blocking subprocess call deadlocks an integration test.**
`#[dialog_common::test]` expands to `#[tokio::test]`, a current-thread
runtime. When that runtime also hosts the in-process access service, a
blocking `Command::output()` starves the server the subprocess is
calling and the test hangs with no error. Route those through
`tokio::task::spawn_blocking` — `rust/tonk-cli/tests/cli_spot.rs` does
this and is the working example.

**A stale rlib silently runs the old test binary.** With a shared target
directory, `cargo test -p tonk-cli` can report the previous commit's
test counts with no warning. `cargo clean -p tonk-cli` first when a
count looks wrong.

**`TONK_SPOTS_STATE` names a directory, not a file.** It holds
`spots.json` and the `spots/` root. Passing a path ending in
`spots.json` makes the CLI create a `spots.json/` directory with a real
`spots.json` nested inside — isolation still works, which is why the
mistake survives.
