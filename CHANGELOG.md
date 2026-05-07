# Changelog

## 0.2.1

Adds `git`-style remote management commands and optional anonymous
usage telemetry.

### New commands

- **`carry remote list`** (alias `ls`) -- list configured remotes.

- **`carry remote show <NAME>`** -- show a single remote's URL,
  subject DID, and upstream status.

- **`carry remote set-upstream <NAME>`** -- wire an existing remote
  up as the push/pull target after the fact.

- **`carry remote remove <NAME>`** (alias `rm`) -- unregister a remote
  and clear its upstream link if it was the upstream.

### New flags

- **`carry remote add --set-upstream`** (`-u`) -- also wire the new
  remote up as the push/pull target in one step, mirroring
  `git remote add -u`. Without the flag the remote is registered but
  no upstream is set; use `carry remote set-upstream` later.

### New

- Anonymous usage telemetry. Each invocation reports the command name,
  carry version, and a blinded blake3 hash of the profile DID to a
  Cloudflare Worker (`carry-telemetry-service`). No IP addresses or
  raw DIDs are collected. Opt out with `DO_NOT_TRACK=1`. A notice is
  printed at `carry init` time and in `carry --help`.

### Changed

- Repository metadata (remotes, branches, tracking links) now lives on
  a dedicated `meta` branch as `tonk-schema` concepts rather than
  being scraped from dialog's on-disk layout. `remote list` / `remote
  show` query that branch; `push` / `pull` continue to drive the
  underlying dialog state directly, and the two are kept in sync
  inside each `remote` command.

## 0.2.0

Repository sync between devices and collaborators via UCAN-S3.

### New commands

- **`carry remote add <NAME> <URL>`** -- register a sync destination.
  Accepts `https://` URLs for UCAN-S3 access services (recommended) or
  `s3://` with explicit `--endpoint`/`--region`/`--bucket` flags for
  direct S3. Optional `--subject <DID>` overrides the default (this
  repo's own DID) for cross-repo pulls.

- **`carry push`** -- fast-forward the configured remote with local
  changes. Exits with an error if the remote has diverged (run
  `carry pull` first).

- **`carry pull`** -- fetch and three-way merge changes from the
  configured remote.

- **`carry invite <DID>`** -- delegate repository access to a
  collaborator's DID. Outputs an invite URL (default base
  `https://tonk.xyz/join`, override with `--url`) carrying the UCAN
  delegation chain and the access service URL in its fragment. The
  recipient redeems it with `carry join`.

- **`carry join <INVITE-URL>`** -- redeem an invite URL. Saves the
  delegation chain, registers the sync remote, and pulls the latest
  data in one step.

### Changed

- `carry invite` now takes a DID argument (the collaborator to invite)
  instead of generating an ephemeral bearer token. Simpler, fewer
  moving parts, and the delegation chain works immediately without
  re-delegation on the recipient's side.

- Bumped `dialog-db` dependency to `ab6f1a08` (`feat/repository-redesign`)
  which brings:
  - `From<RemoteBranch> for UpstreamState` (no more `NodeReference` in
    carry's public surface)
  - `remote(name).create(SiteAddress)` with default subject
  - `From<S3Address>` / `From<UcanAddress>` for `SiteAddress`
  - `profile.access().claim().delegate()` replaces `Ucan::delegate()`
  - `Repository<SignerCredential>` type parameter

### Security

- Direct S3 credentials (`--access-key`/`--secret-key`) are persisted in
  plaintext inside `.carry/`. A warning is printed at `remote add` time.
  Prefer a UCAN-S3 access service (`https://` URL) so credentials stay
  on the server.

## 0.1.0

Initial release. Local-first semantic database CLI with `init`, `assert`,
`query`, `retract`, `status`, and `identity` commands.
