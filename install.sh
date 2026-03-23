#!/bin/sh
set -e

# Carry CLI installer
# Usage:
#   Install: curl -fsSL https://raw.githubusercontent.com/tonk-labs/tonk/feat/carry/install.sh | sh
#   Uninstall: curl -fsSL https://raw.githubusercontent.com/tonk-labs/tonk/feat/carry/install.sh | sh -s -- uninstall

REPO="tonk-labs/tonk"
TAG="latest"
INSTALL_DIR="/usr/local/bin"
BINARY_NAME="carry"

main() {
    if [ "${1:-}" = "uninstall" ]; then
        uninstall
        return 0
    fi
    # Detect platform
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "$OS" in
        Darwin) os="macos" ;;
        Linux)  os="linux" ;;
        *)
            echo "Error: Unsupported operating system: $OS"
            exit 1
            ;;
    esac

    case "$ARCH" in
        arm64|aarch64) arch="arm64" ;;
        x86_64)        arch="x86_64" ;;
        *)
            echo "Error: Unsupported architecture: $ARCH"
            exit 1
            ;;
    esac

    ARTIFACT="${BINARY_NAME}-${os}-${arch}"
    URL="https://github.com/${REPO}/releases/download/${TAG}/${ARTIFACT}"

    echo "Installing ${BINARY_NAME} (${os}/${arch})..."

    # Download to a temp file
    TMPFILE="$(mktemp)"
    trap 'rm -f "$TMPFILE"' EXIT

    if ! curl -fsSL -o "$TMPFILE" "$URL"; then
        echo "Error: Download failed."
        echo "  URL: $URL"
        echo ""
        echo "  This may mean there is no build for your platform (${os}/${arch})."
        exit 1
    fi

    chmod +x "$TMPFILE"

    # Install to INSTALL_DIR, using sudo if needed, falling back to ~/.local/bin
    if [ -w "$INSTALL_DIR" ]; then
        mv "$TMPFILE" "${INSTALL_DIR}/${BINARY_NAME}"
    elif command -v sudo >/dev/null 2>&1 && sudo -n true 2>/dev/null || [ -t 0 ]; then
        echo "Installing to ${INSTALL_DIR} (you may be prompted for your password)..."
        sudo mv "$TMPFILE" "${INSTALL_DIR}/${BINARY_NAME}"
        sudo chmod +x "${INSTALL_DIR}/${BINARY_NAME}"
    else
        INSTALL_DIR="${HOME}/.local/bin"
        echo "No admin privileges detected. Installing to ${INSTALL_DIR} instead..."
        mkdir -p "$INSTALL_DIR"
        mv "$TMPFILE" "${INSTALL_DIR}/${BINARY_NAME}"
        chmod +x "${INSTALL_DIR}/${BINARY_NAME}"
    fi

    # Verify
    if ! command -v "$BINARY_NAME" >/dev/null 2>&1; then
        echo ""
        echo "Done! carry was installed to ${INSTALL_DIR}/${BINARY_NAME}"
        echo ""
        echo "If '${INSTALL_DIR}' is not on your PATH, add it:"
        echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
        return 0
    fi

    echo ""
    echo "Done! carry is installed at ${INSTALL_DIR}/${BINARY_NAME}"

    # Install shell completions
    install_completions

    echo ""
    echo "Get started with:"
    echo "  carry --help"
}

install_completions() {
    SHELL_NAME="$(basename "$SHELL")"
    COMP_DIR=""
    COMP_FILE=""
    NEEDS_SUDO=false

    # Check if we can use sudo
    CAN_SUDO=false
    if command -v sudo >/dev/null 2>&1 && sudo -n true 2>/dev/null || [ -t 0 ]; then
        CAN_SUDO=true
    fi

    case "$SHELL_NAME" in
        zsh)
            if [ "$CAN_SUDO" = true ]; then
                COMP_DIR="/usr/local/share/zsh/site-functions"
                NEEDS_SUDO=true
            else
                COMP_DIR="${HOME}/.zfunc"
            fi
            COMP_FILE="${COMP_DIR}/_carry"
            ;;
        bash)
            if [ "$CAN_SUDO" = true ]; then
                COMP_DIR="/usr/local/share/bash-completion/completions"
                NEEDS_SUDO=true
            else
                COMP_DIR="${HOME}/.local/share/bash-completion/completions"
            fi
            COMP_FILE="${COMP_DIR}/carry"
            ;;
        fish)
            COMP_DIR="${HOME}/.config/fish/completions"
            COMP_FILE="${COMP_DIR}/carry.fish"
            ;;
        *)
            return 0
            ;;
    esac

    # Generate completions from the installed binary
    COMPLETIONS="$(COMPLETE="$SHELL_NAME" "${INSTALL_DIR}/${BINARY_NAME}" 2>/dev/null)" || return 0

    if [ -z "$COMPLETIONS" ]; then
        return 0
    fi

    # Write completions to the appropriate directory
    if [ "$NEEDS_SUDO" = true ]; then
        if [ ! -d "$COMP_DIR" ]; then
            sudo mkdir -p "$COMP_DIR"
        fi
        echo "$COMPLETIONS" | sudo tee "$COMP_FILE" >/dev/null
    else
        mkdir -p "$COMP_DIR"
        echo "$COMPLETIONS" > "$COMP_FILE"
    fi

    echo "Shell completions installed for ${SHELL_NAME}."
    if [ "$SHELL_NAME" = "zsh" ] && [ "$NEEDS_SUDO" = false ]; then
        echo "Add this to your .zshrc if not already present:"
        echo "  fpath=(~/.zfunc \$fpath)"
        echo "  autoload -Uz compinit && compinit"
    fi
    echo "Restart your shell or run 'exec \$SHELL' to enable them."
}

uninstall() {
    echo "Uninstalling ${BINARY_NAME}..."

    REMOVED=""

    # Remove binary from both possible locations
    for BIN_DIR in "$INSTALL_DIR" "${HOME}/.local/bin"; do
        BIN_PATH="${BIN_DIR}/${BINARY_NAME}"
        if [ -f "$BIN_PATH" ]; then
            if [ -w "$BIN_DIR" ]; then
                rm "$BIN_PATH"
            else
                echo "Removing ${BIN_PATH} (you may be prompted for your password)..."
                sudo rm "$BIN_PATH"
            fi
            REMOVED="${REMOVED} ${BIN_PATH}"
        fi
    done

    # Remove completion files (both system and user-local paths)
    for COMP_FILE in \
        "/usr/local/share/zsh/site-functions/_carry" \
        "/usr/local/share/bash-completion/completions/carry" \
        "${HOME}/.zfunc/_carry" \
        "${HOME}/.local/share/bash-completion/completions/carry" \
        "${HOME}/.config/fish/completions/carry.fish"; do
        if [ -f "$COMP_FILE" ]; then
            case "$COMP_FILE" in
                "${HOME}"*)
                    rm "$COMP_FILE"
                    ;;
                *)
                    sudo rm "$COMP_FILE"
                    ;;
            esac
            REMOVED="${REMOVED} ${COMP_FILE}"
        fi
    done

    if [ -z "$REMOVED" ]; then
        echo "Nothing to remove — carry does not appear to be installed."
    else
        echo "Removed:${REMOVED}"
        echo ""
        echo "carry has been uninstalled."
    fi
}

main "$@"
