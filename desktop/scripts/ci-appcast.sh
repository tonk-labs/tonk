#!/bin/bash
# Sign the notarized archive for Sparkle and write a signed feed beside it.
set -euo pipefail
export DEVELOPER_DIR=${TONK_XCODE_DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}
unset SDKROOT
cd "$(dirname "$0")/../.."
source desktop/scripts/channel.sh
app="desktop/.build/$desktop_app_name.app"
[[ -d "$app" ]] || { echo "Missing $app" >&2; exit 1; }
# PlistBuddy reports a missing key on stdout, so test its status, not its output.
if ! feed_url=$(/usr/libexec/PlistBuddy -c 'Print :SUFeedURL' "$app/Contents/Info.plist" 2>/dev/null); then
    echo "$desktop_app_name has no update feed; skipping the appcast."
    exit 0
fi
[[ -n "${SPARKLE_PRIVATE_KEY:-}" ]] || { echo 'Missing SPARKLE_PRIVATE_KEY for an app that checks for updates.' >&2; exit 1; }
public_key=$(/usr/libexec/PlistBuddy -c 'Print :SUPublicEDKey' "$app/Contents/Info.plist")
archive=desktop/artifacts/Tonk-macos-arm64.zip
feed=desktop/artifacts/Tonk-appcast.xml
sign_update=desktop/.build/artifacts/sparkle/Sparkle/bin/sign_update
rm -f "$feed"
enclosure=$(printf '%s' "$SPARKLE_PRIVATE_KEY" | "$sign_update" --ed-key-file - "$archive")
signature=$(printf '%s' "$enclosure" | sed -nE 's/.*sparkle:edSignature="([^"]+)".*/\1/p')
swift desktop/scripts/verify-ed-signature.swift "$public_key" "$signature" "$archive"
# The archive sits beside the feed on the same release.
python3 desktop/scripts/write-appcast.py "$app/Contents/Info.plist" \
    "${feed_url%/*}/$(basename "$archive")" "$enclosure" "$feed"
# The app requires a signed feed (SURequireSignedFeed); sign_update embeds it.
printf '%s' "$SPARKLE_PRIVATE_KEY" | "$sign_update" --ed-key-file - "$feed"
grep -q 'sparkle-signatures' "$feed" || { echo 'The update feed was not signed.' >&2; exit 1; }
echo "Wrote $feed for $feed_url"
