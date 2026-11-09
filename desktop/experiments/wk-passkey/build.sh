#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/../.."
spike_dir="$PWD/.build/wk-passkey"
spike_app="$spike_dir/Tonk Passkey Test.app"
mkdir -p "$spike_app/Contents/MacOS"

# The same App ID is deliberate: test the eventual desktop association.
cat > "$spike_app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>PasskeySpike</string>
<key>CFBundleIdentifier</key><string>xyz.tonk</string>
<key>CFBundleName</key><string>Tonk Passkey Test</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>1</string>
<key>LSMinimumSystemVersion</key><string>15.0</string>
</dict></plist>
PLIST

xcrun swiftc -parse-as-library -target arm64-apple-macos15.0 \
  -module-cache-path "$spike_dir/module-cache" \
  experiments/wk-passkey/PasskeySpike.swift \
  -o "$spike_app/Contents/MacOS/PasskeySpike"

sign_args=()
if [[ -n "${TONK_PROVISIONING_PROFILE:-}" ]]; then
  security cms -D -i "$TONK_PROVISIONING_PROFILE" > "$spike_dir/profile.plist"
  python3 - "$spike_dir" <<'PY'
import json, pathlib, plistlib, sys
root = pathlib.Path(sys.argv[1])
profile = plistlib.loads((root / 'profile.plist').read_bytes())
ent = profile['Entitlements']
app_id = ent.get('com.apple.application-identifier', '')
if not app_id.endswith('.xyz.tonk'):
    raise SystemExit('The profile must explicitly provision xyz.tonk.')
domains = ent.get('com.apple.developer.associated-domains', [])
if '*' not in domains and 'webcredentials:tonk.network' not in domains:
    raise SystemExit('The profile must allow webcredentials:tonk.network.')
selected = {key: ent[key] for key in (
    'com.apple.application-identifier', 'com.apple.developer.team-identifier')}
selected['com.apple.developer.associated-domains'] = ['webcredentials:tonk.network']
(root / 'entitlements.plist').write_bytes(plistlib.dumps(selected))
(root / 'apple-app-site-association').write_text(json.dumps(
    {'webcredentials': {'apps': [app_id]}}, indent=2) + '\n')
print('Generated AASA file for the provisioned App ID in ' + str(root))
PY
  cp "$TONK_PROVISIONING_PROFILE" "$spike_app/Contents/embedded.provisionprofile"
  sign_args=(--entitlements "$spike_dir/entitlements.plist")
else
  rm -f "$spike_app/Contents/embedded.provisionprofile"
  echo "Baseline build: no domain association. Expected to fail until provisioned."
fi
codesign --force --sign "${TONK_SIGN_IDENTITY:--}" --timestamp=none \
  "${sign_args[@]}" "$spike_app"
codesign --verify --strict "$spike_app"
echo "$spike_app/Contents/MacOS/PasskeySpike"
