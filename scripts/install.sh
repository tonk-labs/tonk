#!/bin/bash

# Colors for better output
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m' # No Color

# Initialize error tracking
ERRORS=()

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
    if [ $? -ne 0 ]; then
        ERRORS+=("Failed to install pnpm")
    fi
    
    # Verify installation
    if ! command -v pnpm &> /dev/null; then
        echo -e "${YELLOW}Failed to install pnpm. Please install it manually.${NC}"
        exit 1
    fi
    echo -e "${GREEN}pnpm installed successfully!${NC}"
fi

# Check if packages are already installed globally
echo -e "\n${GREEN}Checking for existing global installations...${NC}"

CLI_INSTALLED=$(npm list -g @tonk/cli 2>/dev/null | grep -c "@tonk/cli")
CREATE_INSTALLED=$(npm list -g @tonk/create 2>/dev/null | grep -c "@tonk/create")

if [ $CLI_INSTALLED -gt 0 ] || [ $CREATE_INSTALLED -gt 0 ]; then
    echo -e "${YELLOW}Warning: Found existing global installation of Tonk packages:${NC}"
    
    if [ $CLI_INSTALLED -gt 0 ]; then
        echo -e "- @tonk/cli"
    fi
    
    if [ $CREATE_INSTALLED -gt 0 ]; then
        echo -e "- @tonk/create"
    fi
    
    echo -e "\n${YELLOW}Do you want to remove these packages and continue with installation? (y/n)${NC}"
    read -n 1 -r
    echo
    
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        echo -e "${GREEN}Removing existing packages...${NC}"
        
        if [ $CLI_INSTALLED -gt 0 ]; then
            npm rm -g @tonk/cli
            if [ $? -ne 0 ]; then
                ERRORS+=("Failed to remove @tonk/cli")
            fi
        fi
        
        if [ $CREATE_INSTALLED -gt 0 ]; then
            npm rm -g @tonk/create
            if [ $? -ne 0 ]; then
                ERRORS+=("Failed to remove @tonk/create")
            fi
        fi
        
        echo -e "${GREEN}Existing packages removed. Continuing with installation...${NC}"
    else
        echo -e "${YELLOW}Installation aborted.${NC}"
        exit 0
    fi
fi

# Clone the repository if not already in it
REPO_DIR="tonk"
if [ ! -d ".git" ]; then
    echo -e "\n${GREEN}Cloning Tonk repository...${NC}"
    git clone https://github.com/tonk-labs/tonk.git $REPO_DIR
    if [ $? -ne 0 ]; then
        ERRORS+=("Failed to clone repository")
    fi
    cd $REPO_DIR
    echo -e "${GREEN}Repository cloned successfully!${NC}"
else
    echo -e "\n${GREEN}Already in a git repository. Proceeding with installation...${NC}"
fi

# Install dependencies for each package in order
PACKAGES=("server" "keepsync" "create" "hub" "cli")

echo -e "\n${GREEN}Installing dependencies for all packages...${NC}"


# Install dependencies for each package
for package in "${PACKAGES[@]}"; do
    if [ -d "packages/$package" ]; then
        echo -e "\n${GREEN}Building package: $package${NC}"
        cd "packages/$package"
        
        # Install dependencies and build using npm for hub package, pnpm for others
        if [ "$package" = "hub" ]; then
            echo -e "${GREEN}Installing and building hub package with npm...${NC}"
            npm install
            if [ $? -ne 0 ]; then
                ERRORS+=("Failed to install dependencies for $package")
            fi
            
        else
            # Install dependencies with pnpm for other packages
            pnpm install
            if [ $? -ne 0 ]; then
                ERRORS+=("Failed to install dependencies for $package")
            fi
            
            # Build the package
            if [ -f "package.json" ]; then
                if grep -q "\"build\":" "package.json"; then
                    echo "Building $package..."
                    pnpm build
                    if [ $? -ne 0 ]; then
                        ERRORS+=("Failed to build $package")
                    fi
                fi
            fi
        fi
        
        cd - > /dev/null
    else
        echo -e "${YELLOW}Warning: Package directory 'packages/$package' not found.${NC}"
    fi
done

# Install and build hub-ui
if [ -d "packages/hub/hub-ui" ]; then
    echo -e "${GREEN}Installing and building hub-ui...${NC}"
    cd "packages/hub/hub-ui"
    
    # Use the same pattern as the rest of the script - check OS and use appropriate commands
    if [ "$OSTYPE" == "msys" ] || [ "$OSTYPE" == "win32" ] || [ "$OSTYPE" == "cygwin" ]; then
        # Windows path handling
        echo -e "${GREEN}Installing and building hub-ui on Windows...${NC}"
        pnpm install
        if [ $? -ne 0 ]; then
            ERRORS+=("Failed to install dependencies for hub-ui")
        fi
        
        # Build the package if build script exists
        if [ -f "package.json" ]; then
            if grep -q "\"build\":" "package.json"; then
                echo "Building hub-ui..."
                pnpm build
                if [ $? -ne 0 ]; then
                    ERRORS+=("Failed to build hub-ui")
                fi
            fi
        fi
    else
        # Unix path handling
        pnpm install
        if [ $? -ne 0 ]; then
            ERRORS+=("Failed to install dependencies for hub-ui")
        fi
        
        # Build the package if build script exists
        if [ -f "package.json" ]; then
            if grep -q "\"build\":" "package.json"; then
                echo "Building hub-ui..."
                pnpm build
                if [ $? -ne 0 ]; then
                    ERRORS+=("Failed to build hub-ui")
                fi
            fi
        fi
    fi
    
    cd - > /dev/null
    echo -e "${GREEN}hub-ui installed and built successfully.${NC}"
else
    echo -e "${YELLOW}Warning: hub-ui directory not found at packages/hub/hub-ui.${NC}"
fi


# Pack and install CLI and Create packages globally
echo -e "\n${GREEN}Installing CLI and Create packages globally...${NC}"


if [ -d "packages/cli" ]; then
    echo -e "${GREEN}Installing CLI package globally...${NC}"
    npm install -g "./packages/cli"
    if [ $? -ne 0 ]; then
        ERRORS+=("Failed to install CLI package globally")
    fi
    echo -e "${GREEN}CLI package installed globally.${NC}"
else
    echo -e "${YELLOW}Warning: CLI package directory not found.${NC}"
fi

if [ -d "packages/create" ]; then
    echo -e "${GREEN}Installing Create package globally...${NC}"
    npm install -g "./packages/create"
    if [ $? -ne 0 ]; then
        ERRORS+=("Failed to install Create package globally")
    fi
    echo -e "${GREEN}Create package installed globally.${NC}"
else
    echo -e "${YELLOW}Warning: Create package directory not found.${NC}"
fi

# Print final status message
echo -e "\n${GREEN}===============================${NC}"
if [ ${#ERRORS[@]} -eq 0 ]; then
    echo -e "${GREEN}🎉 Congratulations! Tonk has been successfully installed! 🎉${NC}"
    echo -e "${GREEN}You can get started by running: ${YELLOW}tonk hello${NC}"
else
    echo -e "${RED}⚠️ Installation ran with errors: ⚠️${NC}"
    for error in "${ERRORS[@]}"; do
        echo -e "${RED}- $error${NC}"
    done
    echo -e "\n${YELLOW}Need help? Join our Telegram support group: ${NC}https://t.me/+9W-4wDR9RcM2NWZk"
fi
echo -e "${GREEN}===============================${NC}"

