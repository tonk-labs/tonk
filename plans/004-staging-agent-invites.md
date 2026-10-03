# Staging agent invitations and CLI prerelease

The staging deployment selected the default Cloudflare artifact, which omits
`connection-invites` from the UI and worker. Select the existing invitation-enabled
preview artifact for staging pushes as well as PRs. Stable retains its current
artifact selection.

Release the workspace as `0.6.16-rc.1`, allowing the merged version bump to tag
and publish the compatible CLI to npm's `next` channel.

## Validation and delivery

- Check artifact selection for PR, staging, and stable events.
- Check Nix formatting and evaluated UI feature configuration.
- Verify Cargo release identity and lockfile consistency.
- Open a PR against staging; wait for CI success before merging.
- Verify staging deployment and prerelease publication after merge.

Status: implementation complete. Local artifact-selection and Wrangler rebuild
checks pass for PR, staging, and stable. Nix evaluation confirms both UI and worker
feature flags. Nix and Rust formatting, Cargo release identity, lockfile-only
workspace version checks, and diff whitespace checks pass. PR CI and post-merge
deployment/publication remain pending.
