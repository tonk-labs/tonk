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
npm install @tonk/cli &>/dev/null
if [ $? -ne 0 ]; then
  echo -e "${RED}❌ Failed to install @tonk/cli${NC}"
  exit 1
fi

echo -e "${GREEN}✅ @tonk/cli installed successfully${NC}"

# Download and install @tonk/create package
echo -e "${BLUE}Downloading and installing @tonk/create...${NC}"
npm install @tonk/create &>/dev/null
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
NODE_PATH="$HOME/.tonk/packages/node_modules" exec node "$HOME/.tonk/packages/node_modules/.bin/tonk-create" "$@"
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

# ASCII Art Display
display_tonk_ascii() {
  # Static TONK ASCII art (simplified version for bash)
  echo -e "${GREEN}"
  echo "       _____   U  ___ u  _   _       _  __"
  echo "      |_ \" _|   \\/\"_ \\/ | \\ |\"|     |\"|/ /"
  echo "        | |     | | | |<|  \\| |>    | ' /"
  echo "       /| |\\.-,_| |_| |U| |\\  |u  U/| . \\\\u"
  echo "      u |_|U \\_)-\\___/  |_| \\_|     |_|\\_\\"
  echo "      _// \\\\_     \\\\    ||   \\\\,-.,-,>> \\\\,-."
  echo "     (__) (__)   (__)   (_\")  (_/  \\.)   (_/"
  echo -e "${NC}"
  echo -e "${YELLOW}・。゜☆。・゜。・。゜☆。・゜★・。゜☆。・゜。・。゜☆。・゜☆。・゜☆。・゜☆。・゜${NC}"
}

# Welcome message with workspace creation
echo -e "\n${BLUE}===============================${NC}"
echo -e "${GREEN}🎉 Tonk CLI installed successfully! 🎉${NC}"
echo -e "${BLUE}===============================${NC}"

# Display ASCII art
display_tonk_ascii

echo -e "\n${GREEN}Hello Builder!${NC}"
echo -e "\nThank you for choosing Tonk! We're thrilled to have you onboard and"
echo -e "excited to see what we can build together."

# Start Tonk daemon with PM2
echo -e "\n${BLUE}Starting Tonk daemon...${NC}"

# Check if pm2 is installed
PM2_EXISTS=false
if command -v pm2 &>/dev/null; then
  PM2_EXISTS=true
else
  echo -e "${BLUE}PM2 is required to run Tonk. Installing now...${NC}"
  if npm install -g pm2 &>/dev/null; then
    PM2_EXISTS=true
    echo -e "${GREEN}PM2 installed successfully.${NC}"
  else
    echo -e "${RED}❌ Failed to install PM2${NC}"
    echo -e "${YELLOW}You can install it manually later with: npm install -g pm2${NC}"
  fi
fi

if [ "$PM2_EXISTS" = true ]; then
  # Check if tonk process is already running in PM2
  if pm2 list 2>/dev/null | grep -q "tonkserver"; then
    echo -e "${YELLOW}Tonk daemon is already running. Restarting...${NC}"
    pm2 restart tonkserver &>/dev/null
  else
    # Start the tonk daemon with PM2
    pm2 start bash --name tonkserver -- "$TONK_BIN_DIR/tonk" -d &>/dev/null
  fi

  if pm2 list 2>/dev/null | grep -q "tonkserver"; then
    echo -e "${GREEN}✅ Tonk daemon started successfully!${NC}"
  else
    echo -e "${YELLOW}⚠️  Tonk daemon startup may have failed. You can start it manually with: tonk hello${NC}"
  fi
fi

# Check if user wants to create a workspace
echo -e "\n${YELLOW}Would you like to create a Tonk workspace in your home directory?${NC}"
echo -e "${BLUE}A workspace provides a structured environment for organizing your Tonk projects,${NC}"
echo -e "${BLUE}including apps, workers, and data processing pipelines.${NC}"
echo -e "\n${YELLOW}Create workspace now? (y/n):${NC} "
read -r CREATE_WORKSPACE

WORKSPACE_CREATED=false
WORKSPACE_PATH=""

if [[ "$CREATE_WORKSPACE" =~ ^[Yy]$ ]]; then
  echo -e "\n${BLUE}Creating Tonk workspace at $HOME/tonk-workspace...${NC}"

  # Create workspace using tonk-create
  if (cd "$HOME" && "$TONK_BIN_DIR/tonk-create" -t workspace -n tonk-workspace -d "My Tonk workspace" >/dev/null 2>&1); then
    echo -e "${GREEN}✅ Workspace created successfully at $HOME/tonk-workspace!${NC}"
    WORKSPACE_CREATED=true
    WORKSPACE_PATH="$HOME/tonk-workspace"
  else
    echo -e "${RED}❌ Failed to create workspace${NC}"
    echo -e "${YELLOW}You can create one manually later with:${NC}"
    echo -e "  ${YELLOW}cd ~ && tonk-create -t workspace -n tonk-workspace${NC}"
  fi
fi

# Final welcome message with getting started instructions
echo -e "\n${BLUE}${YELLOW}・。゜☆。・゜。・。゜☆。・゜★・。゜☆。・゜。・。゜☆。・゜☆。・゜☆。・゜☆。・゜${NC}"

echo -e "\n${BLUE}Getting Started:${NC}"

if [ "$WORKSPACE_CREATED" = true ]; then
  echo -e "• Navigate to your new workspace: ${YELLOW}cd $WORKSPACE_PATH${NC}"
  echo -e "• Start the console app: ${YELLOW}cd console && pnpm install && pnpm dev${NC}"
else
  echo -e "• Create a workspace: ${YELLOW}tonk-create -t workspace -n my-workspace${NC}"
  echo -e "• Navigate to your workspace directory"
fi

echo -e "• Open your favourite vibe coding editor and let the vibecode flow 😎"
echo -e "• Talk to your LLM to create new projects and share them out"

echo -e "\n${BLUE}Resources:${NC}"
echo -e "• Documentation: ${BLUE}https://tonk-labs.github.io/tonk/${NC}"
echo -e "• Join our Telegram: ${BLUE}https://t.me/+9W-4wDR9RcM2NWZk${NC}"

echo -e "\nOur team would love to hear from you. If you need any assistance or"
echo -e "have questions, please don't hesitate to reach out through our"
echo -e "community channels."

echo -e "\n${GREEN}Happy building with Tonk!${NC}"

echo -e "\n${BLUE}Installation details:${NC}"
echo -e "• Location: $TONK_DIR"
echo -e "• To uninstall: rm -rf $TONK_DIR and remove from PATH"
if [ "$WORKSPACE_CREATED" = true ]; then
  echo -e "• Workspace: $WORKSPACE_PATH"
fi

echo -e "\n${YELLOW}Restart your terminal to start using tonk commands!${NC}"
echo -e "${GREEN}===============================${NC}"
