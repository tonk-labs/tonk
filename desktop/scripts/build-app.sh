#!/bin/bash
set -euo pipefail
# Use host Xcode rather than an inherited Nix SDK.
export DEVELOPER_DIR=${TONK_XCODE_DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}
unset SDKROOT
cd "$(dirname "$0")/.."
node ../mcp/scripts/sync-desktop.mjs
# A stable signer lets WebKit's Keychain access survive executable changes.
# Ad-hoc signing uses the binary hash as its identity and prompts after rebuilds.
identity=${TONK_SIGN_IDENTITY:-}
if [[ -z "$identity" ]]; then
    identities=$(security find-identity -v -p codesigning)
    development_ids=()
    while IFS= read -r fingerprint; do
        [[ -n "$fingerprint" ]] && development_ids+=("$fingerprint")
    done < <(printf '%s\n' "$identities" | sed -nE 's/.* ([[:xdigit:]]{40}) "Apple Development:.*$/\1/p')
    if [[ ${#development_ids[@]} -ne 1 ]]; then
        echo "Set TONK_SIGN_IDENTITY to a code-signing identity from security find-identity -v -p codesigning." >&2
        echo "For an explicit ad-hoc build, use TONK_SIGN_IDENTITY=-; Keychain prompts may recur after rebuilding." >&2
        exit 1
    fi
    identity=${development_ids[0]}
fi
# Swift Build can stamp the deployment minimum as the linked SDK version,
# which makes AppKit use legacy controls despite compiling against the new SDK.
minimum_macos=$(/usr/libexec/PlistBuddy -c 'Print :LSMinimumSystemVersion' Resources/Info.plist)
sdk_version=$(xcrun --sdk macosx --show-sdk-version)
configuration=${TONK_BUILD_CONFIGURATION:-debug}
swift build -c "$configuration" -Xlinker -platform_version -Xlinker macos \
    -Xlinker "$minimum_macos" -Xlinker "$sdk_version"
app="$PWD/.build/Tonk.app"
# Recreate the generated bundle so renamed executables cannot survive a rebuild.
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
mkdir -p "$app/Contents/Resources"
cp ".build/$configuration/Tonk" "$app/Contents/MacOS/Tonk"
cp Resources/Info.plist "$app/Contents/Info.plist"
xcrun actool "$PWD/Resources/Tonk.icon" \
    --compile "$app/Contents/Resources" \
    --platform macosx --minimum-deployment-target 15.0 \
    --app-icon Tonk \
    --output-partial-info-plist "$PWD/.build/tonk-icon-info.plist"
/usr/libexec/PlistBuddy -c "Merge '$PWD/.build/tonk-icon-info.plist'" "$app/Contents/Info.plist"
for bundle in ".build/$configuration/textual_Textual.bundle" ".build/$configuration/swiftui-math_SwiftUIMath.bundle"; do
    ditto "$bundle" "$app/Contents/Resources/$(basename "$bundle")"
done
sign_args=()
if [[ -n "${TONK_PROVISIONING_PROFILE:-}" ]]; then
    security cms -D -i "$TONK_PROVISIONING_PROFILE" > .build/tonk-passkey-profile.plist
    python3 scripts/passkey-entitlements.py .build/tonk-passkey-profile.plist .build/tonk-passkey-entitlements.plist
    cp "$TONK_PROVISIONING_PROFILE" "$app/Contents/embedded.provisionprofile"
    /usr/libexec/PlistBuddy -c 'Add :TonkDirectPasskeyEnabled bool true' "$app/Contents/Info.plist"
    sign_args=(--entitlements .build/tonk-passkey-entitlements.plist)
fi
if [[ "${TONK_RELEASE_BUILD:-0}" == 1 ]]; then
    python3 scripts/stamp-release.py "$app/Contents/Info.plist"
    sign_args+=(--options runtime --timestamp)
else
    sign_args+=(--timestamp=none)
fi
codesign --force --sign "$identity" "${sign_args[@]}" "$app"
codesign --verify --strict "$app"
if [[ -n "${TONK_PROVISIONING_PROFILE:-}" ]]; then
    signed_team=$(codesign -dv "$app" 2>&1 | sed -n 's/^TeamIdentifier=//p')
    if [[ "$signed_team" != 8WVKS2F24C ]]; then
        echo "The passkey profile requires a Tonk Labs signing identity (8WVKS2F24C)." >&2
        exit 1
    fi
fi
echo "$app"
