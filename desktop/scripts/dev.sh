#!/bin/bash
set -euo pipefail
# Use host Xcode rather than an inherited Nix SDK.
export DEVELOPER_DIR=${TONK_XCODE_DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}
unset SDKROOT
cd "$(dirname "$0")/.."

command=${1:-help}
if [[ $# -gt 0 ]]; then
    shift
fi

bold=''
accent=''
reset=''
if [[ -t 1 && ${TERM:-dumb} != dumb && ${NO_COLOR+x} != x ]]; then
    bold=$'\033[1m'
    accent=$'\033[1;36m'
    reset=$'\033[0m'
fi

status() {
    printf '\n%b%s%b\n' "$bold" "$1" "$reset"
}

command_row() {
    printf '  %b%-14s%b %s\n' "$accent" "$1" "$reset" "$2"
}

build() {
    status "Building Tonk Desktop"
    bash scripts/build-app.sh
}

open_app() {
    local app="$PWD/.build/Tonk.app"
    if [[ ! -d "$app" ]]; then
        echo "Build the app first: dev:build" >&2
        exit 1
    fi
    status "Opening Tonk Desktop"
    if [[ $# -gt 0 ]]; then
        /usr/bin/open "$app" --args "$@"
    else
        /usr/bin/open "$app"
    fi
}

case "$command" in
    dev:build|dev:install)
        if [[ $# -gt 0 ]]; then
            echo "$command does not accept arguments; configure signing with TONK_SIGN_IDENTITY and TONK_PROVISIONING_PROFILE." >&2
            exit 1
        fi
        build
        if [[ "$command" == dev:install ]]; then
            status "Installing to ~/Applications/Tonk.app"
            destination="$HOME/Applications/Tonk.app"
            mkdir -p "$HOME/Applications"
            staging=$(mktemp -d "$HOME/Applications/.tonk-install.XXXXXX")
            trap 'rm -rf "$staging"' EXIT
            /usr/bin/ditto .build/Tonk.app "$staging/Tonk.app"
            /usr/bin/codesign --verify --strict "$staging/Tonk.app"
            rm -rf "$destination"
            mv "$staging/Tonk.app" "$destination"
            echo "$destination"
        fi
        ;;
    dev:open)
        open_app "$@"
        ;;
    dev:run)
        build
        open_app "$@"
        ;;
    test:unit)
        status "Running Swift tests"
        node ../mcp/scripts/sync-desktop.mjs
        exec swift test "$@"
        ;;
    test:workflow)
        status "Running isolated local agent workflow"
        exec python3 scripts/test-agent-workflow.py "$@"
        ;;
    test:smoke)
        status "Running native smoke test"
        exec bash scripts/smoke.sh "$@"
        ;;
    dev:help|help|--help|-h)
        status "Tonk Desktop"
        printf '  Xcode Swift · Node.js · Python\n'
        status "Development"
        command_row dev:build "Build and sign the app"
        command_row dev:install "Build and install to ~/Applications"
        command_row dev:open "Open the existing build"
        command_row dev:run "Build and open the app"
        status "Testing"
        command_row test:unit "Run Swift tests (accepts --filter)"
        command_row test:smoke "Run the native smoke test"
        command_row test:workflow "Test shared MCP authoring and restart against local runtime"
        printf '\n  Run dev:help to show these commands again.\n\n'
        ;;
    *)
        echo "Unknown command: $command" >&2
        exit 1
        ;;
esac
