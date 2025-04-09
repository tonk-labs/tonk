#!/bin/bash

# Colors for better output
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}=== Tonk Installation Script ===${NC}"

# Check if git is installed
if ! command -v git &> /dev/null; then
    echo -e "${YELLOW}Git is not installed. Please install Git first.${NC}"
    exit 1
fi

# Check if Node.js is installed
if ! command -v node &> /dev/null; then
    echo -e "${YELLOW}Node.js is not installed. Please install Node.js first.${NC}"
    exit 1
fi

# Check if pnpm is installed
if ! command -v pnpm &> /dev/null; then
    echo -e "${YELLOW}pnpm is not installed. Installing pnpm...${NC}"
    npm install -g pnpm
    
    # Verify installation
    if ! command -v pnpm &> /dev/null; then
        echo -e "${YELLOW}Failed to install pnpm. Please install it manually.${NC}"
        exit 1
    fi
    echo -e "${GREEN}pnpm installed successfully!${NC}"
fi

# Clone the repository if not already in it
REPO_DIR="tonk"
if [ ! -d ".git" ]; then
    echo -e "\n${GREEN}Cloning Tonk repository...${NC}"
    git clone https://github.com/tonk-labs/tonk.git $REPO_DIR
    cd $REPO_DIR
    echo -e "${GREEN}Repository cloned successfully!${NC}"
else
    echo -e "\n${GREEN}Already in a git repository. Proceeding with installation...${NC}"
fi

# Install dependencies for each package in order
PACKAGES=("server" "keepsync" "create" "hub" "hub-ui" "cli")

echo -e "\n${GREEN}Installing dependencies for all packages...${NC}"

# First install root dependencies if package.json exists in root
if [ -f "package.json" ]; then
    echo -e "\n${GREEN}Installing root dependencies...${NC}"
    pnpm install
fi

# Install dependencies for each package
for package in "${PACKAGES[@]}"; do
    if [ -d "packages/$package" ]; then
        echo -e "\n${GREEN}Building package: $package${NC}"
        cd "packages/$package"
        
        # Install dependencies and build using npm for hub package, pnpm for others
        if [ "$package" = "hub" ]; then
            echo -e "${GREEN}Installing and building hub package with npm...${NC}"
            npm install
            
            if [ -f "package.json" ]; then
                if grep -q "\"build\":" "package.json"; then
                    echo "Building $package with npm..."
                    npm run build
                fi
            fi
        else
            # Install dependencies with pnpm for other packages
            pnpm install
            
            # Build the package
            if [ -f "package.json" ]; then
                if grep -q "\"build\":" "package.json"; then
                    echo "Building $package..."
                    pnpm build
                fi
            fi
        fi
        
        cd - > /dev/null
    else
        echo -e "${YELLOW}Warning: Package directory 'packages/$package' not found.${NC}"
    fi
done

# Link CLI and Create packages globally
echo -e "\n${GREEN}Linking CLI and Create packages globally...${NC}"

if [ -d "packages/cli" ]; then
    cd "packages/cli"
    npm link
    cd - > /dev/null
    echo -e "${GREEN}CLI package linked globally.${NC}"
else
    echo -e "${YELLOW}Warning: CLI package directory not found.${NC}"
fi

if [ -d "packages/create" ]; then
    cd "packages/create"
    npm link
    cd - > /dev/null
    echo -e "${GREEN}Create package linked globally.${NC}"
else
    echo -e "${YELLOW}Warning: Create package directory not found.${NC}"
fi

echo -e "\n${GREEN}Installation completed successfully!${NC}"
echo -e "${GREEN}You can now run: ${YELLOW}tonk hello${NC}"
echo -e "${GREEN}To get started with Tonk.${NC}" 