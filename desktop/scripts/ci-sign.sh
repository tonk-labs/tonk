#!/bin/bash
# Run only on a trusted macOS CI runner. Secrets never enter the Nix store.
set -euo pipefail
cd "$(dirname "$0")/../.."
[[ "${RUNNER_OS:-}" == macOS ]] || { echo 'Signing requires a macOS CI runner.' >&2; exit 1; }
source desktop/scripts/channel.sh
# Only stable carries the xyz.tonk passkey profile; nightly signs without entitlements.
passkeys=$([[ "$desktop_channel" == stable ]] && echo 1 || echo 0)
required_inputs=(MACOS_CERTIFICATE_P12 MACOS_CERTIFICATE_PASSWORD RUNNER_TEMP)
[[ "$passkeys" == 1 ]] && required_inputs+=(TONK_PROVISIONING_PROFILE_BASE64)
for required in "${required_inputs[@]}"; do
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
if [[ "$passkeys" == 1 ]]; then
  printf '%s' "$TONK_PROVISIONING_PROFILE_BASE64" | base64 --decode > "$signing_dir/profile.provisionprofile"
  export TONK_PROVISIONING_PROFILE="$signing_dir/profile.provisionprofile"
  security cms -D -i "$TONK_PROVISIONING_PROFILE" > "$signing_dir/profile.plist"
  python3 desktop/scripts/passkey-entitlements.py "$signing_dir/profile.plist" "$signing_dir/expected.plist" "$TONK_SIGN_IDENTITY"
else
  unset TONK_PROVISIONING_PROFILE
fi
# Import before entering Nix; the shared build uses the host Keychain and Xcode.
nix --accept-flake-config develop .#ci -c build:desktop
app="desktop/.build/$desktop_app_name.app"
codesign --verify --strict "$app"
signed_team=$(codesign -dv "$app" 2>&1 | sed -n 's/^TeamIdentifier=//p')
[[ "$signed_team" == 8WVKS2F24C ]] || { echo 'Signed app is not from Tonk Labs.' >&2; exit 1; }
codesign -d --extract-certificates="$signing_dir/cert" "$app" 2>/dev/null
if [[ "$passkeys" == 1 ]]; then
  codesign -d --entitlements :- "$app" > "$signing_dir/signed.plist" 2>/dev/null
fi
python3 - "$signing_dir" "$app" "$TONK_SIGN_IDENTITY" "$desktop_bundle_id" "$desktop_channel" "$passkeys" <<'PY'
import hashlib, plistlib, sys
from pathlib import Path
root, app = map(Path, sys.argv[1:3])
identity, bundle_id, channel, passkeys = sys.argv[3], sys.argv[4], sys.argv[5], sys.argv[6] == '1'
assert hashlib.sha1((root / 'cert0').read_bytes()).hexdigest().upper() == identity.upper(), 'Unexpected signing certificate'
info = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
assert info.get('CFBundleIdentifier') == bundle_id, 'Unexpected app identifier'
assert info.get('TonkChannel') == channel, 'Unexpected app channel'
if passkeys:
    expected = plistlib.loads((root / 'expected.plist').read_bytes())
    signed = plistlib.loads((root / 'signed.plist').read_bytes())
    assert all(signed.get(key) == value for key, value in expected.items()), 'Signed entitlements differ'
    assert (app / 'Contents/embedded.provisionprofile').read_bytes() == (root / 'profile.provisionprofile').read_bytes(), 'Embedded profile differs'
    assert info.get('TonkDirectPasskeyEnabled') is True, 'Direct passkeys are disabled'
    print('Verified Tonk signing identity, embedded profile, associated domains and direct-passkey flag.')
else:
    assert not (app / 'Contents/embedded.provisionprofile').exists(), 'Unexpected embedded profile'
    assert 'TonkDirectPasskeyEnabled' not in info, 'Direct passkeys need the stable profile'
    print(f'Verified Tonk signing identity and {channel} app identity.')
PY
mkdir -p desktop/artifacts
ditto -c -k --sequesterRsrc --keepParent "$app" desktop/artifacts/Tonk-macos-arm64.zip
(cd desktop/artifacts && shasum -a 256 Tonk-macos-arm64.zip > Tonk-macos-arm64.sha256)
