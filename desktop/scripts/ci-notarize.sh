#!/bin/bash
# Notarize the signed app, then package only the stapled bytes.
set -euo pipefail
export DEVELOPER_DIR=${TONK_XCODE_DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}
unset SDKROOT
cd "$(dirname "$0")/../.."
for required in APP_STORE_CONNECT_KEY_P8 APP_STORE_CONNECT_KEY_ID APP_STORE_CONNECT_ISSUER_ID RUNNER_TEMP; do
    [[ -n "${!required:-}" ]] || { echo "Missing notarization input: $required" >&2; exit 1; }
done
umask 077
notary_dir=$(mktemp -d "$RUNNER_TEMP/tonk-desktop-notary.XXXXXX")
trap 'rm -rf "$notary_dir"' EXIT
key="$notary_dir/AuthKey.p8"
printf '%s' "$APP_STORE_CONNECT_KEY_P8" > "$key"
app=desktop/.build/Tonk.app
archive=desktop/artifacts/Tonk-macos-arm64.zip
# Remove uploadable outputs until notarization and stapling succeed.
rm -f "$archive" desktop/artifacts/Tonk-macos-arm64.sha256
ditto -c -k --sequesterRsrc --keepParent "$app" "$notary_dir/submit.zip"
xcrun notarytool submit "$notary_dir/submit.zip" \
    --key "$key" --key-id "$APP_STORE_CONNECT_KEY_ID" \
    --issuer "$APP_STORE_CONNECT_ISSUER_ID" --wait --timeout 60m \
    --output-format json > "$notary_dir/result.json"
python3 - "$notary_dir/result.json" <<'PY'
import json, sys
result = json.load(open(sys.argv[1]))
print('Apple notarization:', result.get('id'), result.get('status'))
if result.get('status') != 'Accepted':
    raise SystemExit('Apple did not accept the Desktop app')
PY
xcrun stapler staple "$app"
xcrun stapler validate "$app"
codesign --verify --strict "$app"
spctl --assess --type execute "$app"
ditto -c -k --sequesterRsrc --keepParent "$app" "$archive"
(cd desktop/artifacts && shasum -a 256 Tonk-macos-arm64.zip > Tonk-macos-arm64.sha256)
