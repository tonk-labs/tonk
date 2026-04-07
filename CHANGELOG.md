# Changelog

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
  collaborator's DID. Outputs a `carry_inv_` token containing the UCAN
  delegation chain and the access service URL. The recipient redeems it
  with `carry join`.

- **`carry join <TOKEN>`** -- redeem an invite token. Saves the
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
