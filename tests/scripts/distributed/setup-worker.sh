#!/bin/bash
set -e

echo "🚀 Setting up Tonk Load Generator Worker..."

cd /home/ec2-user

echo "📦 Updating system packages..."
sudo yum update -y

echo "📦 Installing system dependencies for Playwright..."
sudo yum install -y \
  atk cups-libs gtk3 libXcomposite alsa-lib \
  libXcursor libXdamage libXext libXi libXrandr libXScrnSaver \
  libXtst pango at-spi2-atk libXt xorg-x11-server-Xvfb \
  xorg-x11-xauth dbus-glib dbus-glib-devel nss mesa-libgbm

echo "📦 Creating working directory..."
mkdir -p /home/ec2-user/worker

echo "📦 Installing nvm..."
if [ ! -d "$HOME/.nvm" ]; then
  curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.3/install.sh | bash
  echo "✓ nvm installed"
else
  echo "✓ nvm already installed"
fi

export NVM_DIR="$HOME/.nvm"
[ -s "$NVM_DIR/nvm.sh" ] && \. "$NVM_DIR/nvm.sh"

echo "📦 Installing Node.js 22..."
if ! command -v node &>/dev/null; then
  nvm install 22
  nvm use 22
  nvm alias default 22
  echo "✓ Node.js 22 installed"
else
  echo "✓ Node.js already installed ($(node --version))"
fi

echo "📦 Installing pnpm..."
if ! command -v pnpm &>/dev/null; then
  npm install -g pnpm
  echo "✓ pnpm installed"
else
  echo "✓ pnpm already installed"
fi

echo "✅ Worker setup complete!"
echo ""
echo "Worker is ready to receive load generator script and dependencies"
