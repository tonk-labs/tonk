# Desktop release channels

Implement in `feat/desktop-release-channels` worktree.

- Reuse the CLI tag resolver and publisher: RC assets go to the version release and `tonk-staging`; final assets go to the version release and GitHub latest.
- Build every main push and publish a rolling `tonk-desktop-dev` prerelease. Serialize dev publication and reject stale main runs.
- Reuse Desktop profile validation; release builds use Swift release configuration, hardened runtime and secure timestamp. Require Apple notarization, staple the app, then archive and checksum the final bytes.
- Keep PR CI without secrets. Document direct downloads in both READMEs.
- Validate signing/notarization failure paths with mocked platform commands, workflow syntax and shell syntax. Real Apple notarization and hosted publication require post-merge CI evidence.

Status: implementation complete and prepared for PR review; not deployed.

Validation:
- 11 Python tests pass, including rejected submissions, rejected notarization,
  stapling/validation/signature/Gatekeeper failures, cleanup, and version stamping.
- actionlint passes for all three changed/new release workflows using the locked
  repository Nix input. The initial global registry attempt failed with a Nix
  cache "Too many open files" error.
- Shell syntax and git diff whitespace checks pass.
- GitHub secret-name inventory confirms all six signing/profile/notarization
  secrets exist; their values and validity were not inspected.
- Native optimized compilation, Apple signing/notarization, Gatekeeper behavior,
  and hosted publishing remain unverified until CI runs. Existing staging/stable
  releases will not gain Desktop assets retroactively.

Download links are documented in the root and Desktop READMEs. All channels
share the existing app identity/data and default production service; this change
adds build distribution, not environment-specific identity or automatic updates.
