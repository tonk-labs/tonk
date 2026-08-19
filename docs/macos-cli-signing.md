# macOS CLI signing

The CLI release workflows Developer ID sign and notarize every macOS binary
before it can be uploaded to a GitHub release or npm. Linux builds are
unchanged. Pull-request builds do not receive release credentials and therefore
upload the Nix build's ad-hoc-signed macOS artifact only for CI inspection.

The release boundary is `.github/actions/sign-notarize-macos/action.yml`. It
copies no credentials into the repository: it imports the certificate into an
ephemeral keychain, signs the already-staged binary with hardened runtime and a
secure timestamp, waits for Apple to accept a ZIP containing that binary, and
then requires `codesign` to validate both its signature and Apple's online
notarization ticket. Any missing credential, rejected submission, or failed
verification stops artifact upload.

## Required credentials

Configure these GitHub Actions repository secrets:

| Secret | Contents |
| --- | --- |
| `MACOS_CERTIFICATE_P12` | Base64-encoded PKCS#12 export containing the Developer ID Application certificate and its private key |
| `MACOS_CERTIFICATE_PASSWORD` | Password used when exporting that PKCS#12 archive |
| `APP_STORE_CONNECT_KEY_P8` | Unmodified PEM contents of an App Store Connect team API private key |
| `APP_STORE_CONNECT_KEY_ID` | App Store Connect API key ID |
| `APP_STORE_CONNECT_ISSUER_ID` | Issuer UUID for the App Store Connect team key |

Use a **Developer ID Application** certificate, not a development, Mac App
Distribution, or Developer ID Installer certificate. The App Store Connect key
must be a team key: Apple does not allow individual API keys to authenticate
`notarytool`. Downloaded `.p8` keys are available only once, so retain the
source credential in the team's secrets manager as well as GitHub.

Export the certificate and private key from Keychain Access as a
password-protected `.p12`, then configure the secrets from a trusted machine:

```sh
base64 -i DeveloperIDApplication.p12 | tr -d '\n' \
  | gh secret set MACOS_CERTIFICATE_P12
gh secret set MACOS_CERTIFICATE_PASSWORD
gh secret set APP_STORE_CONNECT_KEY_P8 < AuthKey_KEY_ID.p8
gh secret set APP_STORE_CONNECT_KEY_ID
gh secret set APP_STORE_CONNECT_ISSUER_ID
```

The commands without redirected input prompt for the value, keeping it out of
shell history. Never commit either credential file or place it in an Actions
artifact.

## Release coverage

- `.github/workflows/cli.yml` signs pushes to `stable` and `staging`, plus
  manually dispatched releases. Pull requests deliberately skip the action.
- `.github/workflows/cli-npm.yml` signs tag-driven and manual npm releases.
- `.github/workflows/cli-pin.yml` signs historical builds with the action from
  the workflow commit. It also ships the current installer so an older
  installer cannot overwrite the new Developer ID signature with an ad-hoc
  signature.

The submitted ZIP is only the notarization transport. Release archives and npm
packages contain the exact signed executable that Apple accepted. Apple's
ticket is published online for Gatekeeper; `codesign --check-notarization` with
a `notarized` requirement verifies that ticket on the runner before CI uploads
the binary. The action does not use `spctl --type execute`: that assessment is
for apps and rejects a valid raw CLI executable as “not an app.”

The first push to `staging` after provisioning the secrets is the end-to-end
credential check. After it publishes, download and inspect the artifact on a
Mac:

```sh
tar -xzf tonk-macos-arm64.tar.gz
codesign --verify --strict --verbose=2 tonk
codesign --display --verbose=4 tonk
codesign -vvvv -R="notarized" --check-notarization tonk
```

## Rotation

Replace all five repository secrets before the certificate or API key expires
or is revoked, then validate with a staging release. Revoke the old App Store
Connect key and remove the old certificate from the team's secrets manager only
after the new staging artifact passes signature and notarization checks.
