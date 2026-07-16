# tonk CLI self-update

## Problem

`rust/tonk-cli/README.md` says it outright: "There is no self-update
command yet. To upgrade, re-run the install command." Users installed
via `install.sh` have no way to learn a newer `tonk` exists, and no way
to get it without re-running a `curl | sh` they have to remember.

Stale CLIs are not a cosmetic problem here. A `tonk` built against an
older dialog-db misbehaves against a newer counterpart in ways that
surface as confusing runtime errors rather than a version complaint.
The tool should tell you.

## Scope

Serves the `install.sh` channel only — the one the README leads with.
Copies installed via npm (`@tonk/cli`) or nix are detected and refused
with a pointer to the right command; npm already has an update story
and we do not fight another package manager's bookkeeping.

Delivers both halves: an explicit `tonk update` that performs the
upgrade, and a passive check that nags when a newer release exists.

## Background: why version alone is not enough

Two release channels, both **rolling** (`.github/workflows/cli.yml`):

- `stable` → the `tonk-latest` tag, rebuilt on every push to `stable`.
- `staging` → the `tonk-staging` tag, rebuilt on every push to `staging`.

Both delete and recreate the tag per build. The Cargo workspace version
(`0.4.0`) stays put across many rebuilds, so two binaries reporting
`tonk 0.4.0` may be different builds.

But a *real* CLI update bumps the version. So the version string is a
correct signal for "is there an update worth caring about," and only an
incomplete one for "is this the exact build the channel is serving."

Nothing embeds a git commit in the binary today — the nix derivation
passes no rev. It does not need to: the **release** can carry the
commit via `GITHUB_SHA`, which is already in scope in the workflow. The
build is untouched by this design.

## Design

### 1. Identity and detection

Both release workflows publish one new asset alongside `checksums.txt`:

```json
{ "version": "0.4.0", "commit": "27b74e22b…", "channel": "stable",
  "built_at": "2026-07-16T…Z" }
```

`manifest.json`, ~120 bytes, makes the remote build self-describing.
`checksums.txt` is unchanged, so `install.sh` copies already in the
wild keep verifying against it exactly as before.

Detection splits by intent:

**The nag compares version strings only.** Local is
`env!("CARGO_PKG_VERSION")`, baked into every binary; remote is
`manifest.version`. A release bumps it, a staging rebuild does not, so
the nag fires on real updates and stays quiet through merge churn. This
needs **no receipt** — it works for every existing binary the day it
ships, including hand-extracted ones.

**`tonk update` compares commits.** Same version, different commit
means a newer build of the same version: not nag-worthy, but exactly
what someone on staging typing `tonk update` is asking for.

The compare is semver `>`, not `!=`, so a `TONK_RELEASE` pin to
something newer than stable is not nagged into a downgrade.

### 2. The receipt

`install.sh` writes `dirs::data_dir()/tonk/install.json` after a
successful install — a snapshot of the manifest plus local facts:

```json
{ "channel": "staging", "version": "0.4.0", "commit": "27b74e22b…",
  "install_dir": "/usr/local/bin", "installed_at": "…" }
```

It is not the backbone of detection; it earns its place for two things:

- recording **which channel** to check, so a staging user is not
  quietly checked against stable;
- letting `tonk update` answer "already current" without downloading a
  ~10MB tarball to find out.

Written best-effort. A failed manifest fetch must never fail an
install — it just leaves no receipt. No receipt means assume stable,
and `tonk update` downloads blind once and writes one.

**`install.sh` is served from the release, not the repo.** Old copies
run forever and cannot be fixed retroactively, so this change must be
purely additive. New installs get receipts; old installs self-heal on
first `tonk update`; the nag never cared.

### 3. `tonk update`

Resolve channel (receipt → `TONK_CHANNEL` → stable), fetch
`manifest.json`, compare commits. Same commit prints
`already current: 0.4.0 (27b74e2, stable)` and stops — no download.
Otherwise download the tarball and `checksums.txt`, and verify SHA256
before anything touches disk.

Target is `std::env::current_exe()`, not the receipt's `install_dir`:
the running binary is the truth, the receipt may be stale or describe a
different copy.

That is also the foreign-install guard. `current_exe()` under
`/nix/store` (read-only) or inside `node_modules` — what an
`npm i -g @tonk/cli` copy looks like — is refused with the right
command for that channel. The rule is enforced by looking at where we
actually live, not by trusting a receipt.

The swap prepares fully on a temp file in the **same directory** as the
target, then renames over it:

1. Extract to `<dir>/.tonk-update-XXXX` (same dir: `rename()` cannot
   cross filesystems).
2. `chmod 0755`; on macOS `xattr -c` then `codesign --force --sign -`.
3. Run `<temp> --version` and require success.
4. `rename()` over the target — atomic; on Unix, replacing a running
   executable is safe, the kernel holds the running inode.
5. Rewrite the receipt.

Step 3 is the point of the ordering. `install.sh` smoke-tests
`--version` *after* overwriting, so a bad binary is already your `tonk`
by the time you find out. Testing before the rename means a failure at
any step leaves the working binary untouched — there is no rollback
path to get wrong, because nothing is ever half-applied.

An unwritable target dir (`/usr/local/bin` usually) reports the
directory and suggests `sudo tonk update`. We never invoke `sudo`
ourselves; a binary that silently escalates to overwrite itself is not
a thing worth building.

### 4. The nag

Runs **at command exit**, concurrently with the telemetry flush the CLI
already awaits, and only when the cache is older than 24h. The marginal
cost is one small GET parallel to a request already in flight, not a
new stall in front of every command. It is independent of telemetry
being enabled — it has its own reason to exist.

State in `dirs::data_dir()/tonk/update.json`, mirroring `telemetry.rs`
(`TONK_UPDATE_STATE` overrides the directory for tests, as
`TONK_TELEMETRY_STATE` does):

```json
{ "check_enabled": true, "last_checked_at": "…", "last_nagged_at": "…",
  "latest_version": "0.5.0", "latest_commit": "abc1234" }
```

The nag prints from the **cache**, never from the in-flight check. If
the check lands before exit, we nag this run; if not, the cache stays
stale and the next run retries. Nothing has to finish in time for
anything to be correct — a slow check just shifts the nag one
invocation later. One line, on stderr, after the command's output:

```
tonk 0.5.0 is available (you have 0.4.0) — run `tonk update`
```

Stderr is load-bearing: agents drive this CLI and parse its stdout, so
a nag on stdout would corrupt `--json` output. At most once per 24h,
only while a newer version exists, never during `tonk update`,
suppressed when `CI` is set. Not suppressed on non-TTY — agents are
exactly who needs to know about version skew.

Opt-out mirrors the two conventions already in the codebase:
`TONK_NO_UPDATE_CHECK=1` (naming follows `TONK_NO_SYNC` in
`auto_sync.rs`) and a persisted `tonk update --disable-check` /
`--enable-check` writing `check_enabled`. Telemetry's `tonk telemetry
off` shape is not mirrored literally because `tonk update off` reads
like it disables updating, not checking.

**No first-run notice for the check.** Telemetry needs one because it
sends data out; the check fetches a public 120-byte file and sends
nothing. A disclosure banner would be noise. Noted because it departs
from the neighbouring module.

### 5. Error handling

The two paths get opposite policies, deliberately.

**The background check fails silently.** No message, no exit-code
impact — offline, DNS failure, rate limit, 404. No `tonk eval` should
turn red because a nag could not reach GitHub. This repo treats silent
failure as a defect, so the exception is named rather than left to look
like an oversight: the check has no caller depending on its result and
no user intent behind it. On failure it still bumps `last_checked_at`,
so an offline laptop retries daily rather than every command.

**`tonk update` fails loudly** — non-zero, distinct message per mode:
unreachable GitHub, missing manifest for the channel (a bad
`TONK_RELEASE` pin), no checksum entry for this platform, unsupported
platform (e.g. `linux-aarch64`, nothing published), foreign install,
unwritable dir, failed smoke test, failed rename. Checksum mismatch is
loudest: refuse, print expected and actual, exit non-zero, change
nothing — it is the only thing between an unsigned binary and your
PATH, and it must behave exactly as the installer's gate does.

In every mode the existing binary is untouched, because the rename is
last.

### 6. Dependencies

Already in the workspace: `reqwest` (its comment in
`rust/tonk-cli/Cargo.toml` says tonk does not call it directly — that
stops being true and the comment needs updating; the reason it exists,
`rustls-tls-native-roots`, means downloads trust the OS CA store and
survive a TLS-intercepting proxy), `sha2`, `tempfile` (currently a
dev-dep of tonk-cli, gets promoted).

New: `tar` + `flate2` to extract (pure Rust, no PATH dependency, makes
extraction unit-testable without a shell) and `semver` for the compare
(hand-rolling a triple works right up to prereleases like `0.4.1-rc.1`,
which the npm workflow already publishes).

## Testing

Pure logic, no network or filesystem: semver compare including
prereleases and pinned-newer, manifest parsing, receipt round-trip,
channel precedence, cache staleness, 24h nag rate limit,
foreign-install path detection.

The swap is the risky half and gets tested for real. Putting the fetch
behind a small trait lets a test drive the actual sequence — extract,
verify, chmod, smoke-test, rename — against a real temp dir with a fake
archive whose "binary" is a `#!/bin/sh` script that prints a version.
That exercises the real `--version` gate and the real `rename()`,
including the case that matters most: **a failed smoke test must leave
the old file byte-identical.**

Tests land in `tests/update.rs` with an explicit `[[test]]` entry
(`autotests = false`), using `#[dialog_common::test]` and `it_does_x`
names grouped by behaviour.

Not covered, stated rather than implied: the real GitHub assets and the
macOS `codesign` path. Both release workflows get a step that parses
the published `manifest.json` — cheap insurance against the CLI
checking for a file the release forgot to publish. The `install.sh`
receipt write and the end-to-end upgrade stay manual verification on a
real install.

## Deliberately out of scope

**Versioned releases.** Making `install.sh` resolve real tags
(`v0.4.0`) the way the npm channel already does would collapse
detection to a plain semver compare with no receipt at all. It is
probably the right long-term shape, but it is a release-process change,
not a CLI feature, and it does nothing for staging, where every push is
a genuinely new build at the same version. This design stays correct
either way: if versioned releases land later, the commit compare keeps
working and the nag gains a nicer message for free.

**Channel switching** (`tonk update --channel staging`). `install.sh`
already does this. Not adding it without a demonstrated need.

**Self-update for npm and nix.** Detected and refused with a pointer,
per scope.
