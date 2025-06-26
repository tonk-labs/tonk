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

# Download and install @tonk/create package
echo -e "${BLUE}Downloading and installing @tonk/create...${NC}"
npm install @tonk/create
if [ $? -ne 0 ]; then
  echo -e "${RED}❌ Failed to install @tonk/create${NC}"
  exit 1
fi

echo -e "${GREEN}✅ @tonk/create installed successfully${NC}"

# Create wrapper script
echo -e "${BLUE}Creating tonk command wrapper...${NC}"
cat >"$TONK_BIN_DIR/tonk" <<'EOF'
#!/bin/bash
# Tonk CLI wrapper script
NODE_PATH="$HOME/.tonk/packages/node_modules" exec node "$HOME/.tonk/packages/node_modules/.bin/tonk" "$@"
EOF

# Make the wrapper executable
chmod +x "$TONK_BIN_DIR/tonk"

# Create tonk-create wrapper script
echo -e "${BLUE}Creating tonk-create command wrapper...${NC}"
cat >"$TONK_BIN_DIR/tonk-create" <<'EOF'
#!/bin/bash
# Tonk Create CLI wrapper script
NODE_PATH="$HOME/.tonk/packages/node_modules" exec node "$HOME/.tonk/packages/node_modules/.bin/create" "$@"
EOF

# Make the wrapper executable
chmod +x "$TONK_BIN_DIR/tonk-create"

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

# Prompt user for workspace creation
echo -e "\n${BLUE}===============================${NC}"
echo -e "${GREEN}🎉 Tonk CLI installed successfully! 🎉${NC}"
echo -e "${BLUE}===============================${NC}"

# Check if user wants to create a workspace
echo -e "\n${YELLOW}Would you like to create a Tonk workspace in your home directory?${NC}"
echo -e "${BLUE}A workspace provides a structured environment for organizing your Tonk projects,${NC}"
echo -e "${BLUE}including apps, workers, and data processing pipelines.${NC}"
echo -e "\n${YELLOW}Create workspace now? (y/n):${NC} "
read -r CREATE_WORKSPACE

if [[ "$CREATE_WORKSPACE" =~ ^[Yy]$ ]]; then
  echo -e "\n${BLUE}Creating Tonk workspace at $HOME/tonk-workspace...${NC}"

  # Create workspace using tonk-create
  if "$TONK_BIN_DIR/tonk-create" -t workspace -n tonk-workspace -d "My Tonk workspace" --init; then
    echo -e "${GREEN}✅ Workspace created successfully at $HOME/tonk-workspace!${NC}"
    echo -e "\n${BLUE}Next steps:${NC}"
    echo -e "1. ${YELLOW}cd ~/tonk-workspace${NC} - Navigate to your workspace"
    echo -e "2. ${YELLOW}cd console && pnpm install && pnpm dev${NC} - Start the console app"
    echo -e "3. ${YELLOW}Open your favourite vibe coding editor to get started"
  else
    echo -e "${RED}❌ Failed to create workspace${NC}"
    echo -e "${YELLOW}You can create one manually later with:${NC}"
    echo -e "  ${YELLOW}cd ~ && tonk-create -t workspace -n tonk-workspace${NC}"
  fi
else
  echo -e "\n${BLUE}No workspace created.${NC}"
  echo -e "\n${YELLOW}To create a workspace later:${NC}"
  echo -e "1. Navigate to your desired directory: ${YELLOW}cd ~/desired-location${NC}"
  echo -e "2. Create workspace: ${YELLOW}tonk-create -t workspace -n my-workspace${NC}"
  echo -e "\n${BLUE}Why create a workspace?${NC}"
  echo -e "• Organize multiple Tonk projects in one place"
  echo -e "• Built-in console app for monitoring your data flows"
  echo -e "• Structured directories for apps (/views) and workers (/workers)"
  echo -e "• Agent-friendly environment with co-located instructions"
  echo -e "• Seamless data processing pipeline development"
fi

echo -e "\n${BLUE}General next steps:${NC}"
echo -e "1. Restart your terminal or run: ${YELLOW}source ~/.bashrc${NC} (or ~/.zshrc)"
echo -e "2. Run: ${YELLOW}tonk --help${NC} to see available commands"
echo -e "3. Run: ${YELLOW}tonk-create --help${NC} to see project creation options"
echo -e "4. Get started with: ${YELLOW}tonk hello${NC}"
echo -e "\n${BLUE}Installation location:${NC} $TONK_DIR"
echo -e "${BLUE}Command locations:${NC} $TONK_BIN_DIR/tonk, $TONK_BIN_DIR/tonk-create"
echo -e "\n${BLUE}To uninstall:${NC} rm -rf $TONK_DIR and remove from PATH"
echo -e "${GREEN}===============================${NC}"
