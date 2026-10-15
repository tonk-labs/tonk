#!/bin/bash
# Run only on a trusted macOS CI runner. Secrets never enter the Nix store.
set -euo pipefail
cd "$(dirname "$0")/../.."
[[ "${RUNNER_OS:-}" == macOS ]] || { echo 'Signing requires a macOS CI runner.' >&2; exit 1; }
for required in MACOS_CERTIFICATE_P12 MACOS_CERTIFICATE_PASSWORD TONK_PROVISIONING_PROFILE_BASE64 RUNNER_TEMP; do
  [[ -n "${!required:-}" ]] || { echo "Missing signing input: $required" >&2; exit 1; }
done
umask 077
signing_dir=$(mktemp -d "$RUNNER_TEMP/tonk-desktop-signing.XXXXXX")
keychain="$signing_dir/signing.keychain-db"
password=$(openssl rand -base64 32)
original_keychains=()
while IFS= read -r item; do
  [[ -n "$item" ]] && original_keychains+=("$item")
done < <(security list-keychains -d user | sed -e 's/^[[:space:]]*"//' -e 's/"[[:space:]]*$//')
cleanup() {
  security list-keychains -d user -s "${original_keychains[@]}" >/dev/null 2>&1 || true
  security delete-keychain "$keychain" >/dev/null 2>&1 || true
  rm -rf "$signing_dir"
  rm -f desktop/.build/tonk-passkey-profile.plist desktop/.build/tonk-passkey-entitlements.plist
}
trap cleanup EXIT
printf '%s' "$MACOS_CERTIFICATE_P12" | base64 --decode > "$signing_dir/identity.p12"
printf '%s' "$TONK_PROVISIONING_PROFILE_BASE64" | base64 --decode > "$signing_dir/profile.provisionprofile"
security create-keychain -p "$password" "$keychain"
security set-keychain-settings -lut 3600 "$keychain"
security unlock-keychain -p "$password" "$keychain"
security import "$signing_dir/identity.p12" -k "$keychain" -P "$MACOS_CERTIFICATE_PASSWORD" -T /usr/bin/codesign >/dev/null
security set-key-partition-list -S apple-tool:,apple: -s -k "$password" "$keychain" >/dev/null
security list-keychains -d user -s "$keychain" "${original_keychains[@]}"
identities=()
while IFS= read -r identity; do
  [[ -n "$identity" ]] && identities+=("$identity")
done < <(security find-identity -v -p codesigning "$keychain" | sed -nE 's/.* ([[:xdigit:]]{40}) ".*/\1/p')
[[ ${#identities[@]} -eq 1 ]] || { echo 'Certificate archive must contain exactly one valid signing identity.' >&2; exit 1; }
export TONK_SIGN_IDENTITY=${identities[0]}
export TONK_PROVISIONING_PROFILE="$signing_dir/profile.provisionprofile"
security cms -D -i "$TONK_PROVISIONING_PROFILE" > "$signing_dir/profile.plist"
python3 desktop/scripts/passkey-entitlements.py "$signing_dir/profile.plist" "$signing_dir/expected.plist" "$TONK_SIGN_IDENTITY"
# Import before entering Nix; the shared build uses the host Keychain and Xcode.
nix --accept-flake-config develop .#ci -c build:desktop
app=desktop/.build/Tonk.app
codesign --verify --strict "$app"
signed_team=$(codesign -dv "$app" 2>&1 | sed -n 's/^TeamIdentifier=//p')
[[ "$signed_team" == 8WVKS2F24C ]] || { echo 'Signed app is not from Tonk Labs.' >&2; exit 1; }
codesign -d --entitlements :- "$app" > "$signing_dir/signed.plist" 2>/dev/null
codesign -d --extract-certificates="$signing_dir/cert" "$app" 2>/dev/null
python3 - "$signing_dir" "$app" "$TONK_SIGN_IDENTITY" <<'PY'
import hashlib, plistlib, sys
from pathlib import Path
root, app = map(Path, sys.argv[1:3])
expected = plistlib.loads((root / 'expected.plist').read_bytes())
signed = plistlib.loads((root / 'signed.plist').read_bytes())
assert all(signed.get(key) == value for key, value in expected.items()), 'Signed entitlements differ'
assert hashlib.sha1((root / 'cert0').read_bytes()).hexdigest().upper() == sys.argv[3].upper(), 'Unexpected signing certificate'
assert (app / 'Contents/embedded.provisionprofile').read_bytes() == (root / 'profile.provisionprofile').read_bytes(), 'Embedded profile differs'
info = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
assert info.get('CFBundleIdentifier') == 'xyz.tonk', 'Unexpected app identifier'
assert info.get('TonkDirectPasskeyEnabled') is True, 'Direct passkeys are disabled'
print('Verified Tonk signing identity, embedded profile, associated domains and direct-passkey flag.')
PY
mkdir -p desktop/artifacts
ditto -c -k --sequesterRsrc --keepParent "$app" desktop/artifacts/Tonk-macos-arm64.zip
(cd desktop/artifacts && shasum -a 256 Tonk-macos-arm64.zip > Tonk-macos-arm64.sha256)
