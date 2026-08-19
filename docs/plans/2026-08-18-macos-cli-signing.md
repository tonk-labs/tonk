# macOS CLI signing and notarization implementation plan

**Goal:** Ensure every macOS `tonk` binary published through GitHub releases or npm is Developer ID signed and accepted by Apple's notarization service before upload.
**Approach:** Add one repository-local composite action that imports release credentials into an ephemeral keychain, signs a copied Nix build output, submits it with `notarytool`, and verifies the result. Call that action from each CLI distribution workflow while leaving pull-request builds secret-free and non-publishing.
**Constraints:**
- Never modify the read-only Nix store output; stage the binary before signing.
- Use a Developer ID Application identity, hardened runtime, and a secure timestamp.
- Fail closed before artifact upload for every push, tag, or manual run that can publish.
- Do not expose Apple credentials to `pull_request` jobs.
- Publish the exact bytes that passed signing and notarization.
- Preserve Linux artifacts and existing release naming, checksums, manifests, and npm package layout.
- Historical pinned builds must use signing logic from the workflow commit, not from the old source ref being built.

## File map

- `.github/actions/sign-notarize-macos/action.yml`: Import credentials, sign one staged CLI binary, notarize it, verify it, and clean up credentials.
- `.github/workflows/cli.yml`: Stage build outputs and sign/notarize release-capable macOS builds.
- `.github/workflows/cli-pin.yml`: Sign/notarize historical macOS builds with the current local action.
- `.github/workflows/cli-npm.yml`: Sign/notarize the macOS binary before npm packaging.
- `install.sh`: Preserve the Developer ID signature during installation.
- `rust/tonk-cli/src/update/swap.rs`: Preserve the Developer ID signature during self-update.
- `docs/macos-cli-signing.md`: Document credential preparation, repository secrets, and release behavior.
- `README.md`: Replace the obsolete unsigned-binary warning.

### Task 1: Add the signing and notarization boundary

**Files:**
- Create: `.github/actions/sign-notarize-macos/action.yml`

**Interfaces:**
- Consumes: `binary-path`, `certificate-p12`, `certificate-password`, `app-store-connect-key`, `app-store-connect-key-id`, and `app-store-connect-issuer-id` inputs.
- Produces: the same binary path, replaced in place with the Developer ID-signed bytes whose ZIP submission Apple accepted.

- [x] Reject any missing input before changing the staged binary.
- [x] Import the base64 PKCS#12 certificate into a randomly passworded temporary keychain and select a `Developer ID Application` identity.
- [x] Sign with `codesign --force --options runtime --timestamp`, then require `codesign --verify --strict` to succeed.
- [x] Submit a ZIP containing the signed binary with `xcrun notarytool submit --wait --output-format json`; require status `Accepted` and print Apple's log on rejection.
- [x] Require `spctl --assess --type execute` to accept the signed binary.
- [x] Delete the temporary keychain, certificate, API key, archive, and notarization response on every exit.
- [x] Parse the action as YAML and syntax-check its shell body.

### Task 2: Route every macOS distribution through the action

**Files:**
- Modify: `.github/workflows/cli.yml:build-cli`
- Modify: `.github/workflows/cli-pin.yml:build`
- Modify: `.github/workflows/cli-npm.yml:build`

**Interfaces:**
- Consumes: the repository secrets `MACOS_CERTIFICATE_P12`, `MACOS_CERTIFICATE_PASSWORD`, `APP_STORE_CONNECT_KEY_P8`, `APP_STORE_CONNECT_KEY_ID`, and `APP_STORE_CONNECT_ISSUER_ID`.
- Produces: the existing `tonk-macos-arm64` artifact name containing the signed/notarized `tonk` binary.

- [x] Copy `result/bin/tonk` to a writable staging path on every matrix leg and upload only that path.
- [x] Invoke the action only for macOS non-pull-request jobs; release-capable jobs must fail if credentials are absent or Apple rejects the binary.
- [x] In the pin workflow, check out the workflow commit under a separate path and invoke its action while continuing to build `inputs.ref` from the workspace root.
- [x] Confirm the release and npm assembly jobs still consume the unchanged artifact names and paths.
- [x] Parse all changed workflows as YAML and run the strongest available GitHub Actions linter.

### Task 3: Preserve the signature after download

**Files:**
- Modify: `install.sh:macOS Gatekeeper handling`
- Modify: `rust/tonk-cli/src/update/swap.rs:prepare`
- Test: `rust/tonk-cli/src/update/swap.rs:tests`
- Modify: `.github/workflows/cli-pin.yml:publish`

**Interfaces:**
- Consumes: the signed binary bytes from the release archive.
- Produces: an installed or self-updated executable with those exact bytes and its embedded Developer ID signature unchanged.

- [x] Add a macOS-only `it_preserves_signed_binary_bytes` test that archives `/usr/bin/true`, installs it through `swap::install`, and asserts the installed bytes are identical; run it before the fix and expect the existing ad-hoc `codesign --force` call to change the bytes.
- [x] Remove installer and self-updater ad-hoc signing and obsolete de-quarantine workarounds while retaining checksum verification, executable permissions, and smoke testing.
- [x] Make historical pinned releases ship the current signature-preserving `install.sh`, because their source ref may contain the obsolete ad-hoc re-signing behavior.
- [x] Run `cargo test -p tonk-cli update::swap::tests`; expect success.

### Task 4: Document provisioning and the user-visible result

**Files:**
- Create: `docs/macos-cli-signing.md`
- Modify: `README.md:direct-download macOS note`

**Interfaces:**
- Consumes: a Developer ID Application certificate exported as PKCS#12 and an App Store Connect API key authorized for notarization.
- Produces: exact administrator instructions for configuring the five GitHub Actions secrets and rotating them.

- [x] Document how to encode the certificate and preserve the PEM API key as GitHub secrets without committing either credential.
- [x] Document the release-versus-pull-request behavior and the fact that raw command-line executables rely on Apple's online notarization ticket.
- [x] Replace instructions that tell users to ad-hoc sign downloaded binaries with a statement that published macOS artifacts are signed and notarized.
- [x] Run `git diff --check`, inspect the complete diff, and re-run YAML/shell validation after the final edit.
