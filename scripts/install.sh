#!/bin/bash

# Colors for better output
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Tonk installation directory
TONK_DIR="$HOME/.tonk"
TONK_BIN_DIR="$TONK_DIR/bin"
TONK_PACKAGES_DIR="$TONK_DIR/packages"

echo -e "${GREEN}=== Tonk CLI Installation ===${NC}"
echo -e "${BLUE}Installing Tonk CLI to $TONK_DIR${NC}"

# Check if Node.js is installed
if ! command -v node &>/dev/null; then
  echo -e "${RED}❌ Node.js is not installed.${NC}"
  echo -e "${YELLOW}Please install Node.js (version 18 or higher) first:${NC}"
  echo -e "  • Visit: https://nodejs.org/"
  echo -e "  • Or use a package manager like nvm, brew, or apt"
  exit 1
fi

# Check Node.js version
NODE_VERSION=$(node -v | sed 's/v//')
NODE_MAJOR_VERSION=$(echo $NODE_VERSION | cut -d. -f1)
if [ "$NODE_MAJOR_VERSION" -lt 18 ]; then
  echo -e "${RED}❌ Node.js version 18 or higher is required. Found: v$NODE_VERSION${NC}"
  echo -e "${YELLOW}Please update Node.js to version 18 or higher.${NC}"
  exit 1
fi

# Check if npm is available
if ! command -v npm &>/dev/null; then
  echo -e "${RED}❌ npm is not installed.${NC}"
  echo -e "${YELLOW}npm usually comes with Node.js. Please reinstall Node.js.${NC}"
  exit 1
fi

echo -e "${GREEN}✅ Node.js v$NODE_VERSION detected${NC}"

# Create installation directories
echo -e "${BLUE}Creating installation directories...${NC}"
mkdir -p "$TONK_BIN_DIR"
mkdir -p "$TONK_PACKAGES_DIR"

# Clean up any existing installation
if [ -d "$TONK_PACKAGES_DIR/cli" ]; then
  echo -e "${YELLOW}Removing existing installation...${NC}"
  rm -rf "$TONK_PACKAGES_DIR/cli"
fi

# Download and install @tonk/cli package
echo -e "${BLUE}Downloading and installing @tonk/cli...${NC}"
cd "$TONK_PACKAGES_DIR"

# Install the package locally (not globally)
npm install @tonk/cli
if [ $? -ne 0 ]; then
  echo -e "${RED}❌ Failed to install @tonk/cli${NC}"
  exit 1
fi

echo -e "${GREEN}✅ @tonk/cli installed successfully${NC}"

# Create wrapper script
echo -e "${BLUE}Creating tonk command wrapper...${NC}"
cat >"$TONK_BIN_DIR/tonk" <<'EOF'
#!/bin/bash
# Tonk CLI wrapper script
NODE_PATH="$HOME/.tonk/packages/node_modules" exec node "$HOME/.tonk/packages/node_modules/.bin/tonk" "$@"
EOF

# Make the wrapper executable
chmod +x "$TONK_BIN_DIR/tonk"

# Check if tonk bin directory is in PATH
echo -e "${BLUE}Checking PATH configuration...${NC}"
if [[ ":$PATH:" != *":$TONK_BIN_DIR:"* ]]; then
  echo -e "${YELLOW}Adding $TONK_BIN_DIR to PATH...${NC}"

  # Detect shell and add to appropriate config file
  SHELL_NAME=$(basename "$SHELL")
  case "$SHELL_NAME" in
  bash)
    SHELL_CONFIG="$HOME/.bashrc"
    [ -f "$HOME/.bash_profile" ] && SHELL_CONFIG="$HOME/.bash_profile"
    ;;
  zsh)
    SHELL_CONFIG="$HOME/.zshrc"
    ;;
  fish)
    echo -e "${YELLOW}Fish shell detected. Please manually add to your config:${NC}"
    echo -e "  set -gx PATH $TONK_BIN_DIR \$PATH"
    SHELL_CONFIG=""
    ;;
  *)
    echo -e "${YELLOW}Unknown shell: $SHELL_NAME${NC}"
    SHELL_CONFIG="$HOME/.profile"
    ;;
  esac

  if [ -n "$SHELL_CONFIG" ]; then
    echo '' >>"$SHELL_CONFIG"
    echo '# Added by Tonk installer' >>"$SHELL_CONFIG"
    echo "export PATH=\"$TONK_BIN_DIR:\$PATH\"" >>"$SHELL_CONFIG"
    echo -e "${GREEN}✅ Added to $SHELL_CONFIG${NC}"
    echo -e "${YELLOW}Please run: source $SHELL_CONFIG${NC}"
    echo -e "${YELLOW}Or restart your terminal to use the 'tonk' command${NC}"
  fi
else
  echo -e "${GREEN}✅ $TONK_BIN_DIR already in PATH${NC}"
fi

# Test installation
echo -e "${BLUE}Testing installation...${NC}"
if "$TONK_BIN_DIR/tonk" --version &>/dev/null; then
  echo -e "${GREEN}✅ Installation successful!${NC}"
else
  echo -e "${YELLOW}⚠️  Installation completed but tonk command test failed${NC}"
  echo -e "${YELLOW}You may need to restart your terminal or run: source ~/.bashrc${NC}"
fi

echo -e "\n${GREEN}===============================${NC}"
echo -e "${GREEN}🎉 Tonk CLI installed successfully! 🎉${NC}"
echo -e "\n${BLUE}Next steps:${NC}"
echo -e "1. Restart your terminal or run: ${YELLOW}source ~/.bashrc${NC} (or ~/.zshrc)"
echo -e "2. Run: ${YELLOW}tonk --help${NC} to see available commands"
echo -e "3. Get started with: ${YELLOW}tonk hello${NC}"
echo -e "\n${BLUE}Installation location:${NC} $TONK_DIR"
echo -e "${BLUE}Command location:${NC} $TONK_BIN_DIR/tonk"
echo -e "\n${BLUE}To uninstall:${NC} rm -rf $TONK_DIR and remove from PATH"
echo -e "${GREEN}===============================${NC}"

